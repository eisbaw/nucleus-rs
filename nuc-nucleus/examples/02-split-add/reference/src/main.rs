//! Reference implementation of example 02-split-add.
//!
//! Same arithmetic as example 01 (per-element `c[i] = a[i] + b[i]`
//! over N i32 little-endian words) — what's different is the
//! algorithm/schedule split that the Nucleus side exercises. The
//! reference impl is unaware of any "split": it is the canonical
//! sequential answer the differential tier-1 test compares against.
//!
//! Independence (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with kernels.rs, std only. The point of
//! the reference is to be a second, hand-audited witness — if a
//! backend matches the reference bit-for-bit, we have evidence the
//! compiler is not "wrong in the same way".
//!
//! Independence from example 01's reference is also deliberate: we
//! intentionally do NOT factor out a shared helper. Two small
//! programs are cheaper to audit than one shared library, and we
//! want each example's reference to be self-contained so its
//! diff/regen story stands alone.
//!
//! Input format (`input.bin`):
//!   - bytes [0       ..   4*N) — array `a`, N i32 little-endian words.
//!   - bytes [4*N     .. 4*2*N) — array `b`, N i32 little-endian words.
//!
//! Output format (`reference.bin`):
//!   - bytes [0       ..   4*N) — array `c = a + b`, N i32 LE words.
//!
//! N is fixed at 256 to match `const N : usize = 256` in
//! `prog.algo.nuc`. If that const changes, this binary must change
//! in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_add` makes overflow behaviour explicit; the
//!     committed fixtures stay within range, but the choice
//!     documents intent.
//!   - No threads, no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = 2 * N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = N * BYTES_PER_WORD;

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
                eprintln!("usage: split-add-reference --in INPUT.bin --out OUTPUT.bin");
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
            "input {} has {} bytes; expected exactly {} (2 * N * 4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }

    // Decode arrays.
    let mut a = [0i32; N];
    let mut b = [0i32; N];
    for i in 0..N {
        let off_a = i * BYTES_PER_WORD;
        a[i] = i32::from_le_bytes([
            bytes[off_a],
            bytes[off_a + 1],
            bytes[off_a + 2],
            bytes[off_a + 3],
        ]);
        let off_b = (N + i) * BYTES_PER_WORD;
        b[i] = i32::from_le_bytes([
            bytes[off_b],
            bytes[off_b + 1],
            bytes[off_b + 2],
            bytes[off_b + 3],
        ]);
    }

    // Compute. Strict left-to-right per index; no parallelism, no
    // SIMD reordering. `wrapping_add` documents the overflow contract.
    let mut c = [0i32; N];
    for i in 0..N {
        c[i] = a[i].wrapping_add(b[i]);
    }

    // Encode and write output. Pre-size the buffer so the file
    // length is exactly OUTPUT_BYTES regardless of write granularity.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for v in &c {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
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
    eprintln!("split-add-reference: {}", msg);
    ExitCode::FAILURE
}
