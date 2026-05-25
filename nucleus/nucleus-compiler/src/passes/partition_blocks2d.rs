//! Partition-blocks2d loop transformation — TASK-0259.
//!
//! Consumes `ResolvedLoopOption::Partition(PartitionKind::Blocks2d)`
//! from `linked.sched.loops` and partitions BOTH axes of an outer-of-2D
//! Repeat-of-Repeat nest into a 2D grid of worker blocks. Each worker
//! owns one (y_band x x_band) rectangle.
//!
//! ## Relationship to `partition_workers` and `partition_rows`
//!
//! All three passes write into the same
//! [`crate::acfg::ACFG::partition_worker_ranges`] sidecar — a
//! `BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>` consumed at
//! projection time by [`crate::passes::petri_to_events`] and the
//! [`backend_common::multi_worker_walker`] (which honours the override
//! when it emits one `Event::Loop` per worker).
//!
//! - `partition=workers` (TASK-0212): 1D compiler-choice row-band on
//!   ANY multi-worker body; writes ONE entry under the loop's iter_var.
//! - `partition=rows` (TASK-0258): explicit 1D row-band on the OUTER
//!   loop of a 2D nest; writes ONE entry under the outer iter_var;
//!   inner Repeat runs intact per worker.
//! - `partition=blocks2d` (this pass): 2D grid of worker blocks on a
//!   Repeat-of-Repeat; writes TWO entries — one under the outer
//!   iter_var (per-worker y-band) and one under the inner iter_var
//!   (per-worker x-band). The walker's per-worker partition_slice
//!   lookup applies each entry INDEPENDENTLY to whichever Repeat it is
//!   rendering, so the (y_band, x_band) effect emerges from two
//!   independent lookups firing on the same worker's render. No new
//!   sidecar shape is needed.
//!
//! ## Why Option A (reuse sidecar) instead of Option B (new field)
//!
//! Option B would add a new
//! `partition_worker_ranges_2d: BTreeMap<(IterVar_y, IterVar_x), BTreeMap<WorkerId, (Range, Range)>>`
//! field plus its serde + walker consumer. Option A reuses the
//! existing per-iter_var per-worker map: the walker
//! ([`backend_common::multi_worker_walker::render_worker_events_inner`])
//! already keys on `(iter_var, worker_id)` independently for each
//! `Event::Loop` it emits — verified in cycle 80 by reading the
//! `Event::Loop` match-arm inside `render_worker_events_inner`
//! (grep-witness: `grep -nE "Event::Loop\s*\{"
//! nucleus/backend-common/src/multi_worker_walker.rs` returns five
//! hits across the file — exactly one is inside
//! `render_worker_events_inner` and uses `partition_worker_ranges[iv]`
//! per worker; the other four are read-only `collect_*` walkers
//! that descend into the loop body without per-worker range lookup).
//! Writing two entries (one per iter_var) under the same worker key
//! set yields the 2D-block effect without changing the sidecar
//! contract.
//!
//! Option A is preferred because it:
//!   - keeps the sidecar shape stable (one fewer ACFG field, one fewer
//!     serde version concern);
//!   - lets all three partition passes share one downstream consumer
//!     surface; and
//!   - works without any backend / walker change.
//!
//! ## Grid-shape decomposition
//!
//! Given `N` workers, the pass deterministically picks the
//! closest-to-square `(R, C)` such that `R * C = N` and `R <= C`. The
//! algorithm walks `i` from `floor(sqrt(N))` down to `1`, returning the
//! first `i` that divides `N` evenly. Examples:
//!
//!   - `N = 4`  → `(2, 2)` (perfect square)
//!   - `N = 6`  → `(2, 3)` (floor(sqrt(6)) = 2, 6 % 2 == 0)
//!   - `N = 9`  → `(3, 3)` (perfect square)
//!   - `N = 12` → `(3, 4)` (floor(sqrt(12)) = 3, 12 % 3 == 0)
//!   - `N = 7`  → degenerate (only (1, 7)); rejected as
//!     [`PartitionBlocks2dError::DegenerateGridShape`].
//!
//! A prime worker count > 1 yields the degenerate `(1, N)` 1D-row
//! decomposition — that defeats the point of `partition=blocks2d`. PRD
//! §6.3.3 line 519 says category errors reject at compile time; a
//! prime-decomposition degenerate grid is exactly that, so we reject
//! it with a typed error rather than silently falling through to a 1D
//! row-band (which is what `partition=rows` is for).
//!
//! ## Worker → (row, col) assignment
//!
//! The body's worker union (collected with
//! [`crate::passes::partition_rows::collect_op_workers`]) is iterated
//! in `BTreeSet<WorkerId>` numeric order. Worker `i` (0-indexed) is
//! assigned grid cell `(row = i / C, col = i % C)`. The cell owns
//! `y_band = y_lo + row * (y_len / R) .. y_lo + (row + 1) * (y_len / R)`
//! and the analogous x band. Deterministic by construction.
//!
//! ## Honest limitations (first cut)
//!
//! - **Exact-divisible split on BOTH axes.** `(y_hi - y_lo) % R != 0`
//!   or `(x_hi - x_lo) % C != 0` is reported as a typed
//!   [`PartitionBlocks2dError::NonDivisible`] error with the `axis`
//!   field set to `'y'` or `'x'`. The remainder policy is a shared
//!   follow-up with `partition_workers` and `partition_rows`
//!   (TASK-0262); this pass adopts the same first-cut limit so the
//!   policy fix lands in one place.
//! - **No new e2e cell in this cycle.** Without halo inference
//!   (TASK-0260), a 2D-block-partitioned stencil produces WRONG output
//!   at both axis boundaries (each worker reads neighbours it does not
//!   own, on BOTH the y and x axes). The bit-identical e2e cell that
//!   would exercise `partition=blocks2d` cannot ship until TASK-0260
//!   (halo inference for 2D-block boundaries) AND a divisible nest
//!   shape are both available. Filed honestly in the task brief.
//! - **No grid-shape override directive.** The grid `(R, C)` is
//!   inferred from `N`; the schedule grammar has no explicit
//!   `partition=blocks2d(R, C)` form. If `N`'s closest-to-square
//!   decomposition is not what the schedule author intended, the only
//!   recourse today is to change the worker count. A future
//!   grammar/IR extension to accept an explicit `(R, C)` belongs to a
//!   separate task — file under TASK-0259 follow-ups if the need
//!   surfaces.
//! - **No interaction with `block=`.** Same caveat as
//!   `partition_workers`: combining `block=N` and `partition=blocks2d`
//!   on the same loop is undefined behaviour for this first cut. None
//!   of the in-tree schedules mix the two.
//!
//! ## Pipeline placement
//!
//! Runs immediately after
//! [`crate::passes::partition_rows::apply_partition_rows`] in the
//! driver pipeline. Order between the three partition passes is purely
//! diagnostic — the grammar accepts at most one `partition=` option
//! per loop (`SchedLowerErrorKind::DuplicateLoopOption`), so the three
//! passes target DISJOINT IterVar keys in `partition_worker_ranges`.
//!
//! ## Determinism
//!
//! Same discipline as the sibling passes: sidecar is two nested
//! `BTreeMap`s keyed by `u64` ids; worker subsets come from
//! `BTreeSet<WorkerId>` (numeric order); grid-shape decomposition is
//! deterministic by construction (walk `i` from `floor(sqrt(N))` down
//! to `1`, first divisor wins). Same input ⇒ byte-identical sidecar.

