//! Reference implementation + fixture generator for example
//! 04-prefix-sum (TASK-0039).
//!
//! Computes the inclusive prefix sum a SECOND, independent way: a
//! single straight-line left-to-right running sum
//! (`acc = acc.wrapping_add(in[k]); out[k] = acc`). This is
//! deliberately a DIFFERENT algorithm from the Nucleus program's
//! three-pass block decomposition — the point of the oracle is to be
//! "wrong in a different way if wrong at all" (docs/reference-impl-
//! policy.md §2). If the block-decomposed Nucleus output matches this
//! naive running sum bit-for-bit on both backends, the decomposition
//! is sound.
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads,
//! no HashMap, no Instant-derived values.
//!
//! Two subcommands:
//!   --gen-input PATH   write the canonical `input.bin` (N i32 LE
//!                      words) so there is NO python fixture step.
//!   --in PATH --out PATH
//!                      read `input.bin`, compute the inclusive
//!                      prefix sum, write `reference.bin` (N i32 LE
//!                      words).
//!
//! Input/output format: bytes [0 .. 4*N) are N i32 little-endian
//! words. For the algorithm's `i32[NB][BS]` shape the file is
//! row-major (block b occupies words [b*BS .. (b+1)*BS)); a flat
//! prefix sum over file order is identical to the block-decomposed
//! result because blocks are contiguous and processed in order.
//!
//! N / NB / BS are fixed at the values declared in `prog.algo.nuc`.
//! If those consts change, this binary must change in the same commit
//! (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! `wrapping_add` makes overflow explicit; strict left-to-right.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const NB: usize = 4;
const BS: usize = N / NB;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;

fn main() -> ExitCode {
    // Compile-time sanity: keep the shape consts internally
    // consistent so a bad edit fails loudly here, not silently in
    // the fixture.
    assert_eq!(NB * BS, N, "NB*BS must equal N");

    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut gen_input: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` would violate
    // the "auditable, not feature-rich" principle (policy §2).
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
                     prefix-sum-reference --gen-input INPUT.bin\n  \
                     prefix-sum-reference --in INPUT.bin --out REFERENCE.bin"
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

    // Read input. Strict size validation: silent acceptance of a
    // wrong-length file would mask fixture drift.
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

    // Decode.
    let mut a = [0i32; N];
    for (k, slot) in a.iter_mut().enumerate() {
        let off = k * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Inclusive prefix sum — a single straight-line running total.
    // INTENTIONALLY a different algorithm from the Nucleus program's
    // three-pass block decomposition.
    let mut out = [0i32; N];
    let mut acc = 0i32;
    for k in 0..N {
        acc = acc.wrapping_add(a[k]);
        out[k] = acc;
    }

    // Encode and write.
    let mut bytes_out = Vec::with_capacity(N * BYTES_PER_WORD);
    for v in &out {
        bytes_out.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&bytes_out) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }

    ExitCode::SUCCESS
}

/// Write the canonical `input.bin`: N i32 LE words, value pattern
/// `a[k] = (k * 7) % 1000 - 500`. Same family as example 03's
/// pattern — deterministic, varies across k (not constant, not
/// monotonic so a dropped/swapped element shows up in the prefix
/// sums), and stays well inside i32 (values in [-500, 499]; the
/// running sum over N=256 stays small, no wraparound for the
/// committed fixture). Row-major over `i32[NB][BS]` is automatic
/// because a flat 0..N walk visits block b's BS words contiguously.
fn gen_input_file(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    for k in 0..N {
        let v: i32 = ((k as i32) * 7) % 1000 - 500;
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
    eprintln!("prefix-sum-reference: {}", msg);
    ExitCode::FAILURE
}
