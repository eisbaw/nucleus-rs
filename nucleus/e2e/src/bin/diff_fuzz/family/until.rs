//! Family: bounded `for..until` single-worker convergence shape.
//!
//! A faithful PARAMETERISATION of the proven `21-jacobi-converge` example
//! (the only non-inert consumer of the `for..until` machinery): a 2-D
//! Jacobi `/4` iteration on a positive-bounded seed, with a per-generation
//! L-infinity convergence scalar and `for t : 0 .. CAP+1 until maxdiff[t]
//! <= TOL`. The compile-time `..CAP+1` cap keeps the loop statically
//! bounded; `maxdiff[t] <= TOL` is the EXACT integer halt predicate.
//!
//! # Backend scope — single-worker pthreads-sync ONLY (HONEST RESIDUAL)
//!
//! This family is NOT a 7-backend differential. The curated e2e matrix
//! itself `[[skip]]`s `21-jacobi-converge` on all six non-pthreads-sync
//! backends: "for..until break emit is tier-1 single-worker pthreads-sync
//! only" (the multi_worker_walker / tcp_plan walkers fail loud on a
//! `break_cond`; the 7-backend / multi-worker break differential is epic
//! S7, TASK-0341.02.01.08). So this family is checked for self-consistency
//! and agreement with the in-process reference on pthreads-sync ALONE
//! (`crate::program::Program::backends` returns the single-backend set
//! for it). It strengthens the harness's compiler-fault coverage of the
//! break-generation rewrite, NOT cross-backend agreement.
//!
//! # Why the generated instance is GUARANTEED to break before the cap
//!
//! The Jacobi `/4` step on a seed with every interior cell in `[0, 256)`
//! drives the interior monotonically toward the zero fixed point, so
//! `maxdiff[t]` is non-increasing and reaches `<= TOL` within a small
//! number of generations; CAP is set generously (>= 48) so the break
//! generation `k` is comfortably `< CAP`. The reference SIMULATES the
//! identical iteration to compute `k` and the converged generation
//! `field[k]` — which is exactly what the codegen's break-rewrite reads
//! (`field[CAP]` source-level, rewritten to `field[k]`). TOL is kept `>=
//! 2` so the break happens while the interior is still NON-ZERO, i.e.
//! distinguishable from the unwritten (all-zero) cap slice — the same
//! load-bearing reason `21-jacobi-converge` uses `TOL = 2`.

use std::fmt::Write as _;

use crate::program::{push_i32_le, SourceBundle};
use crate::rng::Rng;

/// Convergence tolerance. `>= 2` so the break-generation interior is
/// non-zero (distinguishable from the zero cap slice). Fixed, not random,
/// so the break-gen guarantee is easy to reason about.
const TOL: i32 = 2;

#[derive(Clone, Debug)]
pub(crate) struct ForUntil1d {
    h: usize,
    w: usize,
    cap: usize,
    /// Row-major seed grid, every cell in `[0, 256)`.
    seed: Vec<i32>,
}

impl ForUntil1d {
    pub(crate) fn generate(rng: &mut Rng) -> ForUntil1d {
        // Deterministic re-draw until the instance satisfies the two
        // properties the break-rewrite test depends on: it breaks STRICTLY
        // inside the cap (`0 < k < cap`, so the early-exit path is exercised
        // and the cap is not hit), AND the converged field is NON-ZERO
        // somewhere (so it is distinguishable from the unwritten all-zero
        // cap slice). A small `/4`-Jacobi grid with a low seed can converge
        // to all-zeros AT the break gen (e.g. seed 23) — that instance is
        // unusable because reading `field[k]` vs `field[cap]` would be
        // indistinguishable. Re-drawing consumes more RNG but stays fully
        // seed-deterministic. The retry is bounded; the fallback grid is
        // 8x8 with a mid-range constant seed (the proven curated shape),
        // which always satisfies both properties.
        const MAX_TRIES: usize = 64;
        for _ in 0..MAX_TRIES {
            let cand = Self::draw(rng);
            let (k, field) = cand.simulate();
            if k > 0 && k < cand.cap && field.iter().any(|&v| v != 0) {
                return cand;
            }
        }
        // Fallback: the curated 8x8 / mid-range constant seed shape, proven
        // to break inside the cap with a non-zero interior.
        ForUntil1d {
            h: 8,
            w: 8,
            cap: 64,
            seed: vec![100; 64],
        }
    }

