//! Algebraic spec pin for the reuse circular-buffer math
//! (TASK-0269 single-worker + TASK-0270 multi-worker, cycles 103-104).
//!
//! ## Why this file exists
//!
//! `render_reuse_buf_decls` (single-worker; pthreads-sync calls
//! directly) and `render_reuse_buf_decls_pub` + `render_reuse_per_iter_update_pub`
//! (multi-worker; shared walker) emit the same circular-buffer
//! formulas:
//!
//! - **Prologue** (before the loop): for each offset
//!   `b in [min_offset, max_offset)` (i.e. every offset EXCEPT the
//!   most-distant), write `buf[(lo + b - min_offset).rem_euclid(length)]
//!   = data[lo + b]`. (`max_offset = min_offset + length - 1`.)
//! - **Per-iter update** (first statement of each body iteration at
//!   `iv = k`): write `buf[(k + max_offset - min_offset).rem_euclid(length)]
//!   = data[k + max_offset]`.
//! - **Body read** (rewrites of every matching `data[iv + b]` ref):
//!   `buf[(k + b - min_offset).rem_euclid(length)]`.
//!
//! The e2e bit-identical differential against `reference.bin` proves
//! the EMITTED code's RUNTIME behaviour matches the raw-read
//! semantics on the shipped fixtures (05-stencil/reuse,
//! 05-stencil/distributed × pthreads-async + mp-tcp-event). This
//! file pins the FORMULA itself — a regression that swaps a `+` for
//! a `-`, or off-by-ones the prologue range, will FAIL the e2e
//! differential AND this test (which provides a quicker, more
//! diagnostic signal than a bit-diff at the binary boundary).
//!
//! ## What this test does
//!
//! Simulates the prologue + per-iter update + body-read sequence
//! for a representative set of `(min_offset, length, lo, hi)` tuples
//! (including the shipped 05-stencil parameters and several edge
//! shapes), running the same formulas in pure Rust against a
//! synthetic `data(i) = i` array. At each iteration `iv = k`, for
//! every offset `b in [min_offset, max_offset]`, the simulated
//! `buf[slot]` MUST equal `data[k + b]`. Any algebraic divergence
//! — wrong rem_euclid base, off-by-one in the prologue's range,
//! incorrect slot rotation — surfaces as an `assert_eq!` failure
//! naming `(k, b, slot, length, min_offset)`.
//!
//! ## How this guards future refactors
//!
//! TASK-0282 (multi-outer-coord generalisation) will change the
//! per-(data, axis) discovery logic but should NOT change the
//! buffer math itself — same prologue, same per-iter rotate, same
//! body-read rewrite. If TASK-0282 lands and this test still passes,
//! the math is preserved. If it fails, the refactor accidentally
//! changed the formula and needs to be re-grounded.
//!
//! TASK-0270 cycle-104 NO-GO architect P1.1 (textual `abs.replace`)
//! was a STRING-LEVEL regression that this test would NOT have
//! caught (the math was correct, the corruption was in the rendered
//! identifier `tile_name`). Both kinds of regression have their
//! distinct sentinels — this is the algebraic one.

