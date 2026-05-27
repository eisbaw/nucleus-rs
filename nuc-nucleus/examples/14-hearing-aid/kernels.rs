// Kernel bodies for example 14-hearing-aid (tier-1 naive shape).
//
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// file, compiled by the host toolchain unmodified. Nucleus does NOT
// interpolate text into them.
//
// Six kernels:
//   - `load_mic()`     — () -> Vec<i32>, effectful. Reads
//                        N_FRAMES * SAMPLES_PER_FRAME i32 LE words
//                        from positions [0, mic_bytes) of `input.bin`
//                        (or `$NUC_INPUT_PATH`).
//   - `load_bt()`      — () -> Vec<i32>, effectful. Reads the next
//                        N_FRAMES * SAMPLES_PER_FRAME i32 LE words
//                        from positions [mic_bytes, mic_bytes+bt_bytes)
//                        of the same input file.
//   - `save_spk(v)`    — (Vec<i32>) -> (), effectful. Writes
//                        N_FRAMES * SAMPLES_PER_FRAME i32 LE words to
//                        positions [0, spk_bytes) of `output.bin` (or
//                        `$NUC_OUTPUT_PATH`). Appends/seeks so
//                        save_bt_out can write the next chunk.
//   - `save_bt_out(v)` — (Vec<i32>) -> (), effectful. Writes
//                        N_FRAMES * SAMPLES_PER_FRAME i32 LE words
//                        starting at position spk_bytes of the same
//                        output file.
//   - `mix2(a, b)`     — pure. Per-sample i32::wrapping_add of two
//                        SAMPLES_PER_FRAME buffers.
//   - `denoise(buf)`   — pure. 3-wide sliding sum with edge replication:
//                        out[i] = buf[max(i-1,0)] +
//                                 buf[i] +
//                                 buf[min(i+1, W-1)]
//                        (all i32::wrapping_add). Deterministic
//                        spectral-smoothing stand-in; NO FFT, NO
//                        division.
//
// Why bulk IO and not stateful per-frame peripheral kernels
// ---------------------------------------------------------
// See prog.algo.nuc header. Per-process stateful kernels (an
// AtomicUsize call counter inside fe_capture/etc.) would NOT survive
// multi-process backends (each process would have its own counter).
// Bulk IO is multi-process-safe by construction (each effectful
// kernel is called once per program run, on the host worker).
//
// IO file layout
// --------------
// input.bin (mic+bt concatenated, mic-first):
//   bytes [0 .. mic_bytes)              = mic_in flat row-major
//   bytes [mic_bytes .. mic_bytes+bt_bytes) = bt_in flat row-major
//   (mic_bytes = bt_bytes = N_FRAMES * SAMPLES_PER_FRAME * 4)
// output.bin (spk+bt_out concatenated, spk-first):
//   bytes [0 .. spk_bytes)              = spk_out flat row-major
//   bytes [spk_bytes .. spk_bytes+bt_out_bytes) = bt_out flat row-major
//
// I/O paths: same convention as the other examples — read
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else sibling filenames
// in cwd.

use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};

/// Mirror prog.algo.nuc consts. Per TASK-0103 (Done cycle 17):
/// `Vec<i32>` IS the canonical convention for aggregate-typed
/// kernel signatures; length checked at runtime.
const N_FRAMES: usize = 4;
const SAMPLES_PER_FRAME: usize = 16;
const FRAME_BYTES: usize = SAMPLES_PER_FRAME * 4;
const BUFFER_LEN: usize = N_FRAMES * SAMPLES_PER_FRAME;
const BUFFER_BYTES: usize = BUFFER_LEN * 4;

// ------------------------- pure kernels ----------------------------

pub fn mix2(a: Vec<i32>, b: Vec<i32>) -> Vec<i32> {
    assert_eq!(
        a.len(),
        SAMPLES_PER_FRAME,
        "mix2: a has {} elements; need {}",
        a.len(),
        SAMPLES_PER_FRAME
    );
    assert_eq!(
        b.len(),
        SAMPLES_PER_FRAME,
        "mix2: b has {} elements; need {}",
        b.len(),
        SAMPLES_PER_FRAME
    );
    let mut out = Vec::with_capacity(SAMPLES_PER_FRAME);
    for i in 0..SAMPLES_PER_FRAME {
        out.push(a[i].wrapping_add(b[i]));
    }
    out
}

pub fn denoise(buf: Vec<i32>) -> Vec<i32> {
    assert_eq!(
        buf.len(),
        SAMPLES_PER_FRAME,
        "denoise: buf has {} elements; need {}",
        buf.len(),
        SAMPLES_PER_FRAME
    );
    let n = SAMPLES_PER_FRAME;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let l = buf[if i > 0 { i - 1 } else { 0 }];
        let c = buf[i];
        let r = buf[if i + 1 < n { i + 1 } else { n - 1 }];
        out.push(l.wrapping_add(c).wrapping_add(r));
    }
    out
}

// ------------------------- effectful kernels -----------------------

pub fn load_mic() -> Vec<i32> {
    load_chunk(0)
}

pub fn load_bt() -> Vec<i32> {
    load_chunk(BUFFER_BYTES)
}

fn load_chunk(start: usize) -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_chunk: cannot read {}: {}", path, e));
    let end = start + BUFFER_BYTES;
    assert!(
        bytes.len() >= end,
        "load_chunk: file {} has {} bytes; need at least {} (BUFFER_BYTES={}, start={})",
        path,
        bytes.len(),
        end,
        BUFFER_BYTES,
        start
    );
    let mut out = Vec::with_capacity(BUFFER_LEN);
    for k in 0..BUFFER_LEN {
        let off = start + k * 4;
        out.push(i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]));
    }
    out
}

pub fn save_spk(v: Vec<i32>) {
    save_chunk(v, 0);
}

pub fn save_bt_out(v: Vec<i32>) {
    save_chunk(v, BUFFER_BYTES);
}

fn save_chunk(v: Vec<i32>, start: usize) {
    assert_eq!(
        v.len(),
        BUFFER_LEN,
        "save_chunk: v has {} elements; need {} (BUFFER_LEN)",
        v.len(),
        BUFFER_LEN
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    // The two save_* kernels write disjoint chunks at known offsets.
    // To make the file's final size deterministic regardless of which
    // kernel fires first (the schedule decides), we open in
    // create-if-needed mode, seek to the chunk start, write the chunk,
    // and let the OS file-size logic do the rest. The two
    // (save_spk, save_bt_out) chunks together cover [0, 2 * BUFFER_BYTES).
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("save_chunk: cannot open {}: {}", path, e));
    f.seek(SeekFrom::Start(start as u64))
        .unwrap_or_else(|e| panic!("save_chunk: seek to {} failed: {}", start, e));
    let mut bytes = Vec::with_capacity(BUFFER_BYTES);
    for w in v {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_chunk: write failed: {}", e));
}

// FRAME_BYTES is exported for documentation; not used by the kernels
// themselves (they operate on whole buffers).
#[allow(dead_code)]
const _FRAME_BYTES: usize = FRAME_BYTES;
