//! Bounded discovery of Xbox disc images, without opening or modifying them.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const ISO_SCAN_DEPTH: usize = 4;

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<PathBuf>,
    pub unreadable: usize,
}

/// Depth zero means files directly inside a selected folder. Four means
/// files in its first four levels of subfolders as well.
pub fn discover_iso_files(roots: &[PathBuf], max_depth: usize) -> ScanResult {
    let mut files = BTreeSet::new();
    let mut unreadable = 0;
    for root in roots {
        match fs::canonicalize(root) {
            Ok(root) => visit(&root, 0, max_depth, &mut files, &mut unreadable),
            Err(_) => unreadable += 1,
        }
    }
    ScanResult { files: files.into_iter().collect(), unreadable }
}

fn visit(dir: &Path, depth: usize, limit: usize, files: &mut BTreeSet<PathBuf>, unreadable: &mut usize) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => { *unreadable += 1; return; }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => { *unreadable += 1; continue; }
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => { *unreadable += 1; continue; }
        };
        // Avoid symlink loops, Windows junctions and accidentally scanning
        // directories outside the selected tree.
        if metadata.file_type().is_symlink() { continue; }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 { continue; }
        }
        if metadata.is_dir() {
            if depth < limit { visit(&path, depth + 1, limit, files, unreadable); }
        } else if metadata.is_file() && path.extension().and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("iso") || s.eq_ignore_ascii_case("xiso"))
        {
            match fs::canonicalize(&path) {
                Ok(path) => { files.insert(path); }
                Err(_) => *unreadable += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("iso-scan-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn file(&self, relative: &str) {
            let p = self.0.join(relative);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"test").unwrap();
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn four_levels_and_case_insensitive_extensions() {
        let t = Temp::new();
        for path in ["a.ISO", "b/a.xiso", "b/c/a.Iso", "b/c/d/a.XISO",
            "b/c/d/e/a.iso", "b/c/d/e/f/too-deep.iso", "b/not-iso.txt"] { t.file(path); }
        fs::create_dir(t.0.join("folder.iso")).unwrap();
        let r = discover_iso_files(&[t.0.clone()], ISO_SCAN_DEPTH);
        assert_eq!(r.files.len(), 5);
        assert_eq!(r.unreadable, 0);
        assert!(r.files.iter().all(|p| !p.ends_with("too-deep.iso")));
        assert_eq!(discover_iso_files(&[t.0.clone()], 0).files.len(), 1);
    }

    #[test]
    fn overlapping_roots_and_missing_folder() {
        let t = Temp::new();
        t.file("A/one.iso");
        t.file("B/one.iso");
        let r = discover_iso_files(&[t.0.clone(), t.0.join("A"), t.0.join("missing")], 4);
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.unreadable, 1);
        assert!(r.files.windows(2).all(|p| p[0] < p[1]));
    }

    #[cfg(unix)]
    #[test]
    fn skips_link_loops_and_external_files() {
        let t = Temp::new();
        let other = Temp::new();
        other.file("outside.iso");
        t.file("inside.iso");
        std::os::unix::fs::symlink(&t.0, t.0.join("loop")).unwrap();
        std::os::unix::fs::symlink(&other.0, t.0.join("outside")).unwrap();
        std::os::unix::fs::symlink(other.0.join("outside.iso"), t.0.join("alias.iso")).unwrap();
        let r = discover_iso_files(&[t.0.clone()], 4);
        assert_eq!(r.files.len(), 1);
        assert!(r.files[0].ends_with("inside.iso"));
    }
}
