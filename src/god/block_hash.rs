//! Experimental OpenCL acceleration. All output remains standard SHA-1.
use sha1::{Digest, Sha1};

const CPU_BATCH_SUBPARTS: usize = 8;
#[cfg(feature = "gpu")]
const GPU_BATCH_SUBPARTS: usize = 32;
pub(super) fn batch_subparts() -> usize {
    #[cfg(feature = "gpu")]
    if matches!(std::env::var("ISO2GOD_GPU").as_deref(), Ok("1" | "hybrid")) {
        return GPU_BATCH_SUBPARTS;
    }
    CPU_BATCH_SUBPARTS
}
const BLOCK_BYTES: usize = super::BLOCK_SIZE as usize;

fn cpu_hashes(data: &[u8]) -> Vec<[u8; 20]> {
    data.chunks(BLOCK_BYTES).map(|b| Sha1::digest(b).into()).collect()
}

pub(super) fn hashes(data: &[u8]) -> Vec<[u8; 20]> {
    #[cfg(feature = "gpu")]
    if let Some(hashes) = opencl::try_hashes(data) {
        return hashes;
    }
    cpu_hashes(data)
}

#[cfg(feature = "gpu")]
mod opencl {
    use super::*;
    use anyhow::{Context as _, Result, anyhow, ensure};
    use opencl3::{
        command_queue::CommandQueue, context::Context,
        device::{CL_DEVICE_TYPE_GPU, Device, get_all_devices},
        kernel::{ExecuteKernel, Kernel},
        memory::{Buffer, CL_MEM_READ_ONLY, CL_MEM_WRITE_ONLY},
        program::Program, types::CL_BLOCKING,
    };
    use std::{ptr, sync::{Mutex, OnceLock}};

    const CAPACITY: usize = super::super::SUBPART_SIZE as usize * GPU_BATCH_SUBPARTS;
    static GPU: OnceLock<Mutex<Option<GpuHasher>>> = OnceLock::new();