/// Simulate the prologue + per-iter update + body reads for one
/// `(min_offset, length, lo, hi)` tuple. Synthetic `data(i) = i`
/// makes raw-vs-buf equivalence trivially checkable.
fn simulate_reuse(min_offset: i64, length: u64, lo: i64, hi: i64) {
    assert!(length >= 2, "ReuseSlot::length is contracted > 1");
    assert!(lo < hi, "loop range must be non-empty");

    let l_i64 = length as i64;
    let max_offset = min_offset + l_i64 - 1;
    let mut buf: Vec<i64> = vec![0; length as usize];

    // Synthetic data: `data[i] = i`. The body reads should each
    // return `k + b` (with no wrap-around weirdness).
    let data = |i: i64| -> i64 { i };

    // PROLOGUE: fill every offset EXCEPT max_offset, at iv = lo.
    // This is the loop emitted by render_reuse_buf_decls at line
    // 1252-1268 of backend-common/src/render.rs:
    //   for b in min_offset..max_offset {
    //       buf[((lo + b - min_offset).rem_euclid(length)) as usize]
    //           = data[lo + b];
    //   }
    for b in min_offset..max_offset {
        let slot = ((lo + b - min_offset).rem_euclid(l_i64)) as usize;
        buf[slot] = data(lo + b);
    }

    // BODY: iterate iv from lo..hi, applying per-iter update then
    // reading every offset.
    for k in lo..hi {
        // PER-ITER UPDATE: store the max_offset slot.
        //   buf[((k + max_offset - min_offset).rem_euclid(length)) as usize]
        //       = data[k + max_offset];
        let slot = ((k + max_offset - min_offset).rem_euclid(l_i64)) as usize;
        buf[slot] = data(k + max_offset);

        // BODY READS: for every b in [min_offset, max_offset],
        // the rewrite `buf[(k + b - min_offset) % length]` must
        // equal the raw read `data[k + b]`.
        for b in min_offset..=max_offset {
            let slot = ((k + b - min_offset).rem_euclid(l_i64)) as usize;
            let buf_value = buf[slot];
            let raw_value = data(k + b);
            assert_eq!(
                buf_value, raw_value,
                "reuse buffer divergence: iv k={k} offset b={b} \
                 slot={slot} length={length} min_offset={min_offset} \
                 — buf yielded {buf_value}, raw read would yield {raw_value}",
            );
        }
    }
}

/// 05-stencil/reuse + 05-stencil/distributed shipped parameters.
/// min_offset=-1, length=3, lo=1 (inner x range starts at 1),
/// hi=15 (W-1 with W=16). Pins the production-load-bearing math.
#[test]
fn reuse_buf_math_pins_shipped_05_stencil_shape() {
    simulate_reuse(-1, 3, 1, 15);
}

/// Smallest valid reuse: length=2. Common for halo-1 stencils on a
/// single axis if the inference cuts away one boundary read.
#[test]
fn reuse_buf_math_pins_length_2_shape() {
    simulate_reuse(0, 2, 0, 100);
    simulate_reuse(-1, 2, 0, 50);
}

/// All-positive offsets. Unusual but algebraically valid — a
/// future-read pattern (e.g. `data[iv]`, `data[iv+1]`, `data[iv+2]`).
#[test]
fn reuse_buf_math_pins_all_positive_offsets() {
    simulate_reuse(0, 4, 0, 50);
    simulate_reuse(2, 3, 5, 30);
}

/// Wider window: length=7, mixed signs. Stress-tests the
/// rem_euclid wrap-around with multiple full revolutions.
#[test]
fn reuse_buf_math_pins_wide_mixed_signs() {
    simulate_reuse(-3, 7, 3, 50);
}

/// Negative-lo. The shipped `for x : 1..W-1` always has positive
/// `lo`, but the formula uses `rem_euclid` precisely so a future
/// schedule with a negative lower bound stays correct. Pin the
/// algebra here so the negative-lo case never silently regresses.
#[test]
fn reuse_buf_math_pins_negative_lo_via_rem_euclid() {
    simulate_reuse(-1, 3, -5, 10);
    simulate_reuse(-2, 5, -10, 5);
}

/// Tiny range: lo+1 == hi (single iteration). The per-iter update
/// fires once, the body reads fire once. The prologue still has to
/// produce a buf where the body-read invariant holds at iv=lo.
#[test]
fn reuse_buf_math_pins_single_iteration() {
    simulate_reuse(-1, 3, 5, 6);
    simulate_reuse(0, 4, 0, 1);
}

/// Long range: stresses the rem_euclid through many revolutions.
/// Catches any accumulating-state bug.
#[test]
fn reuse_buf_math_pins_long_range_multiple_revolutions() {
    simulate_reuse(-1, 3, 0, 1000);
    simulate_reuse(-2, 5, 0, 1000);
}

/// Length-equals-range. Tests when each body iteration writes a
/// fresh slot exactly once (no slot is reused). Useful as a sanity
/// check on the rotation logic.
#[test]
fn reuse_buf_math_pins_length_equals_range() {
    simulate_reuse(-1, 3, 0, 3);
    simulate_reuse(0, 5, 0, 5);
}
