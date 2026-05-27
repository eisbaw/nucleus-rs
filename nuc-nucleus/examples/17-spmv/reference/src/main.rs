//! Reference implementation of example 17-spmv. Landed cycle 210
//! (TASK-0341.03 AC#1 language-sanity slice).
//!
//! Sparse Matrix-Vector multiply on a dense-stored sparse matrix:
//!
//!   y[i] = sum_k val[i][k] * x[col_idx[i][k]]
//!
//! for i in 0..M, k in 0..NNZ. The reference uses the natural
//! direct-index form `x[col_idx[i][k]]`; the algorithm's
//! prog.algo.nuc spells the same computation via a rectangular
//! masked-accumulator nest (kernel `spmv_step` does `acc + v*x_j`
//! iff `j == c` else `acc`). The two access patterns are
//! structurally different but compute the same SpMV result —
//! per docs/reference-impl-policy.md §2 ("not wrong in the same
//! way"), output bit-identity is what validates the lowering.
//!
//! Input format (`input.bin`, 224 bytes):
//!   - bytes [0   ..  96)  val[M][NNZ]    — M*NNZ i32 LE, row-major.
//!   - bytes [96  .. 192)  col_idx[M][NNZ] — M*NNZ i32 LE, row-major.
//!   - bytes [192 .. 224)  x[N]            — N i32 LE.
//!
//! Output format (`reference.bin`, 32 bytes):
//!   - bytes [0 .. 32) — y[M], M i32 LE words.
//!
//! M = 8, N = 8, NNZ = 3 to match `prog.algo.nuc`. If those consts
//! change, this binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only; `wrapping_mul` for the product,
//!     `wrapping_add` for the accumulator.
//!   - Strict outer-to-inner traversal (i, then k).
//!   - No threads, no HashMap iteration, no Instant-derived values.
//!   - Strict bounds check: `col_idx[i][k]` is validated to be in
//!     [0, N) so a fixture drift surfaces here rather than as a
//!     silent zero contribution (the algorithm's masked-accumulator
//!     would silently sum zero on every j if no j matches an
//!     out-of-range col_idx).

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const M: usize = 8;
const N: usize = 8;
const NNZ: usize = 3;
const BYTES_PER_WORD: usize = 4;
const VAL_BYTES: usize = M * NNZ * BYTES_PER_WORD;
const COL_IDX_BYTES: usize = M * NNZ * BYTES_PER_WORD;
const X_BYTES: usize = N * BYTES_PER_WORD;
const VAL_OFFSET: usize = 0;
const COL_IDX_OFFSET: usize = VAL_BYTES;
const X_OFFSET: usize = VAL_BYTES + COL_IDX_BYTES;
const INPUT_BYTES: usize = VAL_BYTES + COL_IDX_BYTES + X_BYTES;
const OUTPUT_BYTES: usize = M * BYTES_PER_WORD;

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
                eprintln!("usage: spmv-reference --in INPUT.bin --out OUTPUT.bin");
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

    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (val + col_idx + x = {}+{}+{})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            VAL_BYTES,
            COL_IDX_BYTES,
            X_BYTES
        ));
    }

    // Decode val[M][NNZ], row-major.
    let mut val = [[0i32; NNZ]; M];
    for i in 0..M {
        for k in 0..NNZ {
            let off = VAL_OFFSET + (i * NNZ + k) * BYTES_PER_WORD;
            val[i][k] = i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
        }
    }

    // Decode col_idx[M][NNZ], row-major. Validate each value is in
    // [0, N). The algorithm's masked-accumulator silently sums zero
    // for out-of-range col_idx (no j in 0..N matches); the reference
    // rejects loudly so fixture drift surfaces here.
    let mut col_idx = [[0i32; NNZ]; M];
    for i in 0..M {
        for k in 0..NNZ {
            let off = COL_IDX_OFFSET + (i * NNZ + k) * BYTES_PER_WORD;
            let c = i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
            if c < 0 || c >= N as i32 {
                return die(&format!(
                    "col_idx[{}][{}] = {} is out of range [0, N); N = {}",
                    i, k, c, N
                ));
            }
            col_idx[i][k] = c;
        }
    }

    // Decode x[N].
    let mut x = [0i32; N];
    for j in 0..N {
        let off = X_OFFSET + j * BYTES_PER_WORD;
        x[j] = i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]);
    }

    // Compute y[i] = sum_k val[i][k] * x[col_idx[i][k]].
    // Strict outer-to-inner traversal of (i, k); same arithmetic
    // surface as kernels.rs `spmv_step` (wrapping_mul +
    // wrapping_add).
    let mut y = [0i32; M];
    for i in 0..M {
        let mut acc: i32 = 0;
        for k in 0..NNZ {
            let c = col_idx[i][k] as usize;
            let prod = val[i][k].wrapping_mul(x[c]);
            acc = acc.wrapping_add(prod);
        }
        y[i] = acc;
    }

    // Encode and write output.
    let mut out_buf = Vec::with_capacity(OUTPUT_BYTES);
    for v in &y {
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

fn die(msg: &str) -> ExitCode {
    eprintln!("spmv-reference: {}", msg);
    ExitCode::FAILURE
}