    fn report(message: &str) {
        eprintln!("{message}");
        if let Some(path) = std::env::var_os("ISO2GOD_GPU_LOG") {
            use std::io::Write;
            if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(log, "{message}");
            }
        }
    }

    struct GpuHasher {
        input: Buffer<u8>,
        output: Buffer<u8>,
        kernel: Kernel,
        queue: CommandQueue,
        _program: Program,
        _context: Context,
    }

    impl GpuHasher {
        fn new() -> Result<Self> {
            let devices = get_all_devices(CL_DEVICE_TYPE_GPU)?;
            let id = devices.iter().copied().find(|id| {
                Device::new(*id).vendor().is_ok_and(|v| v.to_ascii_lowercase().contains("nvidia"))
            }).or_else(|| devices.first().copied()).context("no OpenCL GPU")?;
            let device = Device::new(id);
            let context = Context::from_device(&device)?;
            let queue = CommandQueue::create_default(&context, 0)?;
            let program = Program::create_and_build_from_source(&context, include_str!("sha1.cl"), "")
                .map_err(|e| anyhow!("OpenCL kernel build: {e}"))?;
            let kernel = Kernel::create(&program, "sha1_blocks")?;
            let input = unsafe { Buffer::create(&context, CL_MEM_READ_ONLY, CAPACITY, ptr::null_mut())? };
            let output = unsafe { Buffer::create(&context, CL_MEM_WRITE_ONLY, CAPACITY / BLOCK_BYTES * 20, ptr::null_mut())? };
            let mut gpu = Self { input, output, kernel, queue, _program: program, _context: context };
            // Fail closed on an incorrect kernel or device result before converting anything.
            let sample: Vec<u8> = (0..(BLOCK_BYTES*257))
                .map(|i| ((i*73 + i/256) % 251) as u8).collect();
            ensure!(gpu.run(&sample)? == cpu_hashes(&sample), "GPU SHA-1 self-test failed");
            // A representative dispatch catches large-batch issues and includes
            // the upload/download cost in the actual speed comparison.
            let sample: Vec<u8> = (0..CAPACITY)
                .map(|i| ((i*73 + i/256 + i/4096) % 251) as u8).collect();
            let start = std::time::Instant::now();
            let expected = cpu_hashes(&sample);
            let cpu_time = start.elapsed();
            let start = std::time::Instant::now();
            let actual = gpu.run(&sample)?;
            let gpu_time = start.elapsed();
            ensure!(actual == expected, "GPU full-batch SHA-1 self-test failed");
            report(&format!("ISO2GOD: {}: CPU {:.1} MiB/s, GPU {:.1} MiB/s (hashing and transfers)",
                device.name().unwrap_or_default(),
                CAPACITY as f64 / 1048576.0 / cpu_time.as_secs_f64(),
                CAPACITY as f64 / 1048576.0 / gpu_time.as_secs_f64()));
            if std::env::var("ISO2GOD_GPU").as_deref() == Ok("hybrid") {
                ensure!(gpu_time < cpu_time, "GPU did not beat one CPU worker in calibration; CPU selected");
            }
            report(&format!("ISO2GOD: OpenCL SHA-1 enabled on {}", device.name().unwrap_or_default()));
            Ok(gpu)
        }

        fn run(&mut self, data: &[u8]) -> Result<Vec<[u8; 20]>> {
            let full_len = data.len() / BLOCK_BYTES * BLOCK_BYTES;
            ensure!(full_len <= CAPACITY, "GPU batch exceeds capacity");
            let count = full_len / BLOCK_BYTES;
            let mut result = Vec::with_capacity(count+1);
            if count != 0 {
                let mut bytes = vec![0u8; count*20];
                unsafe {
                    self.queue.enqueue_write_buffer(&mut self.input, CL_BLOCKING, 0, &data[..full_len], &[])?;
                    let event = ExecuteKernel::new(&self.kernel)
                        .set_arg(&self.input).set_arg(&self.output)
                        .set_global_work_size(count).enqueue_nd_range(&self.queue)?;
                    self.queue.enqueue_read_buffer(&self.output, CL_BLOCKING, 0, &mut bytes, &[event.get()])?;
                }
                result.extend(bytes.chunks_exact(20).map(|b| <[u8; 20]>::try_from(b).unwrap()));
            }
            if full_len != data.len() {
                result.push(Sha1::digest(&data[full_len..]).into());
            }
            Ok(result)
        }
    }

    pub(super) fn try_hashes(data: &[u8]) -> Option<Vec<[u8; 20]>> {
        let hybrid = match std::env::var("ISO2GOD_GPU").as_deref() {
            Ok("hybrid") => true,
            Ok("1") => false,
            _ => return None,
        };
        // A tiny tail is cheaper on the CPU than a GPU dispatch.
        if data.len() < BLOCK_BYTES * 128 {
            return None;
        }
        // A single reusable device queue and buffers; CPU workers retain parallel I/O.
        let gpu = GPU.get_or_init(|| Mutex::new(match GpuHasher::new() {
            Ok(gpu) => Some(gpu),
            Err(e) => { report(&format!("ISO2GOD: GPU unavailable, using CPU: {e:#}")); None }
        }));
        // In hybrid mode, never stall a CPU worker behind a busy GPU.
        // It hashes its own batch while the GPU handles another worker's batch.
        let mut gpu = if hybrid { gpu.try_lock().ok()? } else { gpu.lock().ok()? };
        match gpu.as_mut()?.run(data) {
            Ok(hashes) => {
                // Cheap sanity check of real data on every GPU dispatch; the
                // full-batch self-test above verifies every hash at startup.
                let mid = hashes.len()/2;
                let offset = mid * BLOCK_BYTES;
                let expected: [u8;20] = Sha1::digest(
                    &data[offset..(offset+BLOCK_BYTES).min(data.len())]
                ).into();
                if hashes[mid] == expected {
                    Some(hashes)
                } else {
                    report("ISO2GOD: GPU returned wrong SHA-1, disabling GPU and rehashing on CPU");
                    *gpu = None;
                    None
                }
            },
            Err(e) => {
                report(&format!("ISO2GOD: GPU failed, using CPU: {e:#}"));
                *gpu = None;
                None
            }
        }
    }

    #[test]
    #[ignore = "requires an OpenCL GPU"]
    fn device_hashes_match_cpu_for_full_and_partial_batches() {
        let mut gpu = GpuHasher::new().unwrap();
        for size in [0, 1, 4095, 4096, 4097, 204*4096, CAPACITY, CAPACITY-31] {
            let data: Vec<u8> = (0..size).map(|n| ((n*17+n/139)%253) as u8).collect();
            assert_eq!(gpu.run(&data).unwrap(), cpu_hashes(&data), "size {size}");
        }
    }
}