use std::collections::{BTreeMap, BTreeSet};

use crate::acfg::{ACFGNode, ACFG};
use crate::event::{IterVar, WorkerId};
use crate::link::LinkedIR;
use crate::passes::partition_rows::find_outer_of_2d;
use crate::sched::{PartitionKind, ResolvedLoopOption};

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors produced by [`apply_partition_blocks2d`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionBlocks2dError {
    /// The schedule referenced `partition=blocks2d` for a loop variable
    /// that the algorithm doesn't loop over. The linker normally
    /// pre-rejects this; the variant exists so the pass fails closed
    /// on an inconsistently-constructed `(LinkedIR, ACFG)` pair (same
    /// invariant guard the sibling passes carry).
    UnknownLoopVar { var: String },
    /// The loop carrying `partition=blocks2d` is NOT the outer of a 2D
    /// nest — i.e. its body does not contain another `Repeat` node
    /// nested under it (possibly through `Sequence` wrappers). PRD
    /// §6.3.3 line 519: "`partition=blocks2d` on a 1D iteration" is a
    /// bad combination rejected at compile time. The semantics of
    /// "2D blocks" presuppose two iteration axes; applying it to a 1D
    /// loop is a category error.
    NotOuterOf2DNest { var: String },
    /// The inner body's worker union has fewer than two workers — the
    /// 2D-block partition is meaningless. Same rationale as
    /// `PartitionRowsError::NoMultiWorkerBody`.
    NoMultiWorkerBody { var: String, workers: usize },
    /// The worker count `N` does not factor into a non-degenerate 2D
    /// grid — its closest-to-square decomposition is `(1, N)`. This
    /// happens for `N` prime (and for `N == 1`, but `NoMultiWorkerBody`
    /// catches that first). The schedule author should either:
    ///   - choose a non-prime worker count whose factorisation matches
    ///     the intended grid; or
    ///   - use `partition=rows` if a 1D row-band is what they wanted.
    DegenerateGridShape { var: String, workers: usize },
    /// The y or x range length is not evenly divisible by the
    /// corresponding grid dimension (`R` for y, `C` for x). First-cut
    /// policy: refuse compile. The `axis` field distinguishes y from
    /// x for the diagnostic. The remainder policy is a shared
    /// follow-up with `partition_workers` / `partition_rows`
    /// (TASK-0262).
    NonDivisible {
        var: String,
        axis: char,
        lo: i64,
        hi: i64,
        cells: usize,
    },
    /// The inner Repeat could not be located in the outer's body
    /// subtree even though `has_inner_repeat` is true. This is a
    /// projection-layer invariant violation — the pass walks
    /// `find_outer_of_2d` + `find_first_inner_repeat`; the latter
    /// MUST return `Some` whenever `contains_repeat(body)` returns
    /// true. Carries the outer var name so a future bug bisects.
    InnerRepeatNotFound { outer_var: String },
}

impl std::fmt::Display for PartitionBlocks2dError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartitionBlocks2dError::UnknownLoopVar { var } => write!(
                f,
                "schedule has `loop {var} : partition=blocks2d` but the ACFG contains no loop \
                 with variable `{var}` (linker-pass invariant violation)"
            ),
            PartitionBlocks2dError::NotOuterOf2DNest { var } => write!(
                f,
                "loop `{var}` has `partition=blocks2d` but is NOT the outer loop of a 2D nest \
                 (no inner Repeat in its body). `partition=blocks2d` requires a 2D iteration \
                 nest (outer = row index, inner = column index); applying it to a 1D loop is a \
                 category error (PRD §6.3.3). Either nest a second loop inside `{var}`, or use \
                 `partition=workers` / `partition=rows`, or omit the directive."
            ),
            PartitionBlocks2dError::NoMultiWorkerBody { var, workers } => write!(
                f,
                "schedule has `loop {var} : partition=blocks2d` but the inner loop body is \
                 placed on {workers} worker(s); `partition=blocks2d` requires a multi-worker \
                 body to form a 2D grid (one worker = no grid)"
            ),
            PartitionBlocks2dError::DegenerateGridShape { var, workers } => write!(
                f,
                "loop `{var}` has `partition=blocks2d` over {workers} worker(s); the \
                 closest-to-square 2D factorisation of {workers} is (1, {workers}) which is a \
                 degenerate 1D row-band, not a 2D grid (TASK-0259: prime / 1-factor worker \
                 counts have no non-degenerate 2D decomposition). Choose a worker count whose \
                 factorisation matches the intended grid (e.g. 4 → 2x2, 6 → 2x3, 9 → 3x3, \
                 12 → 3x4), or use `partition=rows` for a 1D row-band."
            ),
            PartitionBlocks2dError::NonDivisible {
                var,
                axis,
                lo,
                hi,
                cells,
            } => write!(
                f,
                "loop `{var}` `partition=blocks2d`: {axis}-axis range {lo}..{hi} (length \
                 {len}) is not evenly divisible across {cells} grid cells on that axis \
                 (TASK-0259 first cut: exact-divisible only on BOTH axes; remainder policy \
                 is a shared follow-up with partition_rows / partition_workers — TASK-0262)",
                len = hi - lo
            ),
            PartitionBlocks2dError::InnerRepeatNotFound { outer_var } => write!(
                f,
                "loop `{outer_var}` `partition=blocks2d`: outer Repeat structurally matches but \
                 the inner Repeat could not be located in its body subtree (projection-layer \
                 invariant violation — `contains_repeat` and `find_first_inner_repeat` \
                 disagreed). Please file a bug with the failing schedule."
            ),
        }
    }
}

