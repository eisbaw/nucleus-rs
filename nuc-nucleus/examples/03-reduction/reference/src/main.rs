//! Reference implementation of example 03-reduction.
//!
//! Two-phase integer sum:
//!   Phase 1: per-partition accumulation. Read N i32 LE words, fold
//!            each `PARTITION_SIZE`-wide partition into `partials[w]`
//!            via `i32::wrapping_add`.
//!   Phase 2: tree combine of the four partials into a single i32
//!            scalar via `i32::wrapping_add`.
//!
//! Independence (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with kernels.rs, std only. The point of
//! the reference is to be a second, hand-audited witness: a backend
//! whose output matches the reference bit-for-bit is unlikely to be
//! "wrong in the same way" as the reference.
//!
//! Independence from example 01/02 references is also deliberate:
//! no shared helper crate. Two small programs are cheaper to audit
//! than one shared library; each example's reference is self-
//! contained so its diff/regen story stands alone.
//!
//! Input format (`input.bin`):
//!   - bytes [0 .. 4*N) — N i32 little-endian words, laid out in
//!     row-major order matching the algorithm's
//!     `a : i32[NUM_WORKERS][PARTITION_SIZE]` shape. Partition w
//!     occupies bytes [4*w*PARTITION_SIZE .. 4*(w+1)*PARTITION_SIZE).
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4) — the single i32 LE scalar `result`.
//!
//! N, NUM_WORKERS, PARTITION_SIZE are fixed at the values declared in
//! `prog.algo.nuc`. If those consts change, this binary must change
//! in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_add` makes overflow behaviour explicit; the
//!     committed fixture stays in-range, but the choice documents
//!     intent.
//!   - Strict left-to-right per partition and over partials. No
//!     parallelism, no threads, no HashMap iteration, no
//!     Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const NUM_WORKERS: usize = 4;
const PARTITION_SIZE: usize = N / NUM_WORKERS;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` for two flags
    // would violate the "auditable, not feature-rich" principle in
    // policy §2.
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
                eprintln!("usage: reduction-reference --in INPUT.bin --out OUTPUT.bin");
                return ExitCode::SUCCESS;
            }
            other => {
                return die(&format!("unknown argument: {}", other));
            }
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

    // Read input. Validate size strictly: silent acceptance of
    // wrong-length files would mask fixture drift.
    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (N * 4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }

    // Decode array. `enumerate` over the array's iter_mut keeps
    // clippy::needless_range_loop happy while staying strict
    // left-to-right.
    let mut a = [0i32; N];
    for (i, slot) in a.iter_mut().enumerate() {
        let off = i * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Phase 1: per-partition accumulation. Strict left-to-right per
    // partition. `wrapping_add` documents the overflow contract.
    let mut partials = [0i32; NUM_WORKERS];
    for w in 0..NUM_WORKERS {
        for i in 0..PARTITION_SIZE {
            partials[w] = partials[w].wrapping_add(a[w * PARTITION_SIZE + i]);
        }
    }

    // Phase 2: tree combine. Same pairing as the algorithm.
    let half1 = partials[0].wrapping_add(partials[1]);
    let half2 = partials[2].wrapping_add(partials[3]);
    let result = half1.wrapping_add(half2);

    // Encode and write output.
    let out_bytes = result.to_le_bytes();
    assert_eq!(out_bytes.len(), OUTPUT_BYTES);

    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&out_bytes) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }

    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("reduction-reference: {}", msg);
    ExitCode::FAILURE
}
