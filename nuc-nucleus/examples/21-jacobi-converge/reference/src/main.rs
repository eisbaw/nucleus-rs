//! Reference implementation of example 21-jacobi-converge. Landed
//! cycle 262 (epic S5; TASK-0341.02.01.06 convergence-check variant).
//!
//! Five-point Jacobi iteration on a 2D H x W i32 grid with a Dirichlet
//! zero-boundary, run UNTIL the per-generation L-infinity convergence
//! scalar drops to TOL, OR a compile-time cap ITERS_CAP:
//!
//!   field[0][y][x]  = seed[y][x]
//!   field[t][y][x]  = (field[t-1][y-1][x] + field[t-1][y+1][x]
//!                    + field[t-1][y][x-1] + field[t-1][y][x+1]) / 4
//!                    (for t >= 1, y in 1..H-1, x in 1..W-1)
//!   maxdiff[t]      = max over the interior of |field[t] - field[t-1]|
//!                    (field[-1] modeled as the zero grid: maxdiff[0] is
//!                     max |seed| over the interior, non-zero)
//!   halt at the FIRST t with maxdiff[t] <= TOL; else run to ITERS_CAP.
//!
//! Output = the CONVERGED generation `field[k]` (k = the break gen on
//! early-exit, or ITERS_CAP on cap-hit), row-major, full grid (incl.
//! 0-valued boundary cells). This mirrors the compiler's runtime
//! break-generation final-read (TASK-0341.02.01.05.02): the converged
//! generation, NOT a fixed `field[ITERS_CAP]`.
//!
//! Independent of Nucleus per docs/reference-impl-policy.md §2: `std`
//! only; shares NO code with `kernels.rs` or the nucleus-compiler. The
//! abs-diff arithmetic is INTENTIONALLY identical in spelling to
//! `kernels.rs::abs_diff_i32` (overflow-safe i64-widening + i32::MAX
//! clamp, TASK-0436) so the differential is meaningful — that is a
//! deliberate semantic mirror, not a shared-code dependency.
//!
//! Input format (`input.bin`): H*W i32 LE words, row-major `[H][W]`
//! (the seed). Boundary cells in the seed are ignored (never copied
//! into the interior iteration; they stay 0 — Dirichlet BC).
//!
//! Output format (`reference.bin`): H*W i32 LE words = the converged
//! `field[k]`, row-major.
//!
//! H = 8, W = 8, ITERS_CAP = 64, TOL = 2 to match `prog.algo.nuc`. If
//! those consts change, this binary must change in the same commit
//! (policy §3). With the committed seed `maxdiff[t]` first drops to
//! <= TOL=2 at generation 30 (< cap), so the early-exit path is
//! exercised — the cap-hit branch is a defensive fallback the committed
//! input does NOT reach. TOL is 2 (not 0) deliberately: at the exact
//! integer fixed point (TOL=0) the converged interior is all-zero =
//! byte-identical to the unwritten cap slice, which would let a broken
//! (hard-coded `field[ITERS_CAP]`) compiler PASS; TOL=2 breaks while the
//! interior is still non-zero so the runtime final-read is load-bearing.
//!
//! `--gen-input` mode regenerates the committed deterministic seed
//! (`input.bin`) instead of running the algorithm. The seed pattern is
//! documented in README.md.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 8;
const W: usize = 8;
const ITERS_CAP: usize = 64;
const TOL: i32 = 2;
const BYTES_PER_WORD: usize = 4;
const IO_BYTES: usize = H * W * BYTES_PER_WORD;

/// Overflow-safe absolute difference, clamped to `i32::MAX`. IDENTICAL
/// in spelling to `kernels.rs::abs_diff_i32` (TASK-0436). i64-widening
/// + `unsigned_abs` has no `i32::MIN.abs()` panic path.
fn abs_diff_i32(n: i32, o: i32) -> i32 {
    let mag: u64 = (i64::from(n) - i64::from(o)).unsigned_abs();
    mag.min(i32::MAX as u64) as i32
}