    /// One raw draw (no validity filter). `generate` wraps this with the
    /// break-gen / non-zero-field guard.
    fn draw(rng: &mut Rng) -> ForUntil1d {
        // H,W >= 6 so the interior `(H-2)*(W-2) >= 16` is large enough that
        // a low seed rarely collapses to all-zeros at the break gen.
        let h = rng.range(6, 10) as usize;
        let w = rng.range(6, 10) as usize;
        // Generous cap; the break-gen for /4-Jacobi on positive seeds is
        // small (the curated 8x8 instance breaks at gen 30 with cap 64).
        let cap = rng.range(48, 80) as usize;
        // Seed values in [64, 255]: kept comfortably positive so the decay
        // toward zero leaves a non-zero interior at the (early) break gen.
        let seed: Vec<i32> = (0..h * w).map(|_| (rng.range(64, 255)) as i32).collect();
        ForUntil1d { h, w, cap, seed }
    }

    pub(crate) fn describe(&self) -> String {
        let (k, _) = self.simulate();
        format!(
            "for_until H={} W={} CAP={} TOL={} break_gen={}",
            self.h, self.w, self.cap, TOL, k
        )
    }

    fn at(grid: &[i32], w: usize, y: usize, x: usize) -> i32 {
        grid[y * w + x]
    }

    /// One Jacobi step producing generation `cur` from `prev`. Mirrors
    /// `jacobi5_or_seed`: interior (1..H-1, 1..W-1) is `(N+S+E+W)/4`
    /// (truncating); the boundary ring is NOT written and stays 0. Matches
    /// the curated kernel + the `for y : 1..H-1 / for x : 1..W-1` nest.
    fn jacobi_step(&self, prev: &[i32], gen0: bool) -> Vec<i32> {
        let mut cur = vec![0i32; self.h * self.w];
        for y in 1..self.h - 1 {
            for x in 1..self.w - 1 {
                let v = if gen0 {
                    // t == 0: seed fallback.
                    Self::at(&self.seed, self.w, y, x)
                } else {
                    let n = Self::at(prev, self.w, y - 1, x);
                    let s = Self::at(prev, self.w, y + 1, x);
                    let e = Self::at(prev, self.w, y, x - 1);
                    let wv = Self::at(prev, self.w, y, x + 1);
                    n.wrapping_add(s).wrapping_add(e).wrapping_add(wv) / 4
                };
                cur[y * self.w + x] = v;
            }
        }
        cur
    }

    /// Overflow-safe |n-o| clamped to i32::MAX (identical to the curated
    /// `abs_diff_i32`). The reference MUST match the emitted kernel.
    fn abs_diff(n: i32, o: i32) -> i32 {
        let mag: u64 = (i64::from(n) - i64::from(o)).unsigned_abs();
        mag.min(i32::MAX as u64) as i32
    }

    /// Per-generation L-infinity max-abs-diff over the interior.
    fn maxdiff(&self, cur: &[i32], prev: &[i32]) -> i32 {
        let mut acc = 0i32;
        for y in 1..self.h - 1 {
            for x in 1..self.w - 1 {
                acc = acc.max(Self::abs_diff(
                    Self::at(cur, self.w, y, x),
                    Self::at(prev, self.w, y, x),
                ));
            }
        }
        acc
    }

