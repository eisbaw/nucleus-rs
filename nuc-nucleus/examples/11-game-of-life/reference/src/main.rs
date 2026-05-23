//! Reference implementation of example 11-game-of-life.
//!
//! Multi-iteration 1D stencil with toroidal wrap-around. Same
//! algorithm as `prog.algo.nuc`:
//!
//!   next[i] = step( cur[(i+N-1) % N], cur[i], cur[(i+1) % N] )
//!
//! where (independently re-implemented from `kernels.rs`, per
//! docs/reference-impl-policy.md §2):
//!
//!   step(l, m, r) = l + m + r       (wrapping)
//!
//! Iterated ITERS times. `cur` and `next` swap roles each iteration
//! (true double-buffer). The output is the final `cur` after ITERS
//! iterations.
//!
//! Independent of Nucleus per docs/reference-impl-policy.md §2:
//! depends on `std` only; does not share any code with kernels.rs or
//! the nucleus-compiler crate.
//!
//! Input format (`input.bin`):
//!   - bytes [0      ..   4*N) — array `seed`, N i32 LE words.
//!   - Total: 4 * N = 128 bytes for N=32.
//!
//! Output format (`reference.bin`):
//!   - bytes [0      ..   4*N) — array `result`, N i32 LE words
//!                                (the final generation after ITERS
//!                                iterations).
//!   - Total: 4 * N = 128 bytes for N=32.
//!
//! N=32 and ITERS=8 are fixed to match `const N : usize = 32;` and
//! `const ITERS : usize = 8;` in `prog.algo.nuc`. If either changes,
//! this binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `wrapping_add` documents the overflow contract; the committed
//!     input (values in 1..=7) + 8 iterations grows at most ~7 * 3^8
//!     ≈ 46k, far below i32::MAX — wrap is never tripped in practice.
//!   - No threads, no HashMap iteration, no Instant-derived values,
//!     no FMA, no f32, no SIMD reorder.
//!
//! Why the swap-buffer shape (not flat grid[t+1][i])
//! -------------------------------------------------
//! Policy §2 — independence: the reference's structural shape must
//! differ from the algorithm's so a bug in the SHARED conceptual
//! model (e.g. an off-by-one in the wrap-around) shows up as a
//! divergence rather than reproducing identically on both sides.
//! `prog.algo.nuc` exposes the iteration axis as the OUTER dimension
//! of a 2D `grid` array (so the schedule can pipeline it); the
//! reference uses two scalar Vec<i32> buffers and swaps them each
//! iteration — a different decomposition of the same recurrence. The
//! per-iteration step expression (`l + m + r` with wrapping_add) is
//! intentionally identical because that is the recurrence definition;
//! the buffer shape, loop structure, and boundary handling are
//! independently re-derived.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 32;
const ITERS: usize = 8;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;
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
                eprintln!(
                    "usage: game-of-life-reference --in INPUT.bin --out OUTPUT.bin"
                );
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

    // Decode the seed array into the first buffer.
    let mut cur: Vec<i32> = Vec::with_capacity(N);
    for i in 0..N {
        let off = i * BYTES_PER_WORD;
        cur.push(i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]));
    }
    let mut next: Vec<i32> = vec![0; N];

    // Iterate ITERS times. After each iteration, swap `cur` and
    // `next` so the next iteration reads the just-written buffer.
    // True double-buffer; matches the spirit of `transfer grid :
    // async, buffer=2` in pipelined.sched.nuc, but here implemented
    // straightforwardly without any concurrency.
    for _t in 0..ITERS {
        for i in 0..N {
            // Wrap-around indices for the three-tap stencil. `i + N
            // - 1` keeps the operand non-negative even at `i == 0`
            // (Rust's `usize` arithmetic would underflow at `i - 1`
            // when `i == 0`); modulo N then maps it back into 0..N.
            let li = (i + N - 1) % N;
            let mi = i;
            let ri = (i + 1) % N;
            // step(l, m, r) = l + m + r, with wrapping_add.
            next[i] = cur[li]
                .wrapping_add(cur[mi])
                .wrapping_add(cur[ri]);
        }
        // Swap buffers. Standard Vec::swap is constant-time
        // pointer/length swap on the headers (no element copy).
        std::mem::swap(&mut cur, &mut next);
    }

    // After ITERS swaps, `cur` holds generation ITERS — the final
    // state. Encode and write.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for v in &cur {
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
    eprintln!("game-of-life-reference: {}", msg);
    ExitCode::FAILURE
}
