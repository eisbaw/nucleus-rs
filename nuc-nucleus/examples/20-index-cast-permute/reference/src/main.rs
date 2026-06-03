//! Reference implementation of example 20-index-cast-permute.
//!
//! The algorithm is the REVERSAL permutation `out[idx(i)] <-- pass(in[i])`
//! with `idx(i) = N-1-i` (reversal bijection) and `pass(x) = x` (identity
//! passthrough). So `out` is `in` REVERSED: `out[N-1-i] = in[i]` for
//! every `i` in `0..N`.
//!
//! The example exists to exercise the TASK-0431 codegen path — a PURE
//! index kernel called with a BARE ITER VAR argument (`idx(i)`), which
//! forces the `(i) as i32` sidecar-driven cast in the generated crate.
//! The reversal (vs an identity, TASK-0431.01) makes the oracle
//! VALUE-DISCRIMINATING: a backend that emitted the cast but mis-evaluated
//! the index — or merely copied `in`→`out` without evaluating `idx` at
//! all — would now MISMATCH this reference, which an identity oracle
//! (`reference.bin == input.bin`) could not catch.
//!
//! Structural independence from kernels.rs (policy §2): this reference
//! does NOT call `idx`/`pass`; it computes the reversal-permutation
//! result directly with a `out[N-1-i] = in[i]` loop. A backend matching
//! it bit-for-bit is unlikely to be "wrong in the same way".
//!
//! Input format (`input.bin`):
//!   - bytes [0 .. 4*N) — N i32 little-endian words (the array `in`).
//!     The size is validated strictly (silent acceptance of a
//!     wrong-length file would mask fixture drift).
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4*N) — N i32 LE words, the array `out` (== `in` reversed).
//!
//! N is fixed at the value declared in `prog.algo.nuc`. If that const
//! changes, this binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only (here: none beyond the copy).
//!   - Strict left-to-right traversal of input.
//!   - No parallelism, no threads, no HashMap iteration, no
//!     Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = N * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` for two flags would
    // violate the "auditable, not feature-rich" principle in policy §2.
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
                eprintln!("usage: index-cast-permute-reference --in INPUT.bin --out OUTPUT.bin");
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

    // Read input. Validate SIZE strictly (wrong-length files mask fixture
    // drift). Values are arbitrary i32 — no range constraint applies.
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

    let mut input = [0i32; N];
    for (i, slot) in input.iter_mut().enumerate() {
        let off = i * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Reversal permutation: out[idx(i)] = pass(in[i]) with idx(i)=N-1-i
    // and pass the identity => out[N-1-i] = in[i], i.e. out is in
    // reversed. Strict left-to-right (computed directly, not via idx —
    // structural independence, policy §2).
    let mut out = [0i32; N];
    for (i, &v) in input.iter().enumerate() {
        out[N - 1 - i] = v;
    }

    // Encode and write output.
    let mut out_buf = Vec::with_capacity(OUTPUT_BYTES);
    for v in &out {
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
    eprintln!("index-cast-permute-reference: {}", msg);
    ExitCode::FAILURE
}
