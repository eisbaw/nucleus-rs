//! Reference implementation + fixture generator for example
//! 24-outer-product (rank-1 / outer product / BLAS-2 `ger`).
//!
//! Computes the outer product a SECOND, independent way from the
//! Nucleus program:
//!
//!     c[i][j] = a[i] * b[j]        (i in 0..M, j in 0..N)
//!
//! using `wrapping_mul` (matches `kernels.rs::mul` exactly —
//! load-bearing for bit-identity). There is NO reduction: each output
//! element is a single independent product, so the result is order-
//! independent by construction. The reference fills `c` row-major in a
//! plain nested loop; the Nucleus program fires the same `mul` per
//! (i, j) — both must produce identical bytes
//! (docs/reference-impl-policy.md §2).
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads, no
//! HashMap, no Instant-derived values.
//!
//! Two subcommands:
//!   --gen-input PATH      write the canonical `input.bin` ((M+N) i32 LE
//!                         words: a in [0..M), b in [M..M+N)) so there
//!                         is NO python step.
//!   --in PATH --out PATH  read `input.bin`, compute the outer product,
//!                         write `reference.bin` (M*N i32 LE words, c
//!                         row-major).
//!
//! Input format (`input.bin`, (M+N)*4 = 96 bytes):
//!   - bytes [0       .. 4*M)     — vector a: M i32 LE words.
//!   - bytes [4*M     .. 4*(M+N)) — vector b: N i32 LE words.
//!
//! Output format (`reference.bin`, M*N*4 = 512 bytes):
//!   - bytes [0 .. 512) — matrix c, row-major: c[i][j] at byte offset
//!                        (i * N + j) * 4.
//!
//! M = 8, N = 16; match `const M : usize = 8;` / `const N : usize = 16;`
//! in `prog.algo.nuc`. If those consts change, this binary must change
//! in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_mul` makes overflow explicit; the committed
//!     fixture stays well in-range, but the choice documents intent.
//!   - No parallelism, no threads, no HashMap iteration, no
//!     Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const M: usize = 8;
const N: usize = 16;
const BYTES_PER_WORD: usize = 4;
/// input.bin holds BOTH vectors back to back: a (M words) then b (N words).
const INPUT_BYTES: usize = (M + N) * BYTES_PER_WORD;
/// output: c is M*N words row-major.
const OUTPUT_ELEMS: usize = M * N;
const OUTPUT_BYTES: usize = OUTPUT_ELEMS * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut gen_input: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` for a few flags
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
            "--gen-input" => {
                i += 1;
                if i >= args.len() {
                    return die("--gen-input requires a path argument");
                }
                gen_input = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage:\n  \
                     outer-product-reference --gen-input INPUT.bin\n  \
                     outer-product-reference --in INPUT.bin --out REFERENCE.bin"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                return die(&format!("unknown argument: {}", other));
            }
        }
        i += 1;
    }

    if let Some(p) = gen_input {
        return gen_input_file(&p);
    }

    let in_path = match in_path {
        Some(p) => p,
        None => return die("missing required --in PATH (or use --gen-input PATH)"),
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
            "input {} has {} bytes; expected exactly {} ((M+N)*4, M={}, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            M,
            N
        ));
    }

    // Decode both vectors. a occupies words [0..M), b occupies [M..M+N).
    let mut a = [0i32; M];
    let mut b = [0i32; N];
    for (k, slot) in a.iter_mut().enumerate() {
        let off = k * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }
    for (k, slot) in b.iter_mut().enumerate() {
        let off = (M + k) * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Outer product, row-major. No reduction: each c[i][j] is a single
    // independent product. `wrapping_mul` matches kernels.rs::mul —
    // load-bearing for bit-identity.
    let mut c = vec![0i32; OUTPUT_ELEMS];
    for i in 0..M {
        for j in 0..N {
            c[i * N + j] = a[i].wrapping_mul(b[j]);
        }
    }

    // Encode and write output. Pre-size the buffer so the file length
    // is exactly OUTPUT_BYTES regardless of write granularity.
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

/// Write the canonical `input.bin`: (M+N) i32 LE words — vector a in
/// [0..M), vector b in [M..M+N).
///
/// Value pattern (deterministic, small magnitudes so NO overflow):
///   a[i] = (i as i32) - 4   in [-4, 3]   (i in 0..M=8)
///   b[j] = (j as i32) - 8   in [-8, 7]   (j in 0..N=16)
/// Each product a[i]*b[j] is in [-32, 32] (|a| <= 4, |b| <= 8) — far
/// inside the i32 range, so `wrapping_mul` never actually wraps on this
/// fixture. The two offsets (-4, -8) make a and b straddle zero with
/// different magnitudes so the outer product has a non-trivial sign
/// pattern (a transposed/dropped element changes the bytes), not an
/// accidental all-zero or constant matrix.
fn gen_input_file(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    // a first, then b — matching load_a ([0..M)) / load_b ([M..M+N)).
    for i in 0..M {
        let v: i32 = (i as i32) - 4;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for j in 0..N {
        let v: i32 = (j as i32) - 8;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create {}: {}", path, e)),
    };
    if let Err(e) = f.write_all(&bytes) {
        return die(&format!("write to {} failed: {}", path, e));
    }
    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("outer-product-reference: {}", msg);
    ExitCode::FAILURE
}
