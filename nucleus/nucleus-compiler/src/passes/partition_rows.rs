//! Partition-rows loop transformation — TASK-0258.
//!
//! Consumes `ResolvedLoopOption::Partition(PartitionKind::Rows)` from
//! `linked.sched.loops` and records a per-(IterVar, WorkerId) loop-range
//! override on the ACFG's [`crate::acfg::ACFG::partition_worker_ranges`]
//! sidecar. The override is honoured at projection time by
//! `crate::passes::petri_to_events::walk` when it emits one
//! [`crate::event::Event::Loop`] per worker, so each worker sees its
//! own exclusive row-band of the OUTER axis.
//!
//! ## Relationship to `partition_workers`
//!
//! The PARTITIONING ALGORITHM (divisible / round-robin band assignment)
//! is identical to [`crate::passes::partition_workers`]. The
//! semantically-meaningful difference is the **structural pre-condition**:
//!
//! - `partition=workers` (TASK-0212) applies to a single loop nest with
//!   a multi-worker body — the schedule author asks the compiler to
//!   choose a band axis (the loop carrying the directive).
//! - `partition=rows` (TASK-0258, this pass) applies to the OUTER loop
//!   of a 2D nest — the schedule author says "I want explicit row-bands
//!   along this axis", and the compiler verifies the nest is structurally
//!   Repeat-of-Repeat (an outer Repeat whose body contains at least one
//!   inner Repeat reachable through `Sequence`s) before applying. The
//!   per-worker row-band is sliced across the INNER body's worker union
//!   (whoever runs the row), NOT across the outer Repeat's worker label;
//!   the same-worker case is the typical one but the code does not pin
//!   "outer Repeat's worker equals inner Repeat's workers" as an
//!   invariant. On a 1D loop, `partition=rows` is a category error per
//!   PRD §6.3.3 ("bad combinations: `partition=rows` on a 1D iteration").
//!
//! Both passes write into the SAME sidecar field
//! (`ACFG::partition_worker_ranges`) because downstream consumers
//! (`sync_inject`, `petri_to_events`, the backend walkers) only need
//! the per-worker range — they do not care which directive produced it.
//! This is intentional: the "extra validation" partition=rows adds is
//! captured by the pass entry, not by the downstream IR shape.
//!
//! ## Helper sharing with `partition_blocks2d`
//!
//! TASK-0259 (cycle 80) lifted `find_outer_of_2d`, `contains_repeat`
//! and `collect_op_workers` to `pub(crate)` so the
//! [`crate::passes::partition_blocks2d`] sibling consumes the same
//! helpers without triple-duplication. The structural pre-condition
//! (outer-of-2D Repeat-of-Repeat with a multi-worker inner body) is
//! byte-identical between `partition_rows` and `partition_blocks2d`;
//! only the per-worker range computation diverges (1D row-bands vs 2D
//! grid-block decomposition).
//!
//! The byte-identical `collect_op_workers` in
//! [`crate::passes::partition_workers`] (TASK-0212) remains as a third
//! local copy. Lifting it across all three passes belongs to the
//! TASK-0244 backend-common-style cleanup track — touching
//! `partition_workers.rs` would also touch the 9 TASK-0212 tests its
//! invariants pin, and a regression should bisect to that consolidation
//! commit, not to a partition-policy task.
//!
//! ## Why not share the row-band arithmetic between passes
//!
//! The row-band slicing math (~6 lines: `slice = len / n; lo + i * slice
//! .. lo + (i+1) * slice`) is byte-identical between
//! [`apply_partition_workers`](crate::passes::partition_workers::apply_partition_workers)
//! and [`apply_partition_rows`]; [`crate::passes::partition_blocks2d`]
//! adds a 2D variant of the same arithmetic on both axes. Extracting
//! the arithmetic across three different error types
//! ([`PartitionRowsError`], `PartitionError`,
//! `PartitionBlocks2dError`) is the smaller cleanup the TASK-0244 track
//! will consolidate. Deliberately NOT done in this cycle: the
//! arithmetic is six lines, and the error-type divergence would force a
//! refactor that touches all three passes plus their tests.
//!
//! ## Honest limitations (cycle 83+)
//!
//! - **Floor-with-spillover remainder policy (TASK-0262).** Same policy
//!   `partition_workers` enforces — first `(L % N)` workers get
//!   `floor(L / N) + 1` rows, the rest get `floor(L / N)`. The 05-stencil
//!   y-loop (1..15, length 14) on 4 workers yields bands 1..5, 5..9,
//!   9..12, 12..15 (4,4,3,3 rows respectively). The divisible case
//!   reduces to exact-split — no observational change for any
//!   pre-TASK-0262 cell.
//! - **Insufficient-work category error retained.** `L < N` (fewer rows
//!   than workers) is rejected as
//!   [`PartitionRowsError::InsufficientWork`]; spillover cannot give
//!   every worker one row in that case.
//! - **Halo inference is NOT part of this pass.** Row-band partitioning
//!   of a stencil produces WRONG output at the band boundaries because
//!   each worker reads neighbours it does not own. Halo widths must be
//!   inferred + Push/Wait pairs synthesised for the halo strips —
//!   that is TASK-0260 (sibling task). Until TASK-0260 lands, e2e
//!   cells exercising `partition=rows` on a stencil cannot be
//!   bit-identical to reference.bin. See the task brief for the
//!   carry-over: 05-stencil/distributed is restored to carry the
//!   directive but the cell remains `[[skip]]` for sibling reasons.
//! - **`partition=blocks2d` is the 2D-grid sibling.** TASK-0259
//!   (cycle 80) landed [`crate::passes::partition_blocks2d`], which
//!   uses the same outer-of-2D structural check as this pass but
//!   partitions BOTH axes across a 2D grid of workers. After
//!   TASK-0259, sched-lower no longer rejects any
//!   [`crate::sched::PartitionKind`] variant; the wildcard-free match
//!   in `lower_loop_option` is the exhaustiveness guard against future
//!   `PartitionKind` extensions (TASK-0410, cycle 237, removed the
//!   never-constructed `UnsupportedPartitionKind` error variant that
//!   used to claim this role).
//!
//! ## Pipeline placement
//!
//! Runs **after** [`crate::passes::partition_workers::apply_partition_workers`]
//! (driver order). Order between the two partition passes is purely
//! diagnostic: a single loop variable cannot carry BOTH `partition=rows`
//! and `partition=workers` because the schedule grammar accepts at most
//! one `partition=` option per loop (parser invariant; cross-checked by
//! `SchedLowerErrorKind::DuplicateLoopOption` at sched-lower). So the
//! sidecar entries written by the two passes target DISJOINT IterVar
//! keys and the order is observationally irrelevant.
//!
//! ## Determinism
//!
//! Same as `partition_workers`: the sidecar map is two nested
//! `BTreeMap`s keyed by `u64` ids, the pass walks the ACFG once with no
//! `HashMap` / `HashSet` iteration on a path that reaches the emitted
//! bytes. Worker subsets come from `BTreeSet<WorkerId>` (numeric order).
//! Same input ⇒ byte-identical sidecar.

