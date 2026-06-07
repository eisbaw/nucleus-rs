//! Hand-written reference oracle for nuc example 28-bin-fsum.
//!
//! Independent of Nucleus (docs/reference-impl-policy.md §2): `std` only,
//! no dependency on the compiler or any generated source. It reproduces
//! the per-bin reproducible FLOAT SUM the distributed schedule computes.
//!
//! ## Lockstep with the generated code (load-bearing — TASK-0453.03)
//!
//! `combine=fsum` is reproducible only because the FOLD ORDER is fixed.
//! This oracle MUST fold in the identical order the codegen emits, or the
//! cross-backend differential would compare the backends against a
//! reference computed in a different order and (because IEEE-754 addition
//! is non-associative) reject byte-identical-and-correct output. The
//! order the codegen emits, and this oracle mirrors exactly:
//!
//!   1. Partition the outer `i` loop (0..N) into NUM_WORKERS contiguous
//!      slices: worker w owns `i in [w*PARTITION_SIZE, (w+1)*PARTITION_SIZE)`.
//!   2. Each worker folds its slice into a PRIVATE `partial[BINS]`
//!      pre-initialised to 0.0 (the fsum identity), in ASCENDING `i`
//!      order: `partial[key[i]] += val[i]`.
//!   3. The host element-wise sums the per-worker partials into
//!      `result[BINS]` (also pre-init 0.0) in WORKER-ID-ASCENDING order
//!      (w0, w1, w2, w3) — the deterministic host event-list order the
//!      backends share (TASK-0389): `result[b] += partial_w[b]`.
//!
//! All arithmetic is `f32`, matching the backends' scalar type, so the
//! rounding is identical down to the bit.
//!
//! ## Why this is NOT a single-pass sum (the residual this oracle pins)
//!
//! A naive single-pass `for i in 0..N { result[key[i]] += val[i] }` would
//! associate the additions differently and can round differently. This
//! oracle deliberately does NOT do that — it reproduces the partitioned
//! fold. `assert_order_is_load_bearing` below checks, on the committed
//! input, that the two orders actually differ for at least one bin, so a
//! future input that made them coincide (hiding the non-associativity)
//! fails loudly rather than silently weakening the test.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

// These constants MUST mirror the distributed schedule's geometry, NOT
// just the algorithm: `NUM_WORKERS` is the worker count in
// `schedules/distributed.sched.nuc` (host + w0..w3) and `PARTITION_SIZE`
// is the contiguous even-block the compiler assigns each worker under
// `partition=workers` (exact because `N % NUM_WORKERS == 0`). If the
// schedule's worker set changes, the fold order changes, so this oracle
// AND `reference.bin` must be regenerated — the cross-backend e2e
// differential is the backstop that catches a stale reference.
const N: usize = 256;
const BINS: usize = 16;
const NUM_WORKERS: usize = 4;
const PARTITION_SIZE: usize = N / NUM_WORKERS; // 64
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = 2 * N * BYTES_PER_WORD; // key stream + val stream
const OUTPUT_BYTES: usize = BINS * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--in" => {
                i += 1;
                if i >= args.len() {
                    return die("--in requires a path argument");
                }
                in_path = Some(args[i].clone());
            }
            "--out" => {
                i += 1;
                if i >= args.len() {
                    return die("--out requires a path argument");
                }
                out_path = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!("usage: bin-fsum-reference --in INPUT.bin --out OUTPUT.bin");
                return ExitCode::SUCCESS;
            }
            other => return die(&format!("unknown argument: {}", other)),
        }
        i += 1;
    }
    let in_path = match in_path {
        Some(p) => p,
        None => return die("missing required --in PATH"),
    };
    let out_path = match out_path {
        Some(p) => p,
        None => return die("missing required --out PATH"),
    };

    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (2 * N * 4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }
    let word_bytes = |idx: usize| -> [u8; 4] {
        let off = idx * BYTES_PER_WORD;
        [bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]
    };

    let mut key = [0i32; N];
    for (i, slot) in key.iter_mut().enumerate() {
        *slot = i32::from_le_bytes(word_bytes(i));
        if *slot < 0 || *slot >= BINS as i32 {
            return die(&format!(
                "key[{}] = {} is out of range [0, BINS); BINS = {}",
                i, *slot, BINS
            ));
        }
    }
    let mut val = [0f32; N];
    for (i, slot) in val.iter_mut().enumerate() {
        *slot = f32::from_le_bytes(word_bytes(N + i));
        if !slot.is_finite() || *slot <= 0.0 {
            return die(&format!(
                "val[{}] = {} must be strictly positive AND finite (NaN/Inf/0/negative \
                 are rejected so the fixed-order fold is well-behaved)",
                i, *slot
            ));
        }
    }

    let result = fixed_order_fold(&key, &val);

    // Pin the residual: the fixed partitioned fold must DIFFER from a
    // naive single-pass sum for at least one bin on this input, proving
    // the fold order is load-bearing (not a trivially-exact sum).
    if let Err(msg) = assert_order_is_load_bearing(&key, &val, &result) {
        return die(&msg);
    }

    let mut out_buf = Vec::with_capacity(OUTPUT_BYTES);
    for v in &result {
        out_buf.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(out_buf.len(), OUTPUT_BYTES);
    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&out_buf) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }
    ExitCode::SUCCESS
}

/// The worker-id-sorted partitioned fold — IDENTICAL to the codegen's
/// fixed fold order (see module doc). This is the reproducible `fsum`.
fn fixed_order_fold(key: &[i32; N], val: &[f32; N]) -> [f32; BINS] {
    let mut result = [0f32; BINS];
    for w in 0..NUM_WORKERS {
        let lo = w * PARTITION_SIZE;
        let hi = lo + PARTITION_SIZE;
        let mut partial = [0f32; BINS];
        for i in lo..hi {
            // Ascending `i` within the worker slice — the worker loop order.
            partial[key[i] as usize] += val[i];
        }
        // Host fan-in, worker-id ascending.
        for b in 0..BINS {
            result[b] += partial[b];
        }
    }
    result
}

/// A naive single-pass sum, used ONLY to confirm the fold order matters.
fn naive_single_pass(key: &[i32; N], val: &[f32; N]) -> [f32; BINS] {
    let mut result = [0f32; BINS];
    for i in 0..N {
        result[key[i] as usize] += val[i];
    }
    result
}

/// Fail loud if the partitioned fold coincides bit-for-bit with the naive
/// single-pass sum on every bin — that would mean the input happens to be
/// exactly summable and the example would no longer demonstrate that the
/// FIXED ORDER is what buys reproducibility.
fn assert_order_is_load_bearing(
    key: &[i32; N],
    val: &[f32; N],
    fixed: &[f32; BINS],
) -> Result<(), String> {
    let naive = naive_single_pass(key, val);
    let differs = (0..BINS).any(|b| naive[b].to_bits() != fixed[b].to_bits());
    if differs {
        Ok(())
    } else {
        Err(
            "input is degenerate: the partitioned fold equals the naive single-pass sum on \
             EVERY bin, so this fixture would not demonstrate that the fixed fold order is \
             load-bearing. Regenerate input.bin with values that genuinely round (see README)."
                .to_string(),
        )
    }
}

fn die(msg: &str) -> ExitCode {
    eprintln!("bin-fsum-reference: {}", msg);
    ExitCode::FAILURE
}