impl std::error::Error for PartitionBlocks2dError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Walk `acfg` and populate
/// [`ACFG::partition_worker_ranges`](crate::acfg::ACFG::partition_worker_ranges)
/// for every loop carrying a `partition=blocks2d` directive whose ACFG
/// shape is the outer of a 2D nest with a multi-worker inner body.
/// Writes TWO entries per directive (outer iter_var → per-worker y-band;
/// inner iter_var → per-worker x-band) so the walker's independent
/// per-iter_var lookup forms the (y_band x x_band) rectangle on each
/// worker's render.
///
/// Pure: input ACFG is consumed, a new one with the sidecar populated
/// is returned. The tree itself is forwarded unchanged.
///
/// On any error, no partial sidecar is committed — the function
/// validates every directive up front before mutating the sidecar.
pub fn apply_partition_blocks2d(
    linked: &LinkedIR,
    acfg: ACFG,
) -> Result<ACFG, PartitionBlocks2dError> {
    // ---- 1. Collect every `partition=blocks2d` directive. ----
    //
    // BTreeSet iteration keeps the pass deterministic.
    let mut partition_vars: BTreeSet<String> = BTreeSet::new();
    for (var, directive) in &linked.sched.loops {
        for opt in &directive.options {
            if matches!(opt, ResolvedLoopOption::Partition(PartitionKind::Blocks2d)) {
                partition_vars.insert(var.clone());
            }
        }
    }

    if partition_vars.is_empty() {
        return Ok(acfg);
    }

    // ---- 2. Pre-validate every directive. ----
    //
    // For each directive: resolve name → id, locate the outer Repeat,
    // verify it carries an inner Repeat (outer-of-2D structural check),
    // locate the inner Repeat's iter_var + range, collect the inner
    // body's worker union, factor `N = R * C` (close-to-square),
    // validate divisibility on BOTH axes. Accumulate the recorded
    // entries; commit only after every directive validates.
    struct Plan {
        outer_iter_var: IterVar,
        outer_range: std::ops::Range<i64>,
        inner_iter_var: IterVar,
        inner_range: std::ops::Range<i64>,
        body_workers: BTreeSet<WorkerId>,
        grid_rows: usize,
        grid_cols: usize,
    }
    let mut to_record: Vec<Plan> = Vec::with_capacity(partition_vars.len());
    for var in &partition_vars {
        let outer_iter_var = match acfg.name_iter_vars.get(var) {
            Some(v) => *v,
            None => {
                return Err(PartitionBlocks2dError::UnknownLoopVar { var: var.clone() });
            }
        };
        let Some((outer_range, has_inner_repeat, body_workers)) =
            find_outer_of_2d(&acfg.root, outer_iter_var)
        else {
            return Err(PartitionBlocks2dError::UnknownLoopVar { var: var.clone() });
        };
        if !has_inner_repeat {
            return Err(PartitionBlocks2dError::NotOuterOf2DNest { var: var.clone() });
        }
        if body_workers.len() < 2 {
            return Err(PartitionBlocks2dError::NoMultiWorkerBody {
                var: var.clone(),
                workers: body_workers.len(),
            });
        }

        // Locate the inner Repeat's iter_var + range.
        let Some((inner_iter_var, inner_range)) =
            find_first_inner_repeat(&acfg.root, outer_iter_var)
        else {
            return Err(PartitionBlocks2dError::InnerRepeatNotFound {
                outer_var: var.clone(),
            });
        };

        // Grid-shape decomposition.
        let n = body_workers.len();
        let Some((grid_rows, grid_cols)) = decompose_grid(n) else {
            return Err(PartitionBlocks2dError::DegenerateGridShape {
                var: var.clone(),
                workers: n,
            });
        };

        // Divisibility on both axes.
        let y_len = outer_range.end - outer_range.start;
        if y_len % (grid_rows as i64) != 0 {
            return Err(PartitionBlocks2dError::NonDivisible {
                var: var.clone(),
                axis: 'y',
                lo: outer_range.start,
                hi: outer_range.end,
                cells: grid_rows,
            });
        }
        let x_len = inner_range.end - inner_range.start;
        if x_len % (grid_cols as i64) != 0 {
            return Err(PartitionBlocks2dError::NonDivisible {
                var: var.clone(),
                axis: 'x',
                lo: inner_range.start,
                hi: inner_range.end,
                cells: grid_cols,
            });
        }

        to_record.push(Plan {
            outer_iter_var,
            outer_range,
            inner_iter_var,
            inner_range,
            body_workers,
            grid_rows,
            grid_cols,
        });
    }

    // ---- 3. Commit the sidecar entries. ----

    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        mut partition_worker_ranges,
        pipeline_depth_for_seq,
        // partition_blocks2d does not consult or mutate halo widths
        // (TASK-0260); forward verbatim. Stage 3 (TASK-0289 halo-strip
        // Push/Wait synthesis) will couple halo to partition_blocks2d's
        // block-pair metadata, but that is a DIFFERENT join (worker ->
        // neighbour grid cell), not a mutation of `halo_widths` itself.
        halo_widths,
        // partition_blocks2d does not consult or mutate reuse widths
        // (TASK-0261); forward verbatim. Reuse is independent of
        // partition policy (the delay-line lives within ONE worker's
        // tile; partitioning bounds the iv-range each worker covers).
        reuse_widths,
        // TASK-0264 cycle 113 AC#1+2: this pass IS the populator of
        // partition_pairs + grid_shape_for_outer_iv. Bring the (likely-
        // empty) existing maps into scope mutably so the to_record loop
        // below can extend them per-directive, preserving any prior
        // entries (composition with other partition passes — e.g. a
        // future schedule combining partition=blocks2d on one nest with
        // partition=workers on a sibling nest).
        mut partition_pairs,
        mut grid_shape_for_outer_iv,
    } = acfg;

    for plan in to_record {
        let y_len = plan.outer_range.end - plan.outer_range.start;
        let x_len = plan.inner_range.end - plan.inner_range.start;
        let y_slice = y_len / (plan.grid_rows as i64);
        let x_slice = x_len / (plan.grid_cols as i64);

        let mut per_worker_y: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
        let mut per_worker_x: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
        for (i, wid) in plan.body_workers.iter().enumerate() {
            let row = i / plan.grid_cols;
            let col = i % plan.grid_cols;
            let y_lo = plan.outer_range.start + (row as i64) * y_slice;
            let y_hi = y_lo + y_slice;
            let x_lo = plan.inner_range.start + (col as i64) * x_slice;
            let x_hi = x_lo + x_slice;
            per_worker_y.insert(*wid, y_lo..y_hi);
            per_worker_x.insert(*wid, x_lo..x_hi);
        }
        partition_worker_ranges.insert(plan.outer_iter_var, per_worker_y);
        partition_worker_ranges.insert(plan.inner_iter_var, per_worker_x);

        // TASK-0264 cycle 113: persist the pair + grid shape next to
        // the per-worker ranges. The pair is keyed by outer_iter_var
        // (matching partition_worker_ranges' outer-axis entry); the
        // grid shape is keyed identically. A downstream consumer
        // (TASK-0289 halo-strip Push/Wait synthesis) reads
        // sidecar.partition_pairs.get(outer_iv) to recover the
        // matching inner iv and sidecar.grid_shape_for_outer_iv.get
        // (outer_iv) for the (rows, cols) inversion.
        //
        // `decompose_grid` returns `(usize, usize)` (max worker count
        // ~hundreds in practice); cast to u32 for sidecar storage
        // (compact + serde-friendly; usize would vary by host arch).
        partition_pairs.insert(plan.outer_iter_var, plan.inner_iter_var);
        grid_shape_for_outer_iv.insert(
            plan.outer_iter_var,
            (plan.grid_rows as u32, plan.grid_cols as u32),
        );
    }

    Ok(ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
    })
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Find the (iter_var, range) of the FIRST inner `Repeat` reachable
/// from the body of the outer Repeat whose iter_var equals
/// `outer_target`. Walks `Sequence` children inside the outer body.
///
/// Returns `None` if either the outer Repeat is not present in the
/// subtree or its body contains no inner Repeat. The caller is expected
/// to have ALREADY verified `find_outer_of_2d` returned
/// `has_inner_repeat == true` before invoking this; the `None` branch
/// here is a defensive belt-and-braces guard ([`PartitionBlocks2dError::InnerRepeatNotFound`]).
///
/// "First" is deterministic: depth-first, child-order. Multiple
/// sibling inner Repeats under the same outer would produce an
/// ambiguous shape — `partition=blocks2d` is contracted only for a
/// strict Repeat-of-Repeat nest, so picking the first is fine for the
/// in-scope schedules.
fn find_first_inner_repeat(
    node: &ACFGNode,
    outer_target: IterVar,
) -> Option<(IterVar, std::ops::Range<i64>)> {
    match node {
        ACFGNode::Repeat { iter_var, body, .. } => {
            if *iter_var == outer_target {
                // Found the outer; search the body for any Repeat.
                first_repeat_in(body)
            } else {
                find_first_inner_repeat(body, outer_target)
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                if let Some(r) = find_first_inner_repeat(c, outer_target) {
                    return Some(r);
                }
            }
            None
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => None,
    }
}

/// Locate the (iter_var, range) of the first `Repeat` node reachable
/// in this subtree (depth-first, sequence-child order). Used by
/// [`find_first_inner_repeat`] once the outer Repeat has been
/// identified.
fn first_repeat_in(node: &ACFGNode) -> Option<(IterVar, std::ops::Range<i64>)> {
    match node {
        ACFGNode::Repeat {
            iter_var, range, ..
        } => Some((*iter_var, range.clone())),
        ACFGNode::Sequence(children) => {
            for c in children {
                if let Some(r) = first_repeat_in(c) {
                    return Some(r);
                }
            }
            None
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => None,
    }
}

/// Decompose `n` into a (rows, cols) grid such that `rows * cols == n`
/// and `rows <= cols` with `rows` as close to `sqrt(n)` as possible.
///
/// Walks `i` from `floor(sqrt(n))` down to `1`; returns the first
/// `(i, n/i)` where `n % i == 0`. Returns `None` for the degenerate
/// case where the only factorisation is `(1, n)` (i.e. `n > 1` and
/// `n` prime). `n == 0` is also `None` (no zero-worker grid is
/// meaningful; callers reject this earlier via
/// `NoMultiWorkerBody`). `n == 1` returns `Some((1, 1))` —
/// `NoMultiWorkerBody` should already have fired upstream, but the
/// helper is honest about its domain.
///
/// Deterministic and dependency-free (uses integer math only).
fn decompose_grid(n: usize) -> Option<(usize, usize)> {
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some((1, 1));
    }
    // floor(sqrt(n)) via integer-only loop (no f64 → usize rounding
    // surprises around 2^52). Find the largest k with k * k <= n.
    let mut sqrt_n: usize = 1;
    while (sqrt_n + 1) * (sqrt_n + 1) <= n {
        sqrt_n += 1;
    }
    // Walk i from sqrt_n down to 2; first divisor wins. If none divides
    // (n is prime), the only factorisation is (1, n) — degenerate.
    let mut i = sqrt_n;
    while i >= 2 {
        if n % i == 0 {
            return Some((i, n / i));
        }
        i -= 1;
    }
    // n is prime (or n == 2 / 3 / 5 / 7 / ... with only (1, n)).
    None
}

// --------------------------------------------------------------------
// Unit tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_grid_perfect_square_4() {
        assert_eq!(decompose_grid(4), Some((2, 2)));
    }

    #[test]
    fn decompose_grid_non_square_6() {
        // floor(sqrt(6)) == 2; 6 % 2 == 0 → (2, 3).
        assert_eq!(decompose_grid(6), Some((2, 3)));
    }

    #[test]
    fn decompose_grid_perfect_square_9() {
        assert_eq!(decompose_grid(9), Some((3, 3)));
    }

    #[test]
    fn decompose_grid_non_square_12() {
        // floor(sqrt(12)) == 3; 12 % 3 == 0 → (3, 4).
        assert_eq!(decompose_grid(12), Some((3, 4)));
    }

    #[test]
    fn decompose_grid_prime_7_is_degenerate() {
        assert_eq!(decompose_grid(7), None);
    }

    #[test]
    fn decompose_grid_prime_11_is_degenerate() {
        assert_eq!(decompose_grid(11), None);
    }

    #[test]
    fn decompose_grid_one_is_identity() {
        assert_eq!(decompose_grid(1), Some((1, 1)));
    }

    #[test]
    fn decompose_grid_zero_is_none() {
        assert_eq!(decompose_grid(0), None);
    }

    #[test]
    fn decompose_grid_two_is_degenerate() {
        // The only factorisation of 2 is (1, 2). NoMultiWorkerBody
        // would reject 1-worker bodies upstream, but 2 workers wanting
        // a 2D grid is also degenerate by this pass's contract — the
        // helper says so honestly.
        assert_eq!(decompose_grid(2), None);
    }

    /// Sanity: contains_repeat (lifted to pub(crate) in cycle 80 so
    /// this pass + partition_rows share the same outer-of-2D check)
    /// still works for a 2D Repeat-of-Repeat body when called via the
    /// `crate::passes::partition_rows` re-export path.
    #[test]
    fn contains_repeat_finds_inner_via_pub_crate() {
        use crate::acfg::{DataflowDag, DataflowEdge, Operation};
        use crate::event::{DataId, KernelId};
        use crate::passes::partition_rows::contains_repeat;
        let mut workers = BTreeSet::new();
        workers.insert(WorkerId(1));
        let op = ACFGNode::Operation(Operation {
            kernel: KernelId(0),
            workers,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    vec![DataId(0)],
                    KernelId(0),
                    Some(DataId(1)),
                )],
            },
        });
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(8),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![op])),
            block_tag: None,
        };
        let body = ACFGNode::Sequence(vec![inner]);
        assert!(contains_repeat(&body));
    }

    /// find_first_inner_repeat returns the inner (iter_var, range) for
    /// a Repeat-of-Repeat where the outer matches.
    #[test]
    fn find_first_inner_repeat_picks_inner() {
        use crate::acfg::{DataflowDag, DataflowEdge, Operation};
        use crate::event::{DataId, KernelId};
        let mut workers = BTreeSet::new();
        workers.insert(WorkerId(1));
        let op = ACFGNode::Operation(Operation {
            kernel: KernelId(0),
            workers,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    vec![DataId(0)],
                    KernelId(0),
                    Some(DataId(1)),
                )],
            },
        });
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(8),
            range: 5..15,
            body: Box::new(ACFGNode::Sequence(vec![op])),
            block_tag: None,
        };
        let outer = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![inner])),
            block_tag: None,
        };
        let (iv, range) = find_first_inner_repeat(&outer, IterVar(7)).unwrap();
        assert_eq!(iv, IterVar(8));
        assert_eq!(range, 5..15);
    }

    /// On a 1D Repeat (no inner Repeat), find_first_inner_repeat
    /// returns None — the InnerRepeatNotFound defensive guard belt.
    #[test]
    fn find_first_inner_repeat_no_inner_returns_none() {
        use crate::acfg::{DataflowDag, DataflowEdge, Operation};
        use crate::event::{DataId, KernelId};
        let mut workers = BTreeSet::new();
        workers.insert(WorkerId(1));
        workers.insert(WorkerId(2));
        let op = ACFGNode::Operation(Operation {
            kernel: KernelId(0),
            workers,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    vec![DataId(0)],
                    KernelId(0),
                    Some(DataId(1)),
                )],
            },
        });
        let outer = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![op])),
            block_tag: None,
        };
        assert!(find_first_inner_repeat(&outer, IterVar(7)).is_none());
    }
}
