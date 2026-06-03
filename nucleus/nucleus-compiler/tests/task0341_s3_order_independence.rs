//! TASK-0341.02.01.04 (grammar-epic S3) AC#2 — pin that the
//! max-abs-diff (L-infinity) reduction is SCHEDULE-ORDER-INDEPENDENT,
//! and therefore that S6 (TASK-0341.02.01.07, deterministic
//! convergence reduction) is a NO-OP for the max norm.
//!
//! ## Why a pure-algebra unit test (no nucleus dependency)
//!
//! The order-independence claim is a property of the REDUCTION ALGEBRA
//! (`max` over per-element non-negative `abs(new - old)`), not of any
//! particular codegen. `max` is associative, commutative, and exact on
//! i32 (and even in IEEE-754 FP — no rounding), so ANY fold order over
//! the per-element abs-diffs yields the IDENTICAL scalar. This test
//! pins exactly that: it folds the same multiset of element-pairs under
//! many distinct orders (sequential, reversed, several pseudo-random
//! permutations, and the depth-2 tree shape the fixture's prog.algo.nuc
//! actually emits) and asserts the scalar is invariant.
//!
//! This is the load-bearing fact for the epic: because the norm is
//! `max`, the convergence break-condition `maxdiff <= tol` is
//! deterministic across ANY schedule (single-worker, pipelined, or the
//! multi-worker collective all-reduce of S7) WITHOUT the integer
//! fixed-point / fixed reduction-tree machinery that a sum/L1 norm
//! would demand. Hence S6 is pinned as a no-op for norm = max.
//!
//! Contrast (documented, NOT asserted here): a SUM/L1 reduction over
//! the same element-pairs is order-free for INTEGER `wrapping_add` too
//! (integer add is associative+commutative), but a FLOAT sum is NOT —
//! which is exactly why 03-reduction restricts to integers and why a
//! float L1 norm would need S6. The max norm sidesteps the FP concern
//! entirely; that is the determinism simplification the epic's
//! architect P3 called for.
//!
//! The full cross-schedule / cross-backend differential for the
//! collective break is S7 (TASK-0341.02.01.08); this test's scope is
//! the order-independence of the reduction ITSELF.

/// The per-element fold step, byte-identical to `max_abs_acc` in the
/// fixture's kernels.rs: `max(acc, abs(n - o))`.
fn max_abs_acc(acc: i32, n: i32, o: i32) -> i32 {
    acc.max(n.wrapping_sub(o).abs())
}

/// The binary tree-combine step, byte-identical to `max_combine`.
fn max_combine(a: i32, b: i32) -> i32 {
    a.max(b)
}

/// Deterministic, varied, mixed-sign generation pair. Mirrors the
/// committed fixture's input pattern (`cruft/spike_s3_ref.py`):
/// new[i] = (i*7)%1000 - 500, old[i] = (i*13)%1000 - 500. N = 32.
fn generation_pair() -> (Vec<i32>, Vec<i32>) {
    let n = 32usize;
    let new: Vec<i32> = (0..n).map(|i| ((i as i32) * 7) % 1000 - 500).collect();
    let old: Vec<i32> = (0..n).map(|i| ((i as i32) * 13) % 1000 - 500).collect();
    (new, old)
}

/// Sequential left-to-right fold (init 0 — the max-identity, valid
/// because every abs-diff is non-negative).
fn fold_in_order(new: &[i32], old: &[i32], order: &[usize]) -> i32 {
    let mut acc = 0i32;
    for &idx in order {
        acc = max_abs_acc(acc, new[idx], old[idx]);
    }
    acc
}

#[test]
fn max_abs_diff_is_fold_order_independent() {
    let (new, old) = generation_pair();
    let n = new.len();

    // The reference scalar: a plain direct max over per-element
    // abs-diffs. This is the order-free "ground truth".
    let reference: i32 = (0..n)
        .map(|i| new[i].wrapping_sub(old[i]).abs())
        .max()
        .expect("non-empty");
    assert_eq!(reference, 186, "fixture reference scalar drifted");

    // 1. Sequential order.
    let seq: Vec<usize> = (0..n).collect();
    assert_eq!(fold_in_order(&new, &old, &seq), reference, "sequential");

    // 2. Reversed order.
    let rev: Vec<usize> = (0..n).rev().collect();
    assert_eq!(fold_in_order(&new, &old, &rev), reference, "reversed");

    // 3. Several deterministic pseudo-random permutations (a tiny LCG
    //    Fisher-Yates so the test has no external dependency and is
    //    itself reproducible).
    for seed in [1u64, 2, 3, 12345, 0xDEADBEEF] {
        let mut perm: Vec<usize> = (0..n).collect();
        let mut state = seed;
        for i in (1..n).rev() {
            // xorshift64 step for a deterministic stream.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = (state as usize) % (i + 1);
            perm.swap(i, j);
        }
        assert_eq!(
            fold_in_order(&new, &old, &perm),
            reference,
            "permutation seed={seed} must yield the identical scalar — max is \
             associative+commutative so the fold order is irrelevant"
        );
    }
}

#[test]
fn tree_combine_matches_sequential_fold() {
    // The fixture's prog.algo.nuc does NOT fold all 32 elements in one
    // sequence — it does a TWO-PHASE reduction: 4 per-partition
    // sequential folds, then a depth-2 tree-combine of the 4 partials.
    // This pins that the two-phase shape yields the same scalar as a
    // flat fold (i.e. the partition + tree structure is order-free).
    let (new, old) = generation_pair();
    // NUM_WORKERS = 4, PARTITION_SIZE = N / NUM_WORKERS = 32 / 4 = 8
    // (mirrors prog.algo.nuc). The `partials` array length below pins
    // NUM_WORKERS structurally.
    let partition_size = 8usize;

    // Phase 1: per-partition fold (init 0 = max-identity).
    let mut partials = [0i32; 4];
    for (w, partial) in partials.iter_mut().enumerate() {
        for i in 0..partition_size {
            let idx = w * partition_size + i;
            *partial = max_abs_acc(*partial, new[idx], old[idx]);
        }
    }
    // Pin the per-partition partials (mirrors cruft/spike_s3_ref.py).
    assert_eq!(partials, [42, 90, 138, 186], "per-partition partials");

    // Phase 2: depth-2 tree-combine, exactly as prog.algo.nuc emits.
    let half1 = max_combine(partials[0], partials[1]);
    let half2 = max_combine(partials[2], partials[3]);
    let maxdiff = max_combine(half1, half2);

    // Flat sequential fold over all 32 elements.
    let flat = fold_in_order(&new, &old, &(0..32).collect::<Vec<_>>());

    assert_eq!(maxdiff, flat, "two-phase tree == flat fold");
    assert_eq!(maxdiff, 186, "scalar max-abs-diff");
}
