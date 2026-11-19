use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::Error;

use crate::convert::Cancelled;

mod con_header;
pub use con_header::*;

mod file_layout;
pub use file_layout::*;

mod gdf_sector;
pub use gdf_sector::*;

mod block_hash;
mod hash_list;
pub use hash_list::*;

pub const BLOCKS_PER_PART: u64 = 0xa1c4;
pub const BLOCKS_PER_SUBPART: u64 = 0xcc;
pub const BLOCK_SIZE: u64 = 0x1000;
pub const SUBPARTS_PER_PART: u32 = 0xcb;
pub const SUBPART_SIZE: u64 = BLOCK_SIZE * BLOCKS_PER_SUBPART;

pub fn write_part<R: Read + Seek, W: Write + Seek>(
    data_volume: R,
    part_index: u64,
    part_file: W,
) -> Result<(), Error> {
    write_part_observed(data_volume, part_index, part_file, |_| {}, || false)
}

/// Writes one part file, reporting copied bytes through `on_bytes` and checking
/// `cancelled` between subparts so long conversions can be stopped promptly.
pub fn write_part_observed<R, W, P, C>(
    mut data_volume: R,
    part_index: u64,
    mut part_file: W,
    mut on_bytes: P,
    mut cancelled: C,
) -> Result<(), Error>
where
    R: Read + Seek,
    W: Write + Seek,
    P: FnMut(u64),
    C: FnMut() -> bool,
{
    data_volume.seek_relative((part_index * BLOCKS_PER_PART * BLOCK_SIZE) as i64)?;

    let mut master_hash_list = HashList::new();

    let master_hash_list_position = part_file.stream_position()?;
    master_hash_list.write(&mut part_file)?;

    // Keep the bytes we just hashed instead of seeking back and reading them
    // a second time. Batch multiple subparts for more GPU work per dispatch.
    let mut batch = Vec::with_capacity(SUBPART_SIZE as usize * block_hash::BATCH_SUBPARTS);
    let mut remaining = SUBPARTS_PER_PART as usize;
    while remaining > 0 {
        if cancelled() {
            return Err(Cancelled.into());
        }
        let n = remaining.min(block_hash::BATCH_SUBPARTS);
        let limit = SUBPART_SIZE * n as u64;
        batch.clear();
        data_volume.by_ref().take(limit).read_to_end(&mut batch)?;
        if batch.is_empty() {
            break;
        }
        let hashes = block_hash::hashes(&batch);
        for (subpart, hash_group) in batch.chunks(SUBPART_SIZE as usize)
            .zip(hashes.chunks(BLOCKS_PER_SUBPART as usize))
        {
            if cancelled() {
                return Err(Cancelled.into());
            }
            let mut sub_hash_list = HashList::new();
            for hash in hash_group {
                sub_hash_list.add_hash(hash);
            }
            sub_hash_list.write(&mut part_file)?;
            master_hash_list.add_block_hash(sub_hash_list.bytes());
            part_file.write_all(subpart)?;
            on_bytes(subpart.len() as u64);
        }
        if batch.len() < limit as usize {
            break;
        }
        remaining -= n;
    }

    part_file.seek(SeekFrom::Start(master_hash_list_position))?;
    master_hash_list.write(&mut part_file)?;

    Ok(())
}

// Independent reference implementation: reads each original subpart twice,
// exactly as the upstream writer did. Compare entire part files, not just hashes.
#[cfg(test)]
mod performance_tests {
    use super::*;
    use std::io::Cursor;

    fn reference(data: &[u8], part_index: u64) -> Vec<u8> {
        let offset = (part_index * BLOCKS_PER_PART * BLOCK_SIZE) as usize;
        let mut result = vec![0; BLOCK_SIZE as usize];
        let mut master = HashList::new();
        let data = data.get(offset..).unwrap_or(&[]);
        for subpart in data.chunks(SUBPART_SIZE as usize).take(SUBPARTS_PER_PART as usize) {
            let mut hashes = HashList::new();
            for block in subpart.chunks(BLOCK_SIZE as usize) {
                hashes.add_block_hash(block);
            }
            master.add_block_hash(hashes.bytes());
            result.extend_from_slice(hashes.bytes());
            result.extend_from_slice(subpart);
        }
        result[..BLOCK_SIZE as usize].copy_from_slice(master.bytes());
        result
    }

    #[test]
    fn fast_writer_preserves_upstream_part_bytes() {
        for size in [0, 1, 4095, 4096, 4097, SUBPART_SIZE as usize - 1,
            SUBPART_SIZE as usize, SUBPART_SIZE as usize + 1,
            SUBPART_SIZE as usize * 8 - 1, SUBPART_SIZE as usize * 8,
            SUBPART_SIZE as usize * 9 + 123] {
            let data: Vec<u8> = (0..size).map(|n| ((n*17+n/139)%253) as u8).collect();
            let mut actual = Cursor::new(Vec::new());
            write_part(Cursor::new(&data), 0, &mut actual).unwrap();
            assert_eq!(actual.into_inner(), reference(&data, 0), "size {size}");
        }
    }

    #[test]
    fn fast_writer_handles_second_part() {
        // Seek past a sparse first part rather than allocating 160 MiB of RAM.
        let path = std::env::temp_dir().join(format!("iso2god-fast-part-{}", std::process::id()));
        let mut input = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();
        input.seek(SeekFrom::Start(BLOCKS_PER_PART * BLOCK_SIZE)).unwrap();
        let tail = vec![0x5a; 8193];
        input.write_all(&tail).unwrap();
        input.seek(SeekFrom::Start(0)).unwrap();
        let mut actual = Cursor::new(Vec::new());
        write_part(&mut input, 1, &mut actual).unwrap();
        drop(input);
        std::fs::remove_file(path).unwrap();
        assert_eq!(actual.into_inner(), reference(&tail, 0));
    }
}