    /// Simulate the bounded loop, returning `(break_gen, field[break_gen])`.
    /// `break_gen` is the first `t` with `maxdiff[t] <= TOL`, or `CAP` if
    /// the predicate never trips within the cap (the cap-hit branch). The
    /// returned field is the converged generation the codegen break-rewrite
    /// reads.
    ///
    /// The prev-generation slice uses the `(t + CAP) % (CAP+1)` index trick:
    /// at t==0 the "previous" slot is the zero-initialised cap slice (all
    /// zero); for t>=1 it is generation t-1. We model that by treating the
    /// t==0 previous as an all-zero grid.
    fn simulate(&self) -> (usize, Vec<i32>) {
        let zero = vec![0i32; self.h * self.w];
        let mut prev = zero.clone();
        let mut cur;
        for t in 0..=self.cap {
            cur = self.jacobi_step(if t == 0 { &zero } else { &prev }, t == 0);
            let md = self.maxdiff(&cur, &prev);
            if md <= TOL {
                return (t, cur);
            }
            prev = cur;
        }
        // Cap hit without convergence — return the cap generation. (For
        // the bounded /4 iteration on [0,256) seeds this is unreachable in
        // practice, but the branch keeps the oracle total.)
        (self.cap, prev)
    }

    fn reference(&self) -> Vec<u8> {
        let (_, field_k) = self.simulate();
        let mut bytes = Vec::with_capacity(field_k.len() * 4);
        push_i32_le(&mut bytes, &field_k);
        bytes
    }

