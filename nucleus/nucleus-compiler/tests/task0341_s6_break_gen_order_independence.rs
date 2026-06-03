//! TASK-0341.02.01.07 (grammar-epic S6) AC#1 + AC#2 — pin that the
//! `for..until` CONVERGENCE BREAK-GENERATION (the runtime `k` at which
//! `maxdiff[k] <= tol` first holds) is SCHEDULE-ORDER-INDEPENDENT for the
//! max-abs-diff (L-infinity) norm, and therefore that S6 is a NO-OP — no
//! integer fixed-point / fixed reduction-tree machinery is needed.
//!
//! ## What this adds over the S3 pin
//!
//! `task0341_s3_order_independence.rs` already pins that the SCALAR
//! `maxdiff` of a SINGLE generation pair is fold-order-independent (max
//! is associative + commutative + exact on i32). The break-generation
//! `k` = `first t such that maxdiff[t] <= tol` being order-free is a
//! COROLLARY of that scalar pin — no new algebra: if each per-generation
//! `maxdiff[t]` is order-free, the whole sequence is, so its first
//! tol-crossing is too. This test's added value is therefore narrower
//! than "a new property": it pins that corollary (a) directly at the
//! ACTUAL OBSERVABLE the backend emits (`if maxdiff[t] <= tol { break }`)
//! against fixture/seed drift, and (b) folding with the POST-TASK-0436
//! overflow-safe abs the shipped example really runs (S3 used the
//! pre-0436 `wrapping_sub().abs()` spelling). It is a regression guard on
//! the convergence observable, not a fresh determinism claim.
//!
//! ## Overflow-safe abs — pins the ACTUALLY-SHIPPED reduction
//!
//! Unlike the S3 algebra test (which used the pre-TASK-0436
//! `wrapping_sub().abs()` spelling, sound only on its bounded fixture),
//! this test folds with `abs_diff_i32` byte-identical to the shipped
//! `nuc-nucleus/examples/21-jacobi-converge/kernels.rs` (i64-widening +
//! `unsigned_abs` + `i32::MAX` clamp, TASK-0436). So the determinism pin
//! reflects the reduction the 21-jacobi-converge cell really runs, not a
//! stand-in that could drift from it.
//!
//! ## Scope
//!
//! Pure reduction algebra — no nucleus/codegen dependency. The full
//! cross-SCHEDULE / cross-backend differential for the COLLECTIVE
//! (multi-worker all-reduce) break is S7 (TASK-0341.02.01.08); the
//! max-of-maxes there is still order-free (this is exactly why S7 needs
//! no fixed-point tree either), but running it across real backends is
//! that slice's scope. Here we pin the single-run order-independence that
//! makes the convergence test deterministic in the first place.

/// Overflow-safe absolute difference, byte-identical to `abs_diff_i32`
/// in `21-jacobi-converge/kernels.rs` (TASK-0436). i64-widening means the
/// subtraction never overflows and `unsigned_abs` has no `i32::MIN.abs()`
/// panic path; the result is clamped to `i32::MAX`.
fn abs_diff_i32(n: i32, o: i32) -> i32 {
    let mag: u64 = (i64::from(n) - i64::from(o)).unsigned_abs();
    mag.min(i32::MAX as u64) as i32
}

/// Fold the per-element abs-diffs of one generation pair into the scalar
/// `maxdiff` under an explicit element `order`. Init 0 is the max-identity
/// (every abs-diff is non-negative).
fn maxdiff_in_order(cur: &[i32], prev: &[i32], order: &[usize]) -> i32 {
    let mut acc = 0i32;
    for &idx in order {
        acc = acc.max(abs_diff_i32(cur[idx], prev[idx]));
    }
    acc
}

/// Depth-balanced binary tree fold (the shape `prog.algo.nuc`'s two-phase
/// partition+tree-combine reduction actually emits) — a DIFFERENT
/// association than the flat left-to-right fold.
fn maxdiff_tree(cur: &[i32], prev: &[i32]) -> i32 {
    let mut level: Vec<i32> = (0..cur.len())
        .map(|i| abs_diff_i32(cur[i], prev[i]))
        .collect();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut chunks = level.chunks_exact(2);
        for c in &mut chunks {
            next.push(c[0].max(c[1]));
        }
        if let [last] = chunks.remainder() {
            next.push(*last);
        }
        level = next;
    }
    level.first().copied().unwrap_or(0)
}

/// A synthetic multi-generation convergence run: `gen[t][i]` halves each
/// generation toward the all-zero fixed point, so consecutive-generation
/// `maxdiff[t]` is strictly decreasing and crosses any small `tol` at a
/// well-defined generation. `T_CAP + 1` slices, `N` interior elements.
const N: usize = 32;
const T_CAP: usize = 16;

fn generations() -> Vec<Vec<i32>> {
    // gen[0][i] is a varied mixed-sign seed; each later generation halves
    // toward 0 (round-to-zero integer shift), so maxdiff shrinks ~2x per
    // step and eventually reaches 0 (the integer fixed point).
    let seed: Vec<i32> = (0..N).map(|i| ((i as i32) * 7) % 1000 - 500).collect();
    let mut gens = Vec::with_capacity(T_CAP + 1);
    gens.push(seed);
    for t in 1..=T_CAP {
        let prev = &gens[t - 1];
        gens.push(prev.iter().map(|&v| v / 2).collect());
    }
    gens
}

