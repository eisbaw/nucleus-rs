//! Reference implementation of example 07-matmul.
//!
//! Integer matrix multiplication C = A * B over 16x16 i32 matrices.
//! For each (i, j), compute
//!
//!     C[i][j] = sum_{k=0}^{N-1} A[i][k] * B[k][j]
//!
//! using `wrapping_mul` + `wrapping_add` (matches `kernels.rs::madd`
//! exactly — load-bearing for bit-identity).
//!
//! Independence (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with kernels.rs, std only. The point of
//! the reference is to be a second, hand-audited witness — if a
//! backend matches the reference bit-for-bit, we have evidence the
//! compiler is not "wrong in the same way".
//!
//! Input format (`input.bin`, 2048 bytes):
//!   - bytes [0    .. 1024) — A, row-major, N*N i32 LE words. A[i][j]
//!                            at byte offset (i * N + j) * 4.
//!   - bytes [1024 .. 2048) — B, row-major, N*N i32 LE words. B[i][j]
//!                            at byte offset 1024 + (i * N + j) * 4.
//!
//! Output format (`reference.bin`, 1024 bytes):
//!   - bytes [0 .. 1024) — C, row-major, N*N i32 LE words.
//!
//! N = 16; matches `const N : usize = 16;` in `prog.algo.nuc`. If
//! that constant changes, this binary must change in the same
//! commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_mul` / `wrapping_add` document the overflow
//!     contract; the committed fixtures stay inside the safe range.
//!   - No threads, no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 16;
const ELEMS: usize = N * N;
const BYTES_PER_WORD: usize = 4;
const MATRIX_BYTES: usize = ELEMS * BYTES_PER_WORD;
const INPUT_BYTES: usize = 2 * MATRIX_BYTES;

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
                eprintln!("usage: matmul-reference --in INPUT.bin --out OUTPUT.bin");
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
            "input {} has {} bytes; expected exactly {} (2 * N*N*4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }

    // Decode A and B into flat row-major Vec<i32>s. Matches the
    // codegen's `Vec<i32>` convention (see kernels.rs header).
    let mut a = vec![0i32; ELEMS];
    let mut b = vec![0i32; ELEMS];
    for k in 0..ELEMS {
        let off_a = k * BYTES_PER_WORD;
        let off_b = MATRIX_BYTES + k * BYTES_PER_WORD;
        a[k] = i32::from_le_bytes([
            bytes[off_a],
            bytes[off_a + 1],
            bytes[off_a + 2],
            bytes[off_a + 3],
        ]);
        b[k] = i32::from_le_bytes([
            bytes[off_b],
            bytes[off_b + 1],
            bytes[off_b + 2],
            bytes[off_b + 3],
        ]);
    }

    // Compute. C is zero-initialised; the i/j/k triple loop folds
    // products into each c[i][j]. Identical k-order to the
    // algorithm's `for k : 0..N` so the wrapping-arithmetic sequence
    // is bit-identical to what the codegen emits.
    let mut c = vec![0i32; ELEMS];
    for i in 0..N {
        for j in 0..N {
            for k in 0..N {
                let x = a[i * N + k];
                let y = b[k * N + j];
                // Same expression as kernels.rs::madd —
                // load-bearing for bit-identity.
                c[i * N + j] = c[i * N + j].wrapping_add(x.wrapping_mul(y));
            }
        }
    }

    // Encode and write output. Pre-size the buffer so the file
    // length is exactly MATRIX_BYTES regardless of write granularity.
    let mut out_bytes = Vec::with_capacity(MATRIX_BYTES);
    for v in &c {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(out_bytes.len(), MATRIX_BYTES);

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
    eprintln!("matmul-reference: {}", msg);
    ExitCode::FAILURE
}
