//! Shared helpers for the affine-stride family of passes (halo / reuse /
//! future bounds-checking elimination, etc.).
//!
//! Lifted out of [`crate::passes::halo_inference`] in cycle 82 (TASK-0261
//! prerequisite) so the reuse-inference pass and the halo-inference pass
//! share ONE definition of "is this index expression affine in this
//! iter-var?". The pre-cycle-82 organisation had the helpers as private
//! items of `halo_inference.rs`; the cycle-81 review explicitly flagged
//! this as a forward-carry — "promote `affine_decompose` to `pub(crate)`
//! and lift to `passes::common::` BEFORE TASK-0261 lands. Both halo and
//! reuse share the prerequisite."
//!
//! ## Scope
//!
//! Three helpers are exposed:
//!
//! - [`affine_decompose`]: try to decompose an [`IrExpr`] as
//!   `coefficient * iv + offset`, returning `Some((coeff, offset))` on
//!   success.
//! - [`eval_const_int`]: try to evaluate an [`IrExpr`] as a pure integer
//!   constant (no iter-var, no DataRef, no Call), const-folding through
//!   the algorithm's `consts` table.
//! - [`expr_mentions`]: a syntactic predicate — does the expression
//!   contain an `Ident(iv)` anywhere in its tree?
//!
//! None of these mutate. All operate on borrowed [`IrExpr`] + a
//! borrowed `BTreeMap<String, ResolvedConst>`. The helpers are
//! `pub(crate)` so out-of-crate callers cannot bypass the pass-level
//! validation that wraps them; the in-crate callers (halo + reuse
//! passes) are the only legitimate users.
//!
//! ## Semantic contract (load-bearing — do not change without bumping
//! every caller's tests)
//!
//! `affine_decompose(e, iv, consts)` returns `Some((coeff, offset))` iff
//! `e` evaluates, for every legal value of `iv`, to exactly
//! `coeff * iv + offset` — where both `coeff` and `offset` are i64
//! const-foldable through `consts`. The shapes recognised are:
//!
//! - `iv`                        → `(1, 0)`
//! - `-iv`                       → `(-1, 0)`
//! - `iv + c` / `c + iv`         → `(1, c)`
//! - `iv - c`                    → `(1, -c)`
//! - `c - iv`                    → `(-1, c)`
//! - `c * iv` / `iv * c`         → `(c, 0)`
//! - `c * iv + d` / `d + c * iv` → `(c, d)`
//! - `c * iv - d` / `d - c * iv` → `(c, -d)` / `(-c, d)`
//! - Pure-constant expressions   → `(0, k)` (no iv mentioned)
//!
//! Any deeper composition (`(a + b) * iv`, `iv + iv`, `iv * iv`, `iv / 2`,
//! `iv % 4`, etc.) returns `None`. The caller is expected to convert
//! `None` into a typed pass-specific error rather than guessing.
//!
//! ## Why this isn't `algo::lower::eval_const`
//!
//! `algo::lower::eval_const` evaluates a fully-resolvable expression at
//! const-folding time (it does NOT know about an iv being symbolic).
//! `eval_const_int` is the iv-aware subset — same arithmetic, but
//! treats Idents as iv when the iv name matches and as const-lookup
//! when not. Keeping it local + small means the helper has zero
//! upstream coupling and the affine pass family does not pay for
//! features it doesn't use (overflow semantics, ResolvedConst
//! materialisation, etc.).
//!
//! ## Why three helpers and not one
//!
//! The three helpers are mutually recursive (`affine_decompose` calls
//! `eval_const_int` and `expr_mentions` to decide which branch to take).
//! Collapsing them into one function would be an awkward inline match
//! pyramid; the public surface is clean as three named operations.

use std::collections::BTreeMap;

use crate::algo::{IrBinOp, IrExpr, ResolvedConst};

// --------------------------------------------------------------------
// Partition-band arithmetic (TASK-0262)
// --------------------------------------------------------------------

/// Per-worker iteration band shape returned by
/// [`compute_partition_bands`].
///
/// Each entry is `(lo, hi)` of the worker's half-open iteration slice.
/// The vector is ordered by worker index (`bands[i]` = the i-th
/// worker), and the bands' union covers the source `range` exactly once
/// (no gap, no overlap) — that is the partition contract.
///
/// `Range<i64>` would be the natural element type, but `Range` is not
/// `Copy` and the bands are commonly consumed multiple times by the
/// caller (write to sidecar AND emit a diagnostic). `(i64, i64)` is
/// `Copy` and ergonomic; the caller materialises a `Range` when needed.
pub(crate) type PartitionBand = (i64, i64);