/// Deterministic committed seed. Documented in README.md. Interior
/// cells only; the boundary stays 0 (Dirichlet BC). The pattern keeps
/// every cell in 0..256.
fn gen_seed() -> [[i32; W]; H] {
    let mut seed = [[0i32; W]; H];
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            seed[y][x] = (((y * 13 + x * 7) % 17) as i32) * 16 + 32;
        }
    }
    seed
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut gen_input = false;

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
            "--gen-input" => gen_input = true,
            "-h" | "--help" => {
                eprintln!(
                    "usage: jacobi-converge-reference --in INPUT.bin --out OUTPUT.bin\n       \
                     jacobi-converge-reference --gen-input --out INPUT.bin"
                );
                return ExitCode::SUCCESS;
            }
            other => return die(&format!("unknown argument: {}", other)),
        }
        i += 1;
    }

    // `--gen-input`: write the deterministic committed seed.
    if gen_input {
        let out_path = match out_path {
            Some(p) => p,
            None => return die("missing required --out PATH for --gen-input"),
        };
        let seed = gen_seed();
        let mut out_bytes = Vec::with_capacity(IO_BYTES);
        for y in 0..H {
            for x in 0..W {
                out_bytes.extend_from_slice(&seed[y][x].to_le_bytes());
            }
        }
        return write_out(&out_path, &out_bytes);
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
    if bytes.len() != IO_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (H*W*4, H={}, W={})",
            in_path,
            bytes.len(),
            IO_BYTES,
            H,
            W
        ));
    }

    // Decode the seed in row-major `[H][W]` layout. Only the interior
    // is used by the iteration (boundary stays 0, Dirichlet BC).
    let mut seed = [[0i32; W]; H];
    for y in 0..H {
        for x in 0..W {
            let off = (y * W + x) * BYTES_PER_WORD;
            seed[y][x] = i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
        }
    }

    // field[0..=ITERS_CAP]; boundary cells stay 0 throughout. field[0]
    // gets the seed interior (single-assignment: the boundary seed
    // cells are never copied, matching `prog.algo.nuc`).
    let mut field = vec![[[0i32; W]; H]; ITERS_CAP + 1];
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            field[0][y][x] = seed[y][x];
        }
    }

    // `prev` is the previous generation for the convergence reduction.
    // For t == 0 it is the zero grid (matching the algorithm's
    // `field[(0 + ITERS_CAP) % (ITERS_CAP + 1)]` = `field[ITERS_CAP]`,
    // pre-initialised to 0).
    let mut prev = [[0i32; W]; H];
    let mut break_gen: i64 = -1;

    for t in 0..=ITERS_CAP {
        // (a) Compute generation t (t==0 already holds the seed).
        if t >= 1 {
            for y in 1..H - 1 {
                for x in 1..W - 1 {
                    let pn = field[t - 1][y - 1][x];
                    let ps = field[t - 1][y + 1][x];
                    let pe = field[t - 1][y][x - 1];
                    let pw = field[t - 1][y][x + 1];
                    let sum = pn.wrapping_add(ps).wrapping_add(pe).wrapping_add(pw);
                    field[t][y][x] = sum / 4;
                }
            }
        }
        // (b) Per-generation L-infinity reduction over the interior.
        let mut maxdiff = 0i32;
        for y in 1..H - 1 {
            for x in 1..W - 1 {
                maxdiff = maxdiff.max(abs_diff_i32(field[t][y][x], prev[y][x]));
            }
        }
        // (c) Halt check (the `until maxdiff[t] <= TOL` clause). The
        //     break captures the generation `t` that broke.
        if maxdiff <= TOL {
            break_gen = t as i64;
            break;
        }
        // Advance prev <- field[t] (full grid, incl. boundary 0s).
        for y in 0..H {
            for x in 0..W {
                prev[y][x] = field[t][y][x];
            }
        }
    }

    // Resolve the final generation: the captured break gen, or the cap
    // (last computed) on cap-hit. Mirrors the compiler's
    // `__nuc_final_gen` (TASK-0341.02.01.05.02 / .05.03).
    let final_gen: usize = if break_gen < 0 {
        eprintln!(
            "[[nuc_converge]] reference: did NOT converge within the cap ({} + 1 generations); \
             extracting the last computed generation {}",
            ITERS_CAP, ITERS_CAP
        );
        ITERS_CAP
    } else {
        break_gen as usize
    };

    // Emit field[final_gen] in row-major order (full grid).
    let mut out_bytes = Vec::with_capacity(IO_BYTES);
    for y in 0..H {
        for x in 0..W {
            out_bytes.extend_from_slice(&field[final_gen][y][x].to_le_bytes());
        }
    }
    write_out(&out_path, &out_bytes)
}

fn write_out(out_path: &str, out_bytes: &[u8]) -> ExitCode {
    assert_eq!(out_bytes.len(), IO_BYTES);
    let mut f = match fs::File::create(out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(out_bytes) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }
    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("jacobi-converge-reference: {}", msg);
    ExitCode::FAILURE
}
