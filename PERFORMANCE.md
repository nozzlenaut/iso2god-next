# Experimental performance branch

Based on upstream iso2god-next 1.3.0, commit fa35128. No game-format changes:
the output still uses the original GOD headers, data parts and SHA-1 hash chains.

## What's changed
- Input bytes are read once rather than read, hashed, rewound and copied again.
- Each worker batches up to eight subparts (about 6.4 MiB) before hashing.
- Optional OpenCL computes SHA-1 for full 4096-byte blocks on a GPU; incomplete
  final blocks and the hash-table chain remain on the CPU.
- A GPU self-test compares the device's hashes with CPU SHA-1 before activation.
  If OpenCL is missing or fails, the CPU implementation is used.
- The GPU option is experimental. CPU is the default; the GPU is not automatically
  faster than a CPU with SHA instructions or a storage-limited conversion.

## Windows usage
Build using the upstream Windows GitHub Actions workflow, or build from source
using the original README's steps. In the folder containing the GUI executable,
place START_GPU.bat next to iso2god-next-gui.exe and double-click it. It enables
OpenCL via ISO2GOD_GPU=1. A current NVIDIA/AMD/Intel driver is required; a separate
CUDA toolkit is not required. The status log at %TEMP%\ISO2GOD-GPU.log reports the selected GPU or
the reason for a CPU fallback. Open the regular GUI executable for CPU mode.

Use a PC SSD for input and output while testing, and keep FTP off. Compare the
same input, trim settings, title and icon settings in separate empty output
directories. Compare timings and the resulting SHA-256 hashes of all package files
before trusting the GPU option. Test the converted game in Aurora as well.

## Tests
- cargo test --lib --test convert
- cargo test --lib --test convert --no-default-features
- cargo test --lib device_hashes_match_cpu_for_full_and_partial_batches -- --ignored
  (requires an OpenCL GPU)
- CPU vs GPU outputs must match byte-for-byte with the same settings.

The GPU path is opt-in until tested on real hardware. Do not overwrite your
only existing GOD copy when testing.
