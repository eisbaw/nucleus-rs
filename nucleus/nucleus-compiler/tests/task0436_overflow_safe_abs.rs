//! TASK-0436 AC#2 — overflow-safe absolute-difference pin for the
//! 21-jacobi-converge convergence reduction kernel.
//!
//! The S3 fixture kernel computes `|n - o|` as
//! `n.wrapping_sub(o).abs()`, which PANICS in debug and returns the
//! NEGATIVE `i32::MIN` in release when `n.wrapping_sub(o) == i32::MIN`
//! (reachable e.g. n=0, o=i32::MIN). That mis-ranks the `max` fold and
//! breaks the "|n-o| >= 0 ⇒ 0 is the max-identity" invariant the whole
//! reduction rests on (panic-not-diagnostic recurring defect).
//!
//! The real (21-jacobi-converge) kernel replaces that with the
//! overflow-safe `abs_diff_i32` (i64-widening + `unsigned_abs` +
//! `i32::MAX` clamp). This test `include!`s the EXACT shipped kernel
//! source and pins:
//!   - the extreme input (n=0, o=i32::MIN) yields a LARGE POSITIVE
//!     magnitude — NO panic, NO negative value;
//!   - `max_abs_acc` folds it correctly (the accumulator never goes
//!     negative, so the zero-init max-identity holds).
//!
//! It lives in the nucleus-compiler test crate so it runs under BOTH
//! `just test` (dev) AND `just test-release` (release) — the release arm
//! is load-bearing here: the S3 kernel's bug is INVISIBLE in dev for the
//! `i32::MIN` path only via panic, but in release it silently returns a
//! negative value, so the overflow-safety must be proven in BOTH
//! profiles (TASK-0291 release-profile-blindness lesson).
//!
//! `include!` (textual) makes the kernel's private `fn abs_diff_i32`
//! visible to this test module. The unused effectful I/O kernels
//! (`load_input` / `save_output`) are pulled in too but never called;
//! `#![allow(dead_code)]` on the included scope is unnecessary because
//! they are `pub`.

// Pull in the EXACT shipped kernel bodies (single source of truth — no
// re-implementation that could drift from what 21-jacobi-converge runs).
include!("../../../nuc-nucleus/examples/21-jacobi-converge/kernels.rs");

#[test]
fn abs_diff_i32_extreme_inputs_no_panic_no_negative() {
    // n=0, o=i32::MIN: the true |0 - i32::MIN| = 2147483648 exceeds
    // i32::MAX, so the clamp policy returns i32::MAX. The point: a LARGE
    // POSITIVE value, never a panic, never a negative.
    let d = abs_diff_i32(0, i32::MIN);
    assert!(d > 0, "abs-diff of extreme inputs must be positive, got {d}");
    assert_eq!(
        d,
        i32::MAX,
        "the overflow-safe clamp must saturate to i32::MAX for an \
         out-of-i32-range magnitude (true magnitude 2147483648)"
    );

    // The symmetric case and i32::MIN vs i32::MAX (the maximal spread).
    assert!(abs_diff_i32(i32::MIN, 0) > 0);
    assert_eq!(abs_diff_i32(i32::MIN, i32::MAX), i32::MAX);
    assert_eq!(abs_diff_i32(i32::MAX, i32::MIN), i32::MAX);

    // In-range values: the magnitude is exact (no clamp).
    assert_eq!(abs_diff_i32(10, 3), 7);
    assert_eq!(abs_diff_i32(3, 10), 7);
    assert_eq!(abs_diff_i32(-5, 5), 10);
    assert_eq!(abs_diff_i32(0, 0), 0);
}

#[test]
fn max_abs_acc_never_goes_negative_on_extreme_inputs() {
    // The accumulator self-read fold `max(acc, |n - o|)` must stay
    // non-negative even when the per-element diff is the extreme case —
    // otherwise the zero-init max-identity (max(0, term) == term for
    // term >= 0) would be violated (a negative term would WIN the max
    // and corrupt the reduction; the exact S3-kernel bug this hardens).
    let folded = max_abs_acc(0, 0, i32::MIN);
    assert!(
        folded >= 0,
        "max_abs_acc must never fold to a negative value (would break the \
         zero-init max-identity); got {folded}"
    );
    assert_eq!(folded, i32::MAX);

    // Folding a smaller term into a larger accumulator keeps the larger.
    assert_eq!(max_abs_acc(100, 5, 2), 100);
    // Folding a larger term raises the accumulator.
    assert_eq!(max_abs_acc(3, 0, 100), 100);
}