/// Errors produced by [`compute_partition_bands`].
///
/// Distinct from the per-pass `PartitionError` / `PartitionRowsError` /
/// `PartitionBlocks2dError` enums because the partition-band math is a
/// shared helper across THREE callers; each caller wraps these into its
/// own typed variant for the pass-level diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartitionBandError {
    /// `(hi - lo) < n_workers` — the source range has fewer rows than
    /// workers. Even the floor-with-spillover policy (TASK-0262) cannot
    /// give every worker at least one row in this case, so it is a
    /// genuinely-undecidable category error: refuse compile.
    ///
    /// Distinct from "non-divisible": non-divisible (`len > N`,
    /// `len % N != 0`) is now ACCEPTED with the spillover policy; the
    /// only category left to reject is "not enough work to go around".
    InsufficientWork {
        len: i64,
        workers: usize,
    },
    /// `n_workers == 0`. A no-op input — the caller should pre-check
    /// the worker set is non-empty and surface a pass-specific
    /// `NoMultiWorkerBody` error before reaching here. Defensive guard
    /// only.
    ZeroWorkers,
    /// `hi < lo`. The source range is degenerate (empty or inverted).
    /// All in-tree examples produce `hi >= lo` from the link step's
    /// `eval_const`, so this is a defensive guard against a malformed
    /// synthetic ACFG; not reachable from valid user input.
    InvalidRange { lo: i64, hi: i64 },
}

/// Compute per-worker partition bands using the numpy.array_split-style
/// **floor-with-spillover** policy (TASK-0262 decision option (b)).
///
/// For a source range of length `L = hi - lo` across `N` workers:
///
/// - Each of the first `(L % N)` workers gets `(L / N) + 1` rows
///   (the "spillover" workers, one extra row each).
/// - The remaining workers each get `(L / N)` rows.
///
/// The returned vector has exactly `N` entries; element `i` is the i-th
/// worker's `(lo_i, hi_i)` half-open band. Bands are contiguous and
/// non-overlapping; their union is `[lo, hi)` exactly.
///
/// ### Why option (b) over option (a) "last-worker-gets-remainder"
///
/// (a) leaves N-1 workers idle at the maximum imbalance (e.g. 14 rows
/// across 4 workers under (a): 3,3,3,5 — the last worker carries 67%
/// more work than its peers). (b) gives 4,4,3,3 — maximum imbalance is
/// one row, irrespective of `N` or `L`. The wall-clock-bound axis is
/// the slowest worker, so minimising the per-worker max is the right
/// objective.
///
/// ### Why option (b) over option (c) "trailing-partial-tile"
///
/// (c) would introduce a synthetic "remainder repeat" on the last
/// worker, mirroring the block_transform full + partial split
/// (TASK-0142). That has the same imbalance as (a) (the partial worker
/// carries the residue) AND adds a structural variant for downstream
/// passes to recognise — a maintenance tax with no win.
///
/// ### What about the leftover-zero-band corner
///
/// `L % N == 0` reduces to the pre-TASK-0262 exact-split behaviour:
/// every worker gets `L / N` rows, no spillover. Equivalent in output
/// to the old policy for every divisible cell (13-cnn-inference
/// batch_parallel exact-split is unchanged).
///
/// ### Determinism
///
/// The function is a pure deterministic function of `(lo, hi,
/// n_workers)`. No hashing, no floating-point arithmetic; integer
/// division by definition. Same input ⇒ byte-identical output across
/// runs.
///
/// ### Failure modes
///
/// Returns [`PartitionBandError::InsufficientWork`] when `L < N` —
/// even spillover cannot give every worker one row, so this stays a
/// hard reject. Callers should wrap as a pass-specific typed error
/// (e.g. `PartitionError::InsufficientWork`).
pub(crate) fn compute_partition_bands(
    lo: i64,
    hi: i64,
    n_workers: usize,
) -> Result<Vec<PartitionBand>, PartitionBandError> {
    if n_workers == 0 {
        return Err(PartitionBandError::ZeroWorkers);
    }
    if hi < lo {
        return Err(PartitionBandError::InvalidRange { lo, hi });
    }
    let len = hi - lo;
    let n = n_workers as i64;
    if len < n {
        return Err(PartitionBandError::InsufficientWork {
            len,
            workers: n_workers,
        });
    }
    let base = len / n;
    let extras = len % n; // first `extras` workers get base+1
    let mut bands: Vec<PartitionBand> = Vec::with_capacity(n_workers);
    let mut cursor = lo;
    for i in 0..n_workers {
        let width = if (i as i64) < extras { base + 1 } else { base };
        let band_lo = cursor;
        let band_hi = cursor + width;
        bands.push((band_lo, band_hi));
        cursor = band_hi;
    }
    debug_assert_eq!(
        cursor, hi,
        "partition bands must cover [{lo}, {hi}) exactly; cursor={cursor}",
    );
    Ok(bands)
}

