//! Partition-workers loop transformation — TASK-0212.
//!
//! Consumes `ResolvedLoopOption::Partition(PartitionKind::Workers)` from
//! `linked.sched.loops` and records a per-(IterVar, WorkerId) loop-range
//! override on the ACFG's [`crate::acfg::ACFG::partition_worker_ranges`]
//! sidecar. The override is honoured at projection time by
//! [`crate::passes::petri_to_events::walk`] when it emits one
//! [`crate::event::Event::Loop`] per worker, so each worker sees its own
//! exclusive slice of the source range.
//!
//! ## Why a sidecar, not a tree rewrite
//!
//! Two cleaner-looking alternatives were considered and rejected:
//!
//! 1. **Replace the `ACFGNode::Repeat` with a per-worker `Sequence` of
//!    Repeats.** The participating workers are an `Operation.workers`
//!    set carried *inside* the body, not on the `Repeat` itself, so a
//!    tree rewrite would have to splice per-worker copies of the body
//!    and filter each copy's Operations to that worker. That is a
//!    lossy lowering: every existing pass that reads
//!    `Operation.workers` (`sync_inject::writers_in`,
//!    `transfer_inject`, `acfg_to_petri`) would then see a different
//!    *structural* program depending on how many workers a partition
//!    targets. The sidecar keeps the program shape stable and consumes
//!    the directive at exactly one site (projection).
//!
//! 2. **A per-`Repeat` payload field.** Same argument as
//!    [`crate::acfg::ACFG::inner_block_iter_vars`]: keeping
//!    `Repeat { iter_var, range, body, block_tag }` stable means every
//!    existing pattern-match keeps compiling. The cost is a small
//!    BTreeMap lookup at the one consumer site.
//!
//! ## Honest limitations (first cut)
//!
//! - **Exact-divisible split only.** `(hi - lo) % N != 0` is reported
//!   as a typed error (`PartitionError::NonDivisible`). A non-divisible
//!   policy (last-worker-gets-remainder, or remainder-tile sibling
//!   like `block_transform`'s trailing partial) is filed as the
//!   follow-up to this task per the task description.
//! - **1D partition axis only.** This pass consumes `partition=workers`
//!   directives. All three [`crate::sched::PartitionKind`] variants now
//!   have consumers: `partition=workers` (this pass, TASK-0212),
//!   `partition=rows` (TASK-0258, [`crate::passes::partition_rows`] —
//!   outer-of-2D row-band), and `partition=blocks2d` (TASK-0259,
//!   [`crate::passes::partition_blocks2d`] — 2D grid of worker blocks
//!   on a Repeat-of-Repeat). Sched-lower no longer rejects any
//!   `PartitionKind` variant.
//! - **No interaction with `block=`.** A user combining
//!   `loop n : block=N, partition=workers;` on the same loop would
//!   confuse: `block_transform` splits the loop into tile + inner with
//!   the inner reusing the original iter_var id. The partition pass
//!   would then partition the inner intra-tile range across workers,
//!   which is not the intended "split the OUTER source iteration
//!   across workers". Filed as a separate gap; the schedules in tree
//!   that use `partition=workers` (CNN batch_parallel) don't also
//!   block.
//! - **No `Operation.workers` rewrite.** The per-worker projection
//!   relies on `petri_to_events` already iterating each worker
//!   individually. Operations *inside* the partitioned body still
//!   advertise the full multi-worker set in `Operation.workers`; the
//!   range-override changes only how many iterations each worker fires
//!   the body, not which workers participate. This is the deliberate
//!   minimal change for the first cut. After TASK-0117
//!   (transfer-injection fan-out) lands, each per-worker iteration
//!   will also carry its own Push/Wait pair.
//!
//! ## Pipeline placement
//!
//! This pass runs **after**
//! [`crate::passes::block_transform::apply_block_transforms`] and
//! **before** [`crate::passes::sync_inject`] /
//! [`crate::passes::transfer_inject`]. Block-transform may have
//! synthesised tile / inner pairs; partition-workers reads the
//! post-block ACFG so the iter_var ids it records are the final ones.
//! Sync / transfer injection doesn't read the partition sidecar (the
//! sidecar is a projection-time concern, not an injection concern),
//! so order relative to them is purely for diagnostic clarity.
//!
//! ## Determinism
//!
//! The sidecar map is two nested `BTreeMap`s keyed by id (numeric
//! `u64`), so the projection iterates them in numeric order. The pass
//! itself walks the ACFG once with no `HashMap`/`HashSet` iteration;
//! the worker subset for each partition loop is read from the body's
//! `Operation.workers` (a `BTreeSet<WorkerId>`). Same input ⇒ same
//! sidecar, byte-identical.

