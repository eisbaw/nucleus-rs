//! Reference implementation + fixture generator for example
//! 12-bitonic-sort (TASK-0044.06).
//!
//! Sorts the input array a SECOND, independent way: std's
//! `sort_unstable`. This is a deliberately DIFFERENT algorithm from
//! the Nucleus kernel's iterative bitonic compare-exchange network
//! — quicksort / pdqsort vs O(N log²(N)) static-network sort. The
//! point of the oracle is to be "wrong in a different way if wrong
//! at all" (docs/reference-impl-policy.md §2). If the bitonic
//! Nucleus output matches std's sort_unstable bit-for-bit on all 7
//! tier-1 backends, the network is correct.
//!
//! `sort_unstable` is order-irrelevant for equal keys, but the
//! committed fixture has DISTINCT values (a derangement of [0, N)),
//! so stability is not a question.
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads,
//! no HashMap, no Instant-derived values. std's sort lives entirely
//! in the std crate.
//!
//! Two subcommands:
//!   --gen-input PATH   write the canonical `input.bin` (N i32 LE
//!                      words) so there is NO python fixture step.
//!   --in PATH --out PATH
//!                      read `input.bin`, sort, write `reference.bin`
//!                      (N i32 LE words).
//!
//! N is fixed at the value declared in `prog.algo.nuc`. If that
//! const changes, this binary must change in the same commit
//! (policy §3). N must be a power of 2 for the Nucleus bitonic
//! kernel; std's sort_unstable has no such restriction.
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! distinct values; `sort_unstable` is deterministic (pdqsort
//! variant in current stable Rust, deterministic for fixed input).
//!
//! Input distribution (--gen-input)
//! ---------------------------------
//! `input[i] = ((i * 53 + 13) % N) * 7 + 100`. With N=64 and 53
//! coprime to 64 (53 is odd), `i * 53 mod 64` is a permutation of
//! [0, 64). Therefore the input is a permutation of
//! `{ k * 7 + 100 : 0 <= k < 64 }` = { 100, 107, 114, ..., 541 }.
//! Distinct, well-separated, well inside i32 range. The sorted
//! output is the same set in increasing order: 100, 107, 114, ...,
//! 541.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 64;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 3 && args[1] == "--gen-input" {
        return gen_input(&args[2]);
    }
    if args.len() == 5 && args[1] == "--in" && args[3] == "--out" {
        return run_reference(&args[2], &args[4]);
    }
    eprintln!(
        "usage:\n  bitonic-sort-reference --gen-input PATH\n  bitonic-sort-reference --in IN_PATH --out OUT_PATH"
    );
    ExitCode::FAILURE
}

fn gen_input(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    for i in 0..N {
        let v: i32 = (((i as i32) * 53 + 13).rem_euclid(N as i32)) * 7 + 100;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("--gen-input: cannot create {}: {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&bytes) {
        eprintln!("--gen-input: write failed: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_reference(in_path: &str, out_path: &str) -> ExitCode {
    let bytes = match fs::read(in_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("--in: cannot read {}: {}", in_path, e);
            return ExitCode::FAILURE;
        }
    };
    if bytes.len() < INPUT_BYTES {
        eprintln!(
            "--in: file {} has {} bytes; need at least {}",
            in_path,
            bytes.len(),
            INPUT_BYTES
        );
        return ExitCode::FAILURE;
    }
    let mut a = Vec::with_capacity(N);
    for k in 0..N {
        let off = k * BYTES_PER_WORD;
        a.push(i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]));
    }
    // Independent sort algorithm: std's sort_unstable (pdqsort).
    // Different control structure from the bitonic compare-exchange
    // network in kernels.rs.
    a.sort_unstable();

    let mut out_bytes = Vec::with_capacity(N * BYTES_PER_WORD);
    for v in &a {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = match fs::File::create(out_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("--out: cannot create {}: {}", out_path, e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&out_bytes) {
        eprintln!("--out: write failed: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