/// Try to decompose `e` as `coefficient * iv + offset` where `iv` is the
/// given iter-var name and both `coefficient` and `offset` const-fold to
/// integers. Returns `Some((coeff, offset))` on success; `None` if the
/// expression is not affine in `iv` in the recognised shape.
///
/// See the module-level docs for the exact set of recognised shapes.
///
/// `consts` is the algorithm's const-folding table; it lets a bound
/// like `OFFSET - 1` fold when `const OFFSET = 1` was declared.
pub(crate) fn affine_decompose(
    e: &IrExpr,
    iv: &str,
    consts: &BTreeMap<String, ResolvedConst>,
) -> Option<(i64, i64)> {
    // Base cases.
    match e {
        IrExpr::Ident(name) if name == iv => return Some((1, 0)),
        IrExpr::Neg(inner) => {
            // `-iv` → (-1, 0). `-(iv + c)` → (-1, -c). `-(c)` is a
            // constant and is handled in `eval_const_int` below.
            if let Some((c, d)) = affine_decompose(inner, iv, consts) {
                return Some((-c, -d));
            }
            // Pure-constant `-c` reaches here only if `inner` is
            // iv-independent (no Ident == iv). Try const-folding the
            // whole expression as a constant offset.
            if let Some(k) = eval_const_int(e, consts) {
                return Some((0, k));
            }
            return None;
        }
        _ => {}
    }

    // Const-foldable constants → (0, k). Catches IntLit, Ident-of-const,
    // and any composition that doesn't mention iv.
    if !expr_mentions(e, iv) {
        return eval_const_int(e, consts).map(|k| (0, k));
    }

    // BinOp cases.
    if let IrExpr::BinOp(op, lhs, rhs) = e {
        let lhs_aff = if expr_mentions(lhs, iv) {
            affine_decompose(lhs, iv, consts)
        } else {
            eval_const_int(lhs, consts).map(|k| (0_i64, k))
        };
        let rhs_aff = if expr_mentions(rhs, iv) {
            affine_decompose(rhs, iv, consts)
        } else {
            eval_const_int(rhs, consts).map(|k| (0_i64, k))
        };
        let (l, r) = (lhs_aff?, rhs_aff?);
        match op {
            IrBinOp::Add => return Some((l.0.checked_add(r.0)?, l.1.checked_add(r.1)?)),
            IrBinOp::Sub => return Some((l.0.checked_sub(r.0)?, l.1.checked_sub(r.1)?)),
            IrBinOp::Mul => {
                // c * iv / iv * c only — at most ONE side mentions iv.
                // (Both sides mentioning iv ⇒ iv*iv ⇒ not affine.)
                match (l.0, r.0) {
                    (0, _) => {
                        // l is pure constant l.1, r is c*iv + d → scale r.
                        return Some((l.1.checked_mul(r.0)?, l.1.checked_mul(r.1)?));
                    }
                    (_, 0) => {
                        return Some((r.1.checked_mul(l.0)?, r.1.checked_mul(l.1)?));
                    }
                    _ => return None,
                }
            }
            // Div / Mod with iv on either side are not affine in iv.
            IrBinOp::Div | IrBinOp::Mod => return None,
        }
    }

    None
}

/// Does `e` syntactically contain an `Ident(iv)` anywhere in its tree?
///
/// Note: DataRef / Call subtrees are NOT walked — the affine-stride
/// family rejects indices that contain DataRef / Call upstream
/// ([`crate::passes::halo_inference`] checks this with its own
/// `expr_contains_dataref_or_call` helper) before this predicate fires.
pub(crate) fn expr_mentions(e: &IrExpr, iv: &str) -> bool {
    match e {
        IrExpr::Ident(n) => n == iv,
        IrExpr::IntLit(_) => false,
        IrExpr::Neg(inner) => expr_mentions(inner, iv),
        IrExpr::BinOp(_, lhs, rhs) => expr_mentions(lhs, iv) || expr_mentions(rhs, iv),
        IrExpr::DataRef(_) | IrExpr::Call { .. } => false,
    }
}