/// The break-generation under one fold order: the first `t >= 1` whose
/// `maxdiff[t]` (cur=gen[t], prev=gen[t-1]) is `<= tol`. Returns
/// `T_CAP` (the cap-hit sentinel resolution) if it never converges —
/// mirrors the backend's `__nuc_final_gen` semantics.
fn break_gen<F: Fn(&[i32], &[i32]) -> i32>(gens: &[Vec<i32>], tol: i32, maxdiff: F) -> usize {
    for t in 1..=T_CAP {
        if maxdiff(&gens[t], &gens[t - 1]) <= tol {
            return t;
        }
    }
    T_CAP
}

#[test]
fn break_generation_is_fold_order_independent() {
    let gens = generations();
    let tol = 2i32;

    // Reference: a plain direct max over per-element abs-diffs, in index
    // order — the order-free "ground truth" the break-gen must match.
    let reference_break = break_gen(&gens, tol, |cur, prev| {
        (0..cur.len())
            .map(|i| abs_diff_i32(cur[i], prev[i]))
            .max()
            .unwrap_or(0)
    });
    // Pin the concrete value so a fixture/seed drift fails loud rather
    // than silently re-baselining (the convergence lands at a non-zero,
    // non-cap generation — the interesting case, not a trivial t=1 or
    // a cap-hit).
    assert_eq!(
        reference_break, 8,
        "fixture break-generation drifted (seed/tol change?)"
    );
    assert!(
        (1..T_CAP).contains(&reference_break),
        "the pin must exercise a genuine early-exit (1 <= k < cap), not \
         t=1 and not a cap-hit"
    );

    let n = N;

    // 1. Sequential element order.
    let seq: Vec<usize> = (0..n).collect();
    assert_eq!(
        break_gen(&gens, tol, |c, p| maxdiff_in_order(c, p, &seq)),
        reference_break,
        "sequential fold order"
    );

    // 2. Reversed element order.
    let rev: Vec<usize> = (0..n).rev().collect();
    assert_eq!(
        break_gen(&gens, tol, |c, p| maxdiff_in_order(c, p, &rev)),
        reference_break,
        "reversed fold order"
    );

    // 3. Several pseudo-random permutations (xorshift; no Math.random
    //    dependence — deterministic per seed).
    for seed in [0x1234_5678u64, 0x9e37_79b9, 0xdead_beef, 0x0badf00d, 0x5555_aaaa] {
        let mut order: Vec<usize> = (0..n).collect();
        let mut s = seed;
        // Fisher-Yates with a xorshift64 stream.
        for i in (1..n).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let j = (s % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        assert_eq!(
            break_gen(&gens, tol, |c, p| maxdiff_in_order(c, p, &order)),
            reference_break,
            "permuted fold order (seed {seed:#x})"
        );
    }

    // 4. The depth-balanced tree association (the two-phase
    //    partition+combine shape the prog.algo.nuc reduction emits).
    assert_eq!(
        break_gen(&gens, tol, maxdiff_tree),
        reference_break,
        "balanced-tree fold association"
    );

    // 5. The CAP-HIT sentinel path is also order-invariant. With an
    //    impossible `tol = -1` (every `maxdiff >= 0`) NO generation
    //    converges, so `break_gen` returns the `T_CAP` sentinel (the
    //    backend's `__nuc_final_gen = cap` cap-hit resolution, .05.03)
    //    under EVERY fold order — exercising the non-convergence branch
    //    that the converging fixture above never reaches.
    let cap_hit_seq = break_gen(&gens, -1, |c, p| maxdiff_in_order(c, p, &seq));
    assert_eq!(cap_hit_seq, T_CAP, "tol=-1 must hit the cap (no convergence)");
    assert_eq!(
        break_gen(&gens, -1, |c, p| maxdiff_in_order(c, p, &rev)),
        cap_hit_seq,
        "cap-hit sentinel is order-invariant (reversed)"
    );
    assert_eq!(
        break_gen(&gens, -1, maxdiff_tree),
        cap_hit_seq,
        "cap-hit sentinel is order-invariant (tree)"
    );
}

/// AC#1: characterise that SUM/L1 would be the order-sensitive case (the
/// reason S6 contemplated a fixed-point tree) — documented as a contrast,
/// and pinned for INTEGER add (which is still order-free; only a FLOAT
/// sum is not, and the epic is integer-only per PRD §10.1). This guards
/// the claim "max is the norm that makes S6 a no-op" against a future
/// edit that switches the norm without reopening S6.
#[test]
fn integer_sum_l1_is_also_order_free_but_is_not_the_chosen_norm() {
    let gens = generations();
    // L1 (sum of abs-diffs) of the first generation step, two orders.
    let cur = &gens[1];
    let prev = &gens[0];
    let l1_seq: i64 = (0..N).map(|i| i64::from(abs_diff_i32(cur[i], prev[i]))).sum();
    let l1_rev: i64 = (0..N)
        .rev()
        .map(|i| i64::from(abs_diff_i32(cur[i], prev[i])))
        .sum();
    assert_eq!(
        l1_seq, l1_rev,
        "integer L1 sum is order-free too (associative+commutative); it is \
         the FLOAT sum that is not — which is why the epic stays integer + \
         max, and why S6 needs no fixed-point machinery"
    );
}