use std::collections::{BTreeMap, BTreeSet};

use crate::acfg::{ACFGNode, ACFG};
use crate::event::{IterVar, WorkerId};
use crate::link::LinkedIR;
use crate::passes::common::{compute_partition_bands, PartitionBandError};
use crate::sched::{PartitionKind, ResolvedLoopOption};

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors produced by [`apply_partition_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionRowsError {
    /// The schedule referenced `partition=rows` for a loop variable
    /// that the algorithm doesn't loop over. The linker normally
    /// pre-rejects this; the variant exists so the pass fails closed
    /// on an inconsistently-constructed `(LinkedIR, ACFG)` pair (same
    /// invariant guard `PartitionError::UnknownLoopVar` carries for
    /// `partition_workers`).
    UnknownLoopVar { var: String },
    /// The loop carrying `partition=rows` is NOT the outer of a 2D
    /// nest — i.e. its body does not contain another `Repeat` node
    /// nested under it (possibly through `Sequence` wrappers). PRD
    /// §6.3.3: "`partition=rows` on a 1D iteration" is a bad
    /// combination rejected at compile time. The semantics of
    /// "rows" presuppose two iteration axes (outer = row index,
    /// inner = column index); applying it to a 1D loop is a
    /// category error.
    NotOuterOf2DNest { var: String },
    /// The loop's body is not placed on multiple workers, but the
    /// schedule asked for `partition=rows`. With one or zero workers
    /// the directive is meaningless. Same rationale as
    /// `PartitionError::NoMultiWorkerBody` (mirrors the
    /// `partition_workers` precedent).
    NoMultiWorkerBody { var: String, workers: usize },
    /// The outer loop's range length is strictly less than the worker
    /// count (TASK-0262). Even the floor-with-spillover remainder
    /// policy cannot give every worker at least one row; refuse
    /// compile rather than silently strip workers from the iteration.
    /// Mirrors `PartitionError::InsufficientWork` exactly.
    InsufficientWork {
        var: String,
        lo: i64,
        hi: i64,
        workers: usize,
    },
}

impl std::fmt::Display for PartitionRowsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartitionRowsError::UnknownLoopVar { var } => write!(
                f,
                "schedule has `loop {var} : partition=rows` but the ACFG contains no loop with \
                 variable `{var}` (linker-pass invariant violation)"
            ),
            PartitionRowsError::NotOuterOf2DNest { var } => write!(
                f,
                "loop `{var}` has `partition=rows` but is NOT the outer loop of a 2D nest \
                 (no inner Repeat in its body). `partition=rows` row-bands the outer axis of a 2D \
                 iteration and is a category error on a 1D loop (PRD §6.3.3). Either nest a \
                 second loop inside `{var}`, or use `partition=workers` (the compiler-choice \
                 1D variant), or omit the directive."
            ),
            PartitionRowsError::NoMultiWorkerBody { var, workers } => write!(
                f,
                "schedule has `loop {var} : partition=rows` but the inner loop body is placed \
                 on {workers} worker(s); `partition=rows` requires a multi-worker body to be \
                 meaningful (row-banding across one worker collapses to the source range)"
            ),
            PartitionRowsError::InsufficientWork {
                var,
                lo,
                hi,
                workers,
            } => write!(
                f,
                "loop `{var}` has range {lo}..{hi} (length {len}) which is strictly less than \
                 {workers} workers — the floor-with-spillover remainder policy (TASK-0262) \
                 cannot give every worker at least one row. Either reduce the worker count \
                 (placement) to at most {len}, or widen the outer loop range.",
                len = hi - lo
            ),
        }
    }
}

impl std::error::Error for PartitionRowsError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Walk `acfg` and populate
/// [`ACFG::partition_worker_ranges`](crate::acfg::ACFG::partition_worker_ranges)
/// for every loop carrying a `partition=rows` directive whose ACFG
/// shape is the outer of a 2D nest with a multi-worker body.
///
/// Pure: input ACFG is consumed, a new one with the sidecar populated
/// is returned. The tree itself is forwarded unchanged.
///
/// On any error, no partial sidecar is committed — the function
/// validates every directive up front before mutating the sidecar.
pub fn apply_partition_rows(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, PartitionRowsError> {
    // ---- 1. Collect every `partition=rows` directive. ----
    //
    // Iteration over `linked.sched.loops` (a `BTreeMap`) keeps the pass
    // deterministic.
    let mut partition_vars: BTreeSet<String> = BTreeSet::new();
    for (var, directive) in &linked.sched.loops {
        for opt in &directive.options {
            if matches!(opt, ResolvedLoopOption::Partition(PartitionKind::Rows)) {
                partition_vars.insert(var.clone());
            }
        }
    }

    if partition_vars.is_empty() {
        return Ok(acfg);
    }

    // ---- 2. Pre-validate. ----
    //
    // For each directive: resolve name → id, locate the Repeat, verify
    // it carries an inner Repeat (outer-of-2D structural check), collect
    // the INNER body's worker union (the row-band axis is the OUTER
    // loop; the workers are determined by the inner body that actually
    // executes per row-band), and validate divisibility.
    let mut to_record: Vec<(IterVar, std::ops::Range<i64>, BTreeSet<WorkerId>)> =
        Vec::with_capacity(partition_vars.len());
    for var in &partition_vars {
        let iter_var = match acfg.name_iter_vars.get(var) {
            Some(v) => *v,
            None => {
                return Err(PartitionRowsError::UnknownLoopVar { var: var.clone() });
            }
        };
        let Some((range, has_inner_repeat, body_workers)) = find_outer_of_2d(&acfg.root, iter_var)
        else {
            return Err(PartitionRowsError::UnknownLoopVar { var: var.clone() });
        };
        if !has_inner_repeat {
            return Err(PartitionRowsError::NotOuterOf2DNest { var: var.clone() });
        }
        if body_workers.len() < 2 {
            return Err(PartitionRowsError::NoMultiWorkerBody {
                var: var.clone(),
                workers: body_workers.len(),
            });
        }
        // TASK-0262: pre-validate via the shared band helper so the
        // L<N category error surfaces here BEFORE we commit any
        // sidecar entries. Mirrors partition_workers exactly.
        if let Err(e) = compute_partition_bands(range.start, range.end, body_workers.len()) {
            return Err(map_band_error(
                var,
                range.start,
                range.end,
                body_workers.len(),
                e,
            ));
        }
        to_record.push((iter_var, range, body_workers));
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
        // partition_rows does not consult or mutate the pipeline-depth
        // sidecar; forward verbatim.
        pipeline_depth_for_seq,
        // partition_rows does not consult or mutate the halo-widths
        // sidecar (TASK-0260); forward verbatim.
        halo_widths,
        // partition_rows does not consult or mutate the reuse-widths
        // sidecar (TASK-0261); forward verbatim.
        reuse_widths,
        // TASK-0264 cycle 113: partition_rows does not consult or
        // mutate partition_pairs / grid_shape_for_outer_iv (those are
        // populated by partition_blocks2d only); forward verbatim.
        partition_pairs,
        grid_shape_for_outer_iv,
    } = acfg;

    for (iter_var, range, body_workers) in to_record {
        // TASK-0262: shared floor-with-spillover band computation.
        // Pre-validated above; the only failure mode (L < N) was
        // caught at pre-validate, so an Err here would be a
        // pre-validate/commit divergence — fail loud.
        let bands = compute_partition_bands(range.start, range.end, body_workers.len())
            .expect("pre-validated; band helper must succeed");
        let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
        for (wid, (lo, hi)) in body_workers.iter().zip(bands.iter()) {
            per_worker.insert(*wid, *lo..*hi);
        }
        partition_worker_ranges.insert(iter_var, per_worker);
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

/// Find the (source range, has-inner-Repeat flag, inner-body worker
/// union) for the first Repeat in `node`'s subtree whose iter_var
/// equals `target`.
///
/// "has-inner-Repeat" is the outer-of-2D check: we look in the body
/// subtree for ANY `ACFGNode::Repeat` (possibly nested inside `Sequence`
/// wrappers). The match is conservative — we walk through `Sequence`
/// children and into nested `Sync` / `Xfer` siblings to spot an inner
/// loop. We do NOT descend through other `Repeat` bodies for the search
/// (an inner loop directly under the outer counts; loops two levels
/// deep also count — both are valid 2D-or-deeper nests).
///
/// Returns `None` if no Repeat with `iter_var == target` exists in the
/// subtree.
///
/// `pub(crate)` so the sibling pass [`crate::passes::partition_blocks2d`]
/// (TASK-0259) shares the same outer-of-2D structural check without
/// duplicating the walk. The byte-identical helper would otherwise be a
/// three-way duplication; lifting to `pub(crate)` is the smallest
/// consolidation that pays back at the second consumer.
pub(crate) fn find_outer_of_2d(
    node: &ACFGNode,
    target: IterVar,
) -> Option<(std::ops::Range<i64>, bool, BTreeSet<WorkerId>)> {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            ..
        } => {
            if *iter_var == target {
                let has_inner_repeat = contains_repeat(body);
                let mut workers = BTreeSet::new();
                collect_op_workers(body, &mut workers);
                Some((range.clone(), has_inner_repeat, workers))
            } else {
                find_outer_of_2d(body, target)
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                if let Some(r) = find_outer_of_2d(c, target) {
                    return Some(r);
                }
            }
            None
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => None,
    }
}

/// Map a [`PartitionBandError`] (from the shared band-computation
/// helper) onto this pass's typed error. Mirrors
/// `crate::passes::partition_workers::map_band_error` exactly — the
/// L<N case surfaces as `InsufficientWork`; the two other variants
/// also map to `InsufficientWork` to keep the error surface narrow at
/// this layer. `ZeroWorkers` is pre-empted by the worker-count check
/// above (so the band helper always sees `>= 2` workers), while
/// `InvalidRange` (hi < lo) is NOT gated upstream — the band helper
/// itself is the fail-closed backstop for an inverted range (see the
/// `partition_workers::map_band_error` docstring for the full rationale
/// on why `eval_const` / the link step do not catch it).
fn map_band_error(
    var: &str,
    lo: i64,
    hi: i64,
    workers: usize,
    e: PartitionBandError,
) -> PartitionRowsError {
    match e {
        PartitionBandError::InsufficientWork { .. }
        | PartitionBandError::ZeroWorkers
        | PartitionBandError::InvalidRange { .. } => PartitionRowsError::InsufficientWork {
            var: var.to_string(),
            lo,
            hi,
            workers,
        },
    }
}

/// Is there ANY `ACFGNode::Repeat` reachable in this subtree? Used for
/// the outer-of-2D structural check.
///
/// Walks through `Sequence` children and into a `Repeat` directly
/// (returning `true` the moment any Repeat is hit). Conservative —
/// extra-deep nests still satisfy the 2D-or-deeper precondition.
///
/// `pub(crate)` for the same TASK-0259 sharing reason as
/// [`find_outer_of_2d`].
pub(crate) fn contains_repeat(node: &ACFGNode) -> bool {
    match node {
        ACFGNode::Repeat { .. } => true,
        ACFGNode::Sequence(children) => children.iter().any(contains_repeat),
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => false,
    }
}

/// Union the `Operation.workers` sets across every Operation reachable
/// in this subtree. Read-only walk.
///
/// `pub(crate)` so [`crate::passes::partition_blocks2d`] (TASK-0259)
/// shares the helper instead of triple-duplicating it. The
/// byte-identical copy in [`crate::passes::partition_workers`] (TASK-
/// 0212) remains for now — that consolidation belongs to the TASK-0244
/// backend-common-style cleanup track.
pub(crate) fn collect_op_workers(node: &ACFGNode, out: &mut BTreeSet<WorkerId>) {
    match node {
        ACFGNode::Operation(op) => {
            for w in &op.workers {
                out.insert(*w);
            }
        }
        ACFGNode::Repeat { body, .. } => collect_op_workers(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_op_workers(c, out);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

// --------------------------------------------------------------------
// Unit tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acfg::{DataflowDag, DataflowEdge, Operation};
    use crate::event::{DataId, KernelId};

    /// Build an Operation node placed on the given worker set with a
    /// trivial single-edge dataflow.
    fn op_on(workers: &[u64]) -> ACFGNode {
        let mut ws: BTreeSet<WorkerId> = BTreeSet::new();
        for w in workers {
            ws.insert(WorkerId(*w));
        }
        ACFGNode::Operation(Operation {
            kernel: KernelId(0),
            workers: ws,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    vec![DataId(0)],
                    KernelId(0),
                    Some(DataId(1)),
                )],
            },
        })
    }

    #[test]
    fn contains_repeat_finds_direct_inner() {
        let body = ACFGNode::Sequence(vec![ACFGNode::Repeat {
            iter_var: IterVar(8),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![op_on(&[1])])),
            block_tag: None,
        }]);
        assert!(contains_repeat(&body));
    }

    #[test]
    fn contains_repeat_misses_1d_body() {
        let body = ACFGNode::Sequence(vec![op_on(&[1, 2])]);
        assert!(!contains_repeat(&body));
    }

    #[test]
    fn find_outer_of_2d_returns_inner_worker_union() {
        // Outer Repeat (y) over 0..16, body = Sequence [ Repeat (x) over
        // 0..8, body = Op on {w1, w2, w3} ]. find_outer_of_2d(y) must
        // return (0..16, has_inner=true, {w1,w2,w3}).
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(8),
            range: 0..8,
            body: Box::new(ACFGNode::Sequence(vec![op_on(&[1, 2, 3])])),
            block_tag: None,
        };
        let outer = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![inner])),
            block_tag: None,
        };
        let (range, has_inner, workers) = find_outer_of_2d(&outer, IterVar(7)).unwrap();
        assert_eq!(range, 0..16);
        assert!(has_inner);
        assert_eq!(
            workers,
            BTreeSet::from([WorkerId(1), WorkerId(2), WorkerId(3)])
        );
    }

    #[test]
    fn find_outer_of_2d_flags_1d_body() {
        // Outer Repeat (y) over 0..16, body = Sequence [ Op on {w1,w2} ]
        // (no inner Repeat). has_inner must be false; the worker union
        // still populates from the body Op.
        let outer = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![op_on(&[1, 2])])),
            block_tag: None,
        };
        let (range, has_inner, workers) = find_outer_of_2d(&outer, IterVar(7)).unwrap();
        assert_eq!(range, 0..16);
        assert!(!has_inner);
        assert_eq!(workers, BTreeSet::from([WorkerId(1), WorkerId(2)]));
    }
}