/// Try to evaluate `e` as an integer constant. Returns `None` if `e`
/// references any non-const identifier (an iter-var or unknown name) or
/// contains a DataRef / Call / overflow / div-by-zero. Mirrors the
/// minimum subset of `algo::lower::eval_const` needed for affine
/// offset recovery — we deliberately keep this local + small so the
/// pass family has no upstream coupling beyond the `consts` table.
pub(crate) fn eval_const_int(e: &IrExpr, consts: &BTreeMap<String, ResolvedConst>) -> Option<i64> {
    match e {
        IrExpr::IntLit(v) => Some(*v),
        IrExpr::Ident(name) => consts.get(name).map(|c| c.value),
        IrExpr::Neg(inner) => eval_const_int(inner, consts).and_then(i64::checked_neg),
        IrExpr::BinOp(op, lhs, rhs) => {
            let l = eval_const_int(lhs, consts)?;
            let r = eval_const_int(rhs, consts)?;
            match op {
                IrBinOp::Add => l.checked_add(r),
                IrBinOp::Sub => l.checked_sub(r),
                IrBinOp::Mul => l.checked_mul(r),
                IrBinOp::Div => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_div(r)
                    }
                }
                IrBinOp::Mod => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_rem(r)
                    }
                }
            }
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => None,
    }
}

