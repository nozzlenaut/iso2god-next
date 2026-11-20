//! Compare the upstream part writer with the optimized writer on the same input.
//! Run: cargo run --release --example bench_writer -- <input-file> <empty-output-dir>
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};
use iso2god_next::god::{self, HashList, BLOCK_SIZE, SUBPART_SIZE, SUBPARTS_PER_PART};
use sha1::{Digest, Sha1};

fn original(mut input: File, mut output: File) -> anyhow::Result<()> {
    let mut master = HashList::new();
    master.write(&mut output)?;
    let mut batch = Vec::with_capacity(SUBPART_SIZE as usize);
    for _ in 0..SUBPARTS_PER_PART {
        batch.clear();
        input.by_ref().take(SUBPART_SIZE).read_to_end(&mut batch)?;
        if batch.is_empty() { break; }
        let mut table = HashList::new();
        for block in batch.chunks(BLOCK_SIZE as usize) { table.add_block_hash(block); }
        table.write(&mut output)?;
        master.add_block_hash(table.bytes());
        input.seek_relative(-(batch.len() as i64))?;
        io::copy(&mut input.by_ref().take(SUBPART_SIZE), &mut output)?;
        if batch.len() < SUBPART_SIZE as usize { break; }
    }
    output.seek(SeekFrom::Start(0))?;
    master.write(&mut output)?;
    output.sync_all()?;
    Ok(())
}

fn check(path: &Path) -> anyhow::Result<[u8; 20]> {
    let mut f = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 1024*1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

fn trial(input: &Path, output: &Path, fast: bool) -> anyhow::Result<(Duration, [u8;20])> {
    let start = Instant::now();
    let infile = File::open(input)?;
    let mut outfile = File::options().write(true).create_new(true).open(output)?;
    if fast {
        god::write_part(infile, 0, &mut outfile)?;
        outfile.sync_all()?;
    } else {
        original(infile, outfile)?;
    }
    let elapsed = start.elapsed();
    let hash = check(output)?;
    fs::remove_file(output)?;
    Ok((elapsed, hash))
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().map(std::path::PathBuf::from).expect("input file");
    let out = args.next().map(std::path::PathBuf::from).expect("output directory");
    fs::create_dir_all(&out)?;
    let bytes = fs::metadata(&input)?.len().min(god::BLOCKS_PER_PART * BLOCK_SIZE);
    let mut old = Vec::new();
    let mut new = Vec::new();
    for round in 0..3 {
        let order = if round % 2 == 0 { [false, true] } else { [true, false] };
        let mut hashes = Vec::new();
        for fast in order {
            let path = out.join(format!("bench-{}-{}-{}.part", std::process::id(), round, fast));
            let (elapsed, hash) = trial(&input, &path, fast)?;
            println!("{}: {:.3}s ({:.1} MiB/s)", if fast {"optimized"} else {"upstream"}, elapsed.as_secs_f64(), bytes as f64 / 1048576.0 / elapsed.as_secs_f64());
            if fast { new.push(elapsed); } else { old.push(elapsed); }
            hashes.push(hash);
        }
        anyhow::ensure!(hashes[0] == hashes[1], "different output bytes");
    }
    old.sort(); new.sort();
    println!("Median: upstream {:.3}s, optimized {:.3}s, speedup {:.2}x; output SHA-1 matches each round", old[1].as_secs_f64(), new[1].as_secs_f64(), old[1].as_secs_f64()/new[1].as_secs_f64());
    Ok(())
}