    fn input(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.seed.len() * 4);
        push_i32_le(&mut bytes, &self.seed);
        bytes
    }

    fn algo_src(&self, seed: u64) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "// GENERATED by diff_fuzz (seed={seed}). Bounded for..until Jacobi convergence."
        );
        let _ = writeln!(s, "const H         : usize = {};", self.h);
        let _ = writeln!(s, "const W         : usize = {};", self.w);
        let _ = writeln!(s, "const ITERS_CAP : usize = {};", self.cap);
        let _ = writeln!(s, "const TOL       : i32   = {TOL};");
        let _ = writeln!(s);
        let _ = writeln!(s, "data seed     : i32[H][W];");
        let _ = writeln!(s, "data field    : i32[ITERS_CAP+1][H][W];");
        let _ = writeln!(s, "data partials : i32[ITERS_CAP+1];");
        let _ = writeln!(s, "data maxdiff  : i32[ITERS_CAP+1];");
        let _ = writeln!(s, "data result   : i32[H][W];");
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "kernel jacobi5_or_seed : (i32, i32, i32, i32, i32, i32) -> i32 pure;"
        );
        let _ = writeln!(s, "kernel max_abs_acc     : (i32, i32, i32) -> i32 pure;");
        let _ = writeln!(s, "kernel ident           : (i32) -> i32 pure;");
        let _ = writeln!(s, "kernel load_input  : ()         -> i32[H][W] effectful;");
        let _ = writeln!(s, "kernel save_output : (i32[H][W]) -> ()        effectful;");
        let _ = writeln!(s);
        let _ = writeln!(s, "seed <-- load_input();");
        let _ = writeln!(s);
        let _ = writeln!(s, "for t : 0 .. ITERS_CAP+1 until maxdiff[t] <= TOL {{");
        let _ = writeln!(s, "    for y : 1 .. H-1 {{");
        let _ = writeln!(s, "    for x : 1 .. W-1 {{");
        let _ = writeln!(s, "        field[t][y][x] <-- jacobi5_or_seed(");
        let _ = writeln!(s, "            field[(t + ITERS_CAP) % (ITERS_CAP + 1)][y-1][x],");
        let _ = writeln!(s, "            field[(t + ITERS_CAP) % (ITERS_CAP + 1)][y+1][x],");
        let _ = writeln!(s, "            field[(t + ITERS_CAP) % (ITERS_CAP + 1)][y][x-1],");
        let _ = writeln!(s, "            field[(t + ITERS_CAP) % (ITERS_CAP + 1)][y][x+1],");
        let _ = writeln!(s, "            seed[y][x],");
        let _ = writeln!(s, "            t");
        let _ = writeln!(s, "        );");
        let _ = writeln!(s, "    }}}}");
        let _ = writeln!(s, "    for ay : 1 .. H-1 {{");
        let _ = writeln!(s, "    for ax : 1 .. W-1 {{");
        let _ = writeln!(s, "        partials[t] <-- max_abs_acc(");
        let _ = writeln!(s, "            partials[t],");
        let _ = writeln!(s, "            field[t][ay][ax],");
        let _ = writeln!(s, "            field[(t + ITERS_CAP) % (ITERS_CAP + 1)][ay][ax]");
        let _ = writeln!(s, "        );");
        let _ = writeln!(s, "    }}}}");
        let _ = writeln!(s, "    maxdiff[t] <-- ident(partials[t]);");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "for ry : 0 .. H {{");
        let _ = writeln!(s, "for rx : 0 .. W {{");
        let _ = writeln!(s, "    result[ry][rx] <-- ident(field[ITERS_CAP][ry][rx]);");
        let _ = writeln!(s, "}}}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "save_output(result);");
        s
    }

    fn sched_src(&self, seed: u64) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "// GENERATED by diff_fuzz (seed={seed}).");
        let _ = writeln!(
            s,
            "// single-worker (host); for..until break emit is pthreads-sync ONLY."
        );
        let _ = writeln!(s, "schedule for \"./prog.algo.nuc\" {{");
        let _ = writeln!(s, "    workers = {{ host }};");
        let _ = writeln!(s);
        let _ = writeln!(s, "    place load_input      on host;");
        let _ = writeln!(s, "    place save_output     on host;");
        let _ = writeln!(s, "    place jacobi5_or_seed on host;");
        let _ = writeln!(s, "    place max_abs_acc     on host;");
        let _ = writeln!(s, "    place ident           on host;");
        let _ = writeln!(s, "}}");
        s
    }

    fn kernels_src(&self, seed: u64) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "// GENERATED by diff_fuzz (seed={seed}).");
        let _ = writeln!(s, "use std::env;");
        let _ = writeln!(s, "use std::fs;");
        let _ = writeln!(s, "use std::io::Write;");
        let _ = writeln!(s);
        let _ = writeln!(s, "const H: usize = {};", self.h);
        let _ = writeln!(s, "const W: usize = {};", self.w);
        let _ = writeln!(s, "const N: usize = H * W;");
        let _ = writeln!(s);
        // Identical kernel bodies to the curated 21-jacobi (the proven,
        // overflow-safe forms). Single source of truth: these mirror the
        // reference `jacobi_step` / `abs_diff` / `maxdiff` above.
        let _ = writeln!(s, "fn abs_diff_i32(n: i32, o: i32) -> i32 {{");
        let _ = writeln!(s, "    let mag: u64 = (i64::from(n) - i64::from(o)).unsigned_abs();");
        let _ = writeln!(s, "    mag.min(i32::MAX as u64) as i32");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "pub fn jacobi5_or_seed(prev_n: i32, prev_s: i32, prev_e: i32, prev_w: i32, seed_yx: i32, t: i32) -> i32 {{"
        );
        let _ = writeln!(s, "    if t == 0 {{");
        let _ = writeln!(s, "        seed_yx");
        let _ = writeln!(s, "    }} else {{");
        let _ = writeln!(
            s,
            "        let sum = prev_n.wrapping_add(prev_s).wrapping_add(prev_e).wrapping_add(prev_w);"
        );
        let _ = writeln!(s, "        sum / 4");
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "pub fn max_abs_acc(acc: i32, n: i32, o: i32) -> i32 {{");
        let _ = writeln!(s, "    acc.max(abs_diff_i32(n, o))");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "pub fn ident(v: i32) -> i32 {{ v }}");
        let _ = writeln!(s);
        let _ = writeln!(s, "pub fn load_input() -> Vec<i32> {{");
        let _ = writeln!(
            s,
            "    let path = env::var(\"NUC_INPUT_PATH\").unwrap_or_else(|_| \"input.bin\".to_string());"
        );
        let _ = writeln!(s, "    let bytes = fs::read(&path).expect(\"read input\");");
        let _ = writeln!(s, "    assert!(bytes.len() >= N * 4);");
        let _ = writeln!(s, "    let mut out = Vec::with_capacity(N);");
        let _ = writeln!(s, "    for i in 0..N {{");
        let _ = writeln!(s, "        let off = i * 4;");
        let _ = writeln!(
            s,
            "        out.push(i32::from_le_bytes([bytes[off], bytes[off+1], bytes[off+2], bytes[off+3]]));"
        );
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "    out");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "pub fn save_output(data: Vec<i32>) {{");
        let _ = writeln!(s, "    assert_eq!(data.len(), N);");
        let _ = writeln!(
            s,
            "    let path = env::var(\"NUC_OUTPUT_PATH\").unwrap_or_else(|_| \"output.bin\".to_string());"
        );
        let _ = writeln!(s, "    let mut bytes = Vec::with_capacity(data.len() * 4);");
        let _ = writeln!(s, "    for v in &data {{ bytes.extend_from_slice(&v.to_le_bytes()); }}");
        let _ = writeln!(
            s,
            "    fs::File::create(&path).expect(\"create output\").write_all(&bytes).expect(\"write\");"
        );
        let _ = writeln!(s, "}}");
        s
    }

    pub(crate) fn bundle(&self, seed: u64) -> SourceBundle {
        SourceBundle {
            algo: self.algo_src(seed),
            sched: self.sched_src(seed),
            kernels: self.kernels_src(seed),
            input: self.input(),
            reference: self.reference(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn break_gen_is_strictly_inside_the_cap() {
        // Every generated instance must break BEFORE the cap (so the
        // early-exit branch — not the cap-hit branch — is exercised) and
        // AFTER gen 0 (gen 0 never converges: maxdiff[0] = max|seed - 0|).
        for s in 0..2000u64 {
            let mut r = Rng::new(s);
            // Only inspect instances; cheap (pure simulation, no build).
            let f = ForUntil1d::generate(&mut r);
            let (k, _) = f.simulate();
            assert!(k > 0, "seed {s}: broke at gen 0 (degenerate)");
            assert!(
                k < f.cap,
                "seed {s}: hit cap {} at gen {k} (no early exit)",
                f.cap
            );
        }
    }

    #[test]
    fn break_field_is_distinguishable_from_zero_cap_slice() {
        // The converged generation must be NON-ZERO somewhere (else it is
        // byte-identical to the unwritten zero cap slice and the break-
        // rewrite would be untestable). TOL=2 guarantees this.
        for s in 0..500u64 {
            let mut r = Rng::new(s);
            let f = ForUntil1d::generate(&mut r);
            let (_, field) = f.simulate();
            assert!(
                field.iter().any(|&v| v != 0),
                "seed {s}: converged field is all-zero (indistinguishable from cap slice)"
            );
        }
    }

    #[test]
    fn algo_uses_for_until_with_exact_predicate() {
        let mut r = Rng::new(1);
        let f = ForUntil1d::generate(&mut r);
        let a = f.algo_src(1);
        assert!(a.contains("until maxdiff[t] <= TOL"));
        assert!(a.contains("for t : 0 .. ITERS_CAP+1"));
        // single-worker schedule.
        assert!(f.sched_src(1).contains("workers = { host };"));
    }

    #[test]
    fn reference_matches_known_curated_shape_at_8x8() {
        // Sanity: an 8x8 grid of mid-range constant seed converges and is
        // non-zero at the break gen.
        let f = ForUntil1d {
            h: 8,
            w: 8,
            cap: 64,
            seed: vec![100; 64],
        };
        let (k, field) = f.simulate();
        assert!(k > 0 && k < 64);
        assert!(field.iter().any(|&v| v != 0));
    }
}