// --------------------------------------------------------------------
// Unit tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! These tests cover the helpers in isolation — same coverage as
    //! the pre-cycle-82 `affine_decompose_*` tests inside
    //! `halo_inference.rs`, moved here verbatim with their pass-context
    //! stripped. The halo + reuse passes carry their own integration
    //! tests that drive these helpers from real IR.
    use super::*;
    use crate::algo::ResolvedConst;
    use crate::algo::ScalarType;

    fn ir_int(v: i64) -> IrExpr {
        IrExpr::IntLit(v)
    }
    fn ir_id(s: &str) -> IrExpr {
        IrExpr::Ident(s.to_string())
    }
    fn ir_add(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Add, Box::new(l), Box::new(r))
    }
    fn ir_sub(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Sub, Box::new(l), Box::new(r))
    }
    fn ir_mul(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Mul, Box::new(l), Box::new(r))
    }

    #[test]
    fn affine_decompose_iv_plus_one() {
        let e = ir_add(ir_id("y"), ir_int(1));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, 1)));
    }

    #[test]
    fn affine_decompose_iv_minus_one() {
        let e = ir_sub(ir_id("y"), ir_int(1));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, -1)));
    }

    #[test]
    fn affine_decompose_const_plus_iv() {
        let e = ir_add(ir_int(2), ir_id("y"));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, 2)));
    }

    #[test]
    fn affine_decompose_bare_iv() {
        let e = ir_id("y");
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, 0)));
    }

    #[test]
    fn affine_decompose_negated_iv() {
        let e = IrExpr::Neg(Box::new(ir_id("y")));
        // coefficient -1 — recognised but caller rejects.
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((-1, 0)));
    }

    #[test]
    fn affine_decompose_strided_two_iv() {
        let e = ir_mul(ir_int(2), ir_id("y"));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((2, 0)));
    }

    #[test]
    fn affine_decompose_iv_squared_is_none() {
        let e = ir_mul(ir_id("y"), ir_id("y"));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), None);
    }

    #[test]
    fn affine_decompose_uses_const_table() {
        // const STRIDE = 2; index = y + STRIDE → (1, 2).
        let mut consts = BTreeMap::new();
        consts.insert(
            "STRIDE".to_string(),
            ResolvedConst {
                name: "STRIDE".to_string(),
                ty: ScalarType::I32,
                value: 2,
            },
        );
        let e = ir_add(ir_id("y"), ir_id("STRIDE"));
        assert_eq!(affine_decompose(&e, "y", &consts), Some((1, 2)));
    }

    #[test]
    fn eval_const_int_int_lit() {
        assert_eq!(eval_const_int(&ir_int(7), &BTreeMap::new()), Some(7));
    }

    #[test]
    fn eval_const_int_div_by_zero_is_none() {
        // 1 / 0 → None (not panic).
        let e = IrExpr::BinOp(IrBinOp::Div, Box::new(ir_int(1)), Box::new(ir_int(0)));
        assert_eq!(eval_const_int(&e, &BTreeMap::new()), None);
    }

    #[test]
    fn eval_const_int_dataref_is_none() {
        // A DataRef in the constant expression → None (not foldable).
        let e = IrExpr::DataRef(crate::algo::IndexedRef {
            name: "grid".to_string(),
            indices: vec![ir_int(0)],
        });
        assert_eq!(eval_const_int(&e, &BTreeMap::new()), None);
    }

    #[test]
    fn expr_mentions_finds_iv() {
        let e = ir_add(ir_id("y"), ir_int(1));
        assert!(expr_mentions(&e, "y"));
        assert!(!expr_mentions(&e, "x"));
    }

    #[test]
    fn expr_mentions_skips_dataref_subtree() {
        // The predicate intentionally does NOT walk DataRef subtrees —
        // the upstream guard rejects DataRef-containing indices before
        // this predicate is reached.
        let e = IrExpr::DataRef(crate::algo::IndexedRef {
            name: "grid".to_string(),
            indices: vec![ir_id("y")],
        });
        assert!(!expr_mentions(&e, "y"));
    }

    // -----------------------------------------------------------------
    // compute_partition_bands (TASK-0262)
    // -----------------------------------------------------------------

    #[test]
    fn bands_exact_divisible_matches_old_policy() {
        // 16 rows across 4 workers: 4, 4, 4, 4 — same as the
        // pre-TASK-0262 exact-split policy.
        let bands = compute_partition_bands(0, 16, 4).expect("divisible OK");
        assert_eq!(bands, vec![(0, 4), (4, 8), (8, 12), (12, 16)]);
    }

    #[test]
    fn bands_non_divisible_spillover_05_stencil_shape() {
        // The 05-stencil/distributed case: y-loop walks 1..15
        // (length 14), 4 workers. Floor = 3, extras = 14 % 4 = 2 → first
        // 2 workers get 4 rows, last 2 get 3. Spillover bands
        // contiguous from lo=1.
        let bands = compute_partition_bands(1, 15, 4).expect("non-divisible accepted");
        assert_eq!(bands, vec![(1, 5), (5, 9), (9, 12), (12, 15)]);
        // Union of bands covers [1, 15) exactly:
        assert_eq!(
            bands.iter().map(|(lo, hi)| hi - lo).sum::<i64>(),
            14,
            "bands total width equals source length"
        );
    }

    #[test]
    fn bands_17_across_3_returns_6_6_5() {
        // 17 rows, 3 workers. Floor=5, extras=17%3=2 → 6,6,5.
        let bands = compute_partition_bands(0, 17, 3).expect("non-divisible accepted");
        assert_eq!(bands, vec![(0, 6), (6, 12), (12, 17)]);
    }

    #[test]
    fn bands_negative_origin_supported() {
        // The lo can be negative (no in-tree example uses this, but the
        // helper must not assume lo >= 0). 5 rows from -2..3 across 2
        // workers: floor=2, extras=1 → 3,2.
        let bands = compute_partition_bands(-2, 3, 2).expect("negative origin OK");
        assert_eq!(bands, vec![(-2, 1), (1, 3)]);
    }

    #[test]
    fn bands_insufficient_work_rejects() {
        // 3 rows across 4 workers — even spillover can't give every
        // worker one row. Hard reject.
        let err = compute_partition_bands(0, 3, 4).expect_err("L<N must reject");
        assert_eq!(
            err,
            PartitionBandError::InsufficientWork {
                len: 3,
                workers: 4
            }
        );
    }

    #[test]
    fn bands_equal_to_workers_one_each() {
        // L == N → 1 row per worker, no spillover.
        let bands = compute_partition_bands(0, 4, 4).expect("L==N OK");
        assert_eq!(bands, vec![(0, 1), (1, 2), (2, 3), (3, 4)]);
    }

    #[test]
    fn bands_zero_workers_rejects() {
        let err = compute_partition_bands(0, 16, 0).expect_err("0 workers must reject");
        assert_eq!(err, PartitionBandError::ZeroWorkers);
    }

    #[test]
    fn bands_inverted_range_rejects() {
        // Defensive — link step's eval_const produces hi >= lo for
        // valid user input; this guards a malformed synthetic.
        let err = compute_partition_bands(10, 5, 2).expect_err("inverted range must reject");
        assert_eq!(err, PartitionBandError::InvalidRange { lo: 10, hi: 5 });
    }
}