use std::collections::{BTreeMap, BTreeSet};

use crate::acfg::{ACFGNode, ACFG};
use crate::event::{IterVar, WorkerId};
use crate::link::LinkedIR;
use crate::sched::{PartitionKind, ResolvedLoopOption};

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors produced by [`apply_partition_workers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionError {
    /// The schedule referenced `partition=workers` for a loop variable
    /// that the algorithm doesn't loop over. The linker normally
    /// pre-rejects this for unknown iter vars; the variant exists so
    /// the pass fails closed on an inconsistently-constructed
    /// `(LinkedIR, ACFG)` pair.
    UnknownLoopVar { var: String },
    /// The loop's range length is not evenly divisible by the worker
    /// count. First-cut policy is to refuse compile; a remainder
    /// policy (last-worker-gets-tail, or trailing partial sibling) is
    /// a follow-up filed by the task description.
    NonDivisible {
        var: String,
        lo: i64,
        hi: i64,
        workers: usize,
    },
    /// The loop's body is not placed on multiple workers, but the
    /// schedule asked for `partition=workers`. With one or zero
    /// workers the directive is meaningless — silent acceptance would
    /// later become a load-bearing assumption with no diagnostic when
    /// it ceases to hold. Fail closed.
    NoMultiWorkerBody { var: String, workers: usize },
}

impl std::fmt::Display for PartitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartitionError::UnknownLoopVar { var } => write!(
                f,
                "schedule has `loop {var} : partition=workers` but the ACFG contains no loop \
                 with variable `{var}` (linker-pass invariant violation)"
            ),
            PartitionError::NonDivisible {
                var,
                lo,
                hi,
                workers,
            } => write!(
                f,
                "loop `{var}` has range {lo}..{hi} (length {len}) not evenly divisible across \
                 {workers} workers (TASK-0212 first cut: exact-divisible only; a remainder \
                 policy is a follow-up)",
                len = hi - lo
            ),
            PartitionError::NoMultiWorkerBody { var, workers } => write!(
                f,
                "schedule has `loop {var} : partition=workers` but the loop body is placed on \
                 {workers} worker(s); `partition=workers` requires a multi-worker body to be \
                 meaningful"
            ),
        }
    }
}

impl std::error::Error for PartitionError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Walk `acfg` and populate
/// [`ACFG::partition_worker_ranges`](crate::acfg::ACFG::partition_worker_ranges)
/// for every loop carrying a `partition=workers` directive.
///
/// Pure: input ACFG is consumed, a new one with the sidecar populated
/// is returned. The tree itself is forwarded unchanged.
///
/// On any error, no partial sidecar is committed — the function
/// validates every directive up front before mutating the sidecar.
pub fn apply_partition_workers(
    linked: &LinkedIR,
    acfg: ACFG,
) -> Result<ACFG, PartitionError> {
    // ---- 1. Collect every `partition=workers` directive. ----
    //
    // `linked.sched.loops` is a `BTreeMap<var_name, ResolvedLoopDirective>`;
    // sorted iteration keeps the pass deterministic.
    let mut partition_vars: BTreeSet<String> = BTreeSet::new();
    for (var, directive) in &linked.sched.loops {
        for opt in &directive.options {
            if matches!(
                opt,
                ResolvedLoopOption::Partition(PartitionKind::Workers)
            ) {
                partition_vars.insert(var.clone());
            }
        }
    }

    if partition_vars.is_empty() {
        return Ok(acfg);
    }

    // ---- 2. Pre-validate (resolve names to ids, find loops, find
    // their body's worker entity). ----
    //
    // We collect (iter_var_id, source_range, body_workers) first so a
    // failure on the last directive doesn't leave the sidecar
    // half-populated. The errors carry the source name (the form the
    // user wrote), not the IterVar id.
    let mut to_record: Vec<(String, IterVar, std::ops::Range<i64>, BTreeSet<WorkerId>)> =
        Vec::with_capacity(partition_vars.len());
    for var in &partition_vars {
        let iter_var = match acfg.name_iter_vars.get(var) {
            Some(v) => *v,
            None => {
                return Err(PartitionError::UnknownLoopVar {
                    var: var.clone(),
                });
            }
        };
        let (range, body_workers) = match find_loop(&acfg.root, iter_var) {
            Some(found) => found,
            None => {
                return Err(PartitionError::UnknownLoopVar {
                    var: var.clone(),
                });
            }
        };
        if body_workers.len() < 2 {
            return Err(PartitionError::NoMultiWorkerBody {
                var: var.clone(),
                workers: body_workers.len(),
            });
        }
        let len = range.end - range.start;
        let n = body_workers.len() as i64;
        if len % n != 0 {
            return Err(PartitionError::NonDivisible {
                var: var.clone(),
                lo: range.start,
                hi: range.end,
                workers: body_workers.len(),
            });
        }
        to_record.push((var.clone(), iter_var, range, body_workers));
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
        // TASK-0134: partition_workers does not consult or mutate the
        // pipeline-depth sidecar; forward verbatim.
        pipeline_depth_for_seq,
    } = acfg;

    for (_var, iter_var, range, body_workers) in to_record {
        let len = range.end - range.start;
        let n = body_workers.len() as i64;
        let slice = len / n; // already validated divisible
        let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
        // `BTreeSet<WorkerId>` iterates in numeric order; assign the
        // first worker the first slice, etc. This is a deterministic
        // function of (range, body_workers).
        for (i, wid) in body_workers.iter().enumerate() {
            let lo = range.start + (i as i64) * slice;
            let hi = lo + slice;
            per_worker.insert(*wid, lo..hi);
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
    })
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Find the (source range, body's worker set) for the first Repeat in
/// `node`'s subtree whose iter_var equals `target`.
///
/// "Body's worker set" is the union of `Operation.workers` across every
/// `ACFGNode::Operation` reachable through the body subtree — the
/// natural notion of "which workers participate in this loop". A loop
/// whose body has no Operation contributes an empty set (caught
/// upstream as `NoMultiWorkerBody`).
fn find_loop(
    node: &ACFGNode,
    target: IterVar,
) -> Option<(std::ops::Range<i64>, BTreeSet<WorkerId>)> {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            ..
        } => {
            if *iter_var == target {
                let mut workers = BTreeSet::new();
                collect_op_workers(body, &mut workers);
                Some((range.clone(), workers))
            } else {
                find_loop(body, target)
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                if let Some(r) = find_loop(c, target) {
                    return Some(r);
                }
            }
            None
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => None,
    }
}

/// Union the `Operation.workers` sets across every Operation reachable
/// in this subtree. Read-only walk.
fn collect_op_workers(node: &ACFGNode, out: &mut BTreeSet<WorkerId>) {
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
    /// trivial single-edge dataflow. The kernel/data ids are arbitrary
    /// constants — the partition pass reads only `Operation.workers`.
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
    fn find_loop_returns_body_worker_union() {
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![
                op_on(&[1, 2]),
                op_on(&[2, 3]),
            ])),
            block_tag: None,
        };
        let (range, workers) = find_loop(&inner, IterVar(7)).unwrap();
        assert_eq!(range, 0..16);
        assert_eq!(
            workers,
            BTreeSet::from([WorkerId(1), WorkerId(2), WorkerId(3)])
        );
    }

    #[test]
    fn find_loop_returns_none_on_missing() {
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![op_on(&[0])])),
            block_tag: None,
        };
        assert!(find_loop(&inner, IterVar(99)).is_none());
    }
}
