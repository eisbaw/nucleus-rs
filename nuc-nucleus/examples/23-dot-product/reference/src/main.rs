//! Reference implementation + fixture generator for example
//! 23-dot-product (map-reduce / inner product).
//!
//! Computes the scalar inner product a SECOND, independent way from the
//! Nucleus program: a single flat left-to-right fold over all N
//! products, NOT the partition-then-tree-combine shape the Nucleus
//! program uses. `wrapping_add` is associative and commutative over
//! i32, so the flat fold and the partitioned tree reduction produce the
//! identical scalar — if the Nucleus output matches this bit-for-bit on
//! every backend, the map-reduce composition is sound
//! (docs/reference-impl-policy.md §2). The deliberately different
//! control structure (flat fold here vs partition+tree there) is what
//! makes this a genuine second witness rather than a copy.
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads, no
//! HashMap, no Instant-derived values.
//!
//! Two subcommands:
//!   --gen-input PATH      write the canonical `input.bin` (2*N i32 LE
//!                         words: a in [0..N), b in [N..2N)) so there
//!                         is NO python step.
//!   --in PATH --out PATH  read `input.bin`, compute the dot product,
//!                         write `reference.bin` (one i32 LE scalar).
//!
//! Input format (`input.bin`):
//!   - bytes [0     .. 4*N)   — vector a: N i32 LE words, row-major
//!                              over `a : i32[NUM_WORKERS][PARTITION_SIZE]`.
//!   - bytes [4*N   .. 8*N)   — vector b: N i32 LE words, same layout.
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4) — the single i32 LE scalar `result`.
//!
//! N, NUM_WORKERS, PARTITION_SIZE are fixed at the values declared in
//! `prog.algo.nuc`. If those consts change, this binary must change in
//! the same commit (policy §3). NUM_WORKERS / PARTITION_SIZE are not
//! actually needed for the flat fold, but are kept as documented
//! constants so the layout contract is explicit.
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_mul` / `i32::wrapping_add` make overflow explicit;
//!     the committed fixture stays well in-range, but the choice
//!     documents intent.
//!   - Strict left-to-right fold. No parallelism, no threads, no
//!     HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const NUM_WORKERS: usize = 4;
const PARTITION_SIZE: usize = N / NUM_WORKERS;
const BYTES_PER_WORD: usize = 4;
/// input.bin holds BOTH vectors back to back: a (N words) then b (N words).
const INPUT_BYTES: usize = 2 * N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = BYTES_PER_WORD;

fn main() -> ExitCode {
    // Touch the documented-but-fold-unused consts so the layout
    // contract is asserted, not just commented.
    debug_assert_eq!(NUM_WORKERS * PARTITION_SIZE, N);

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
                     dot-product-reference --gen-input INPUT.bin\n  \
                     dot-product-reference --in INPUT.bin --out REFERENCE.bin"
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
            "input {} has {} bytes; expected exactly {} (2 * N * 4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }

    // Decode both vectors. a occupies words [0..N), b occupies [N..2N).
    let mut a = [0i32; N];
    let mut b = [0i32; N];
    for (k, slot) in a.iter_mut().enumerate() {
        let off = k * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }
    for (k, slot) in b.iter_mut().enumerate() {
        let off = (N + k) * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Map-reduce, computed as a single flat left-to-right fold (the
    // independent control structure). `wrapping_add`'s commutativity is
    // what makes this equal the Nucleus partition+tree reduction.
    let mut result = 0i32;
    for k in 0..N {
        let prod = a[k].wrapping_mul(b[k]);
        result = result.wrapping_add(prod);
    }

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

/// Write the canonical `input.bin`: 2*N i32 LE words — vector a in
/// [0..N), vector b in [N..2N).
///
/// Value pattern (deterministic, small magnitudes so NO overflow):
///   a[k] = (k % 7) as i32 - 3   in [-3, 3]
///   b[k] = (k % 5) as i32 - 2   in [-2, 2]
/// Each product is in [-6, 6]; the full inner product over N=256 terms
/// is bounded by 256 * 6 = 1536 in absolute value — far inside the i32
/// range, so the `wrapping_*` ops never actually wrap on this fixture.
/// The two different small moduli (7 and 5) keep a and b out of phase
/// so the dot product is non-trivial (a transposed/dropped element
/// would change the result), not an accidental zero.
fn gen_input_file(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    // a first, then b — matching load_input ([0..N)) / load_input_b
    // ([N..2N)) and the algorithm's row-major partition layout.
    for k in 0..N {
        let v: i32 = (k % 7) as i32 - 3;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for k in 0..N {
        let v: i32 = (k % 5) as i32 - 2;
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
    eprintln!("dot-product-reference: {}", msg);
    ExitCode::FAILURE
}
