//! Reference implementation of example 18-multigather.
//!
//! Two output arrays from one input array `a`:
//!   p[i] = a[i] + a[i]        (= 2*a, the first loop output)
//!   q[i] = p[i] + a[i]        (= 3*a, the second loop output)
//!
//! The Nucleus side runs `p` and `q` as TWO loop outputs on ONE
//! (w0 -> host) channel under the distributed schedule; this reference
//! is unaware of any worker split — it is the canonical sequential
//! answer the differential tier-1 test compares against.
//!
//! Independence (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with kernels.rs, std only.
//!
//! Input format (`input.bin`):
//!   - bytes [0     ..   4*N) — array `a`, N i32 little-endian words.
//!
//! Output format (`reference.bin`):
//!   - bytes [0     ..   4*N) — array `p = 2*a`, N i32 LE words.
//!   - bytes [4*N   .. 4*2*N) — array `q = 3*a`, N i32 LE words.
//!
//! N is fixed at 64 to match `const N : usize = 64` in
//! `prog.algo.nuc`. If that const changes, this binary must change in
//! the same commit (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! `i32::wrapping_add` makes overflow behaviour explicit; no threads,
//! no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 64;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = 2 * N * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

    // Tiny hand-rolled arg parser (policy §2 — auditable, not
    // feature-rich).
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
                eprintln!("usage: multigather-reference --in INPUT.bin --out OUTPUT.bin");
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

    // Decode `a`.
    let mut a = [0i32; N];
    for (i, slot) in a.iter_mut().enumerate() {
        let off = i * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]);
    }

    // Compute. Strict left-to-right per index; `wrapping_add` documents
    // the overflow contract.
    let mut p = [0i32; N];
    let mut q = [0i32; N];
    for i in 0..N {
        p[i] = a[i].wrapping_add(a[i]);
        q[i] = p[i].wrapping_add(a[i]);
    }

    // Encode and write output: p first, then q.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for v in &p {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    for v in &q {
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
    eprintln!("multigather-reference: {}", msg);
    ExitCode::FAILURE
}
