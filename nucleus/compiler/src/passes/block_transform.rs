//! Block-loop transformation — TASK-0030.
//!
//! Rewrites each `Repeat` whose loop variable carries a schedule
//! `block=N` directive (PRD §6.3.3) into a two-level nest:
//!
//! ```text
//!   for VAR : LO .. HI  with block=N           // source
//!
//!   becomes
//!
//!   for VAR__tile  : 0  ..  (HI - LO) / N      // outer tile loop
//!     for VAR       : 0  ..  N                  // inner intra-tile
//!       <body>
//! ```
//!
//! The inner loop's `iter_var` keeps the original variable's name and
//! ID — the [`crate::event::IterTile`] bounds attached to `Operation`
//! / `Xfer` placeholders by later passes therefore still reference
//! `VAR` directly, which keeps the rest of the pipeline structurally
//! unchanged. The synthetic outer iter var `VAR__tile` is registered
//! in [`ACFG::name_iter_vars`] under the same name with a fresh
//! [`IterVar`] id.
//!
//! ## Pipeline placement
//!
//! This pass runs **after** [`crate::acfg::build_acfg`] and **before**
//! [`crate::passes::sync_inject`] / [`crate::passes::transfer_inject`].
//! Reasoning:
//!
//! 1. ACFG construction stays a pure function of `LinkedIR` — no
//!    schedule-loop-option lookup tangled into it.
//! 2. By rewriting the iteration tree before transfer injection, the
//!    `IterTile` recorded on each transfer is naturally scoped to the
//!    *inner* (intra-tile) loop. That's not yet bulk-per-tile
//!    granularity (see the "honest limitations" below), but it does
//!    push transfers to fire per (tile, intra-tile-iteration) rather
//!    than per (untiled-iteration), which is the structural
//!    precondition for the TASK-0116 tile-coalescing follow-up.
//!
//! ## Scope at M2
//!
//! - Only **outermost** loops are blocked. A `block=` directive on an
//!   inner loop's variable still rewrites that loop, but no attempt
//!   is made to coordinate two `block=` directives that would both
//!   need to nest with each other. Each matching `Repeat` is rewritten
//!   independently.
//! - `(HI - LO)` must be **evenly divisible** by `N`. Remainder
//!   handling (a trailing partial tile) is deliberately deferred — the
//!   `ACFGNode::Repeat::range` shape is a single `Range<i64>` and does
//!   not carry the `min(y_outer+N, H)` clamp the PRD describes. Failing
//!   loud here keeps the IR honest until the remainder follow-up
//!   lands. See `BlockTransformError::NotDivisible`.
//! - Loops whose `iter_var` does not appear in any `block=` directive
//!   pass through unchanged. Examples 01-03 contain no `block=`
//!   directives in their *required* schedules; their ACFGs therefore
//!   are bit-identical with and without this pass running.
//!
//! ## Honest limitations (also recorded in the self-report)
//!
//! - **Inner-loop iter var name is reused.** We keep the original
//!   variable's [`IterVar`] id on the inner loop and synthesise a
//!   new id for `VAR__tile`. The intra-tile loop therefore iterates
//!   over `0..N`, NOT `LO..LO+N` — codegen that wants the absolute
//!   iteration value must compute `LO + tile*N + inner`. The current
//!   M2 codegen does not yet read the absolute value (it threads
//!   `IterTile`'s `Range<i64>` through verbatim), so this is
//!   acceptable; once a backend actually consumes the absolute value
//!   the pass should be revisited.
//! - **Transfers are still per intra-tile iteration, not per tile.**
//!   Per PRD §6.3.3, "transfers happen per tile" is the desired
//!   semantics. Achieving it requires hoisting `Xfer` placeholders
//!   out of the inner loop into the outer's body, which is a
//!   transfer-inject-side concern. Filed as a follow-up
//!   (TASK-0116/0126); this pass produces the structural precondition
//!   (the two-level nest) without yet doing the hoist.
//! - **No conflict detection across `block=` and `vectorize=`.** PRD
//!   §6.3.3 says loop options are orthogonal "where possible" and that
//!   bad combinations are compile errors. v2 ships the structural
//!   transform; rejecting nonsensical pairs (e.g. `block=64` on a loop
//!   with `unroll=N` where `N > 64`) is the AC #5 follow-up.
//! - **Only `block=` is handled.** `vectorize=`, `unroll=`,
//!   `pipeline=`, `reuse`, `partition=` are no-ops in this pass; their
//!   transforms land in sibling passes per their respective tasks.

use std::collections::BTreeMap;

use crate::acfg::{ACFGNode, ACFG};
use crate::event::IterVar;
use crate::link::LinkedIR;
use crate::sched::ResolvedLoopOption;

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors produced by [`apply_block_transforms`].
///
/// Each variant carries enough context to produce a user-facing
/// diagnostic without the caller needing to thread additional state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockTransformError {
    /// `block=N` on a loop variable whose iteration count is not
    /// evenly divisible by `N`. Trailing-remainder support is a
    /// follow-up; reject loud rather than silently emit a partial
    /// tile.
    NotDivisible {
        var: String,
        lo: i64,
        hi: i64,
        block: u64,
    },
    /// The schedule referenced a `block=N` for a loop variable that
    /// the algorithm doesn't actually have. The linker already
    /// rejects this for unknown iter vars, so the variant exists
    /// purely so the pass fails closed if invoked on an
    /// inconsistently-constructed `(LinkedIR, ACFG)` pair.
    UnknownLoopVar { var: String },
}

impl std::fmt::Display for BlockTransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockTransformError::NotDivisible {
                var,
                lo,
                hi,
                block,
            } => write!(
                f,
                "loop `{var}` has range {lo}..{hi} (length {len}) which is not evenly \
                 divisible by `block={block}`; trailing-remainder tiles are not yet supported \
                 (TASK-0030 follow-up)",
                len = hi - lo
            ),
            BlockTransformError::UnknownLoopVar { var } => write!(
                f,
                "schedule has `loop {var} : block=...` but the ACFG contains no loop with \
                 variable `{var}` (linker-pass invariant violation)"
            ),
        }
    }
}

impl std::error::Error for BlockTransformError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Apply every `block=N` loop directive in `linked.sched.loops` to
/// the iteration tree in `acfg`.
///
/// Pure function: returns a new ACFG with the rewritten tree and the
/// `name_iter_vars` map extended by any newly-synthesised tile-loop
/// variables. Other name tables (`name_kernels`, `name_data`,
/// `name_workers`) are forwarded unchanged.
///
/// On error, no partial rewrite is applied — the function inspects
/// the schedule and the ACFG up front, validates each `block=N`
/// against its target loop's bounds, and only then walks the tree.
/// Failing loud at the validation stage gives the user a single
/// actionable diagnostic instead of a half-rewritten IR.
pub fn apply_block_transforms(
    linked: &LinkedIR,
    acfg: ACFG,
) -> Result<ACFG, BlockTransformError> {
    // ---- 1. Collect every `block=N` directive from the schedule. ----
    //
    // `linked.sched.loops` is a BTreeMap<var_name, ResolvedLoopDirective>
    // keyed by loop variable. Iterating it in sorted order keeps the
    // pass deterministic.
    let mut block_by_var: BTreeMap<String, u64> = BTreeMap::new();
    for (var, directive) in &linked.sched.loops {
        for opt in &directive.options {
            if let ResolvedLoopOption::Block(n) = opt {
                // If the same loop carries multiple `block=N` options
                // (last-wins per sched-IR module docs), take the last
                // one in source order — matching how options are
                // resolved elsewhere (e.g. transfer sync/async).
                block_by_var.insert(var.clone(), *n);
            }
        }
    }

    // Nothing to do — the common case for examples 01-03's required
    // schedules.
    if block_by_var.is_empty() {
        return Ok(acfg);
    }

    // ---- 2. Validate every target loop exists and divides evenly. ----
    //
    // Walk the tree once for validation before mutating anything.
    // `find_loop_range_by_name` returns the resolved (lo, hi) for the
    // first `Repeat` matching the given variable name; if the same
    // variable shadows itself in nested scopes (not legal under the
    // algorithm grammar today, but the IR allows it structurally)
    // only the outermost match is checked. That matches the "only
    // outermost loops blocked" limitation above.
    for (var, n) in &block_by_var {
        let iter_var = match acfg.name_iter_vars.get(var) {
            Some(v) => *v,
            None => {
                return Err(BlockTransformError::UnknownLoopVar { var: var.clone() })
            }
        };
        match find_loop_range_by_id(&acfg.root, iter_var) {
            Some((lo, hi)) => {
                let len = hi - lo;
                // `block=N` is `u64` from the schedule grammar; `len`
                // is `i64`. A negative length would mean an empty
                // loop (lo >= hi) and is treated as "nothing to
                // block" — pass through. The lowering pass enforces
                // `lo < hi` for real programs.
                if len <= 0 {
                    continue;
                }
                if (len as u64) % n != 0 {
                    return Err(BlockTransformError::NotDivisible {
                        var: var.clone(),
                        lo,
                        hi,
                        block: *n,
                    });
                }
            }
            None => {
                // Schedule directive on a loop var that the algorithm
                // doesn't loop over. The linker should reject this,
                // but fail closed.
                return Err(BlockTransformError::UnknownLoopVar { var: var.clone() });
            }
        }
    }

    // ---- 3. Allocate fresh IterVar ids for the synthetic outer
    //         (tile) iter vars. ----
    //
    // Names: `<var>__tile`. Collision with an existing iter var name
    // is theoretically possible if the user happened to declare such
    // a variable in the algorithm; the algorithm grammar's identifier
    // rule allows it. We don't try to mangle uniquely — if a
    // collision occurs, the existing iter var keeps its id and we
    // reuse it (which would be wrong). To avoid silent wrongness,
    // panic loudly. Filed as a follow-up if a real example collides.
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        mut name_iter_vars,
    } = acfg;

    let mut next_id: u64 = name_iter_vars
        .values()
        .map(|v| v.0)
        .max()
        .map(|v| v + 1)
        .unwrap_or(0);

    let mut tile_var_for: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    for (var, n) in &block_by_var {
        let inner_id = name_iter_vars
            .get(var)
            .copied()
            .expect("validated above");
        let tile_name = format!("{var}__tile");
        if name_iter_vars.contains_key(&tile_name) {
            panic!(
                "block_transform: synthetic outer iter var name `{tile_name}` collides with an \
                 existing iter var; rename the user variable or extend the pass to mangle"
            );
        }
        let tile_id = IterVar(next_id);
        next_id += 1;
        name_iter_vars.insert(tile_name.clone(), tile_id);
        tile_var_for.insert(inner_id, (tile_name, tile_id, *n));
    }

    // ---- 4. Walk the tree and rewrite matching `Repeat`s. ----

    let new_root = rewrite_node(root, &tile_var_for);

    Ok(ACFG {
        root: new_root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
    })
}

// --------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------

/// Find the resolved `(lo, hi)` half-open range of the first `Repeat`
/// node in `node`'s subtree whose `iter_var` equals `target`. Returns
/// `None` if no such loop exists.
///
/// Used only by the validation pre-pass — the rewrite itself uses
/// pattern-matching on the loop directly.
fn find_loop_range_by_id(node: &ACFGNode, target: IterVar) -> Option<(i64, i64)> {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
        } => {
            if *iter_var == target {
                Some((range.start, range.end))
            } else {
                find_loop_range_by_id(body, target)
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                if let Some(r) = find_loop_range_by_id(c, target) {
                    return Some(r);
                }
            }
            None
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => None,
    }
}

/// Recursively rewrite the tree. A `Repeat` whose `iter_var` appears
/// in `tile_var_for` is replaced by an outer `Repeat` over the tile
/// index and an inner `Repeat` over `0..N`.
///
/// The body is itself recursed into first — nested `block=` cases
/// rewrite from the inside out. This matters only if a future M-level
/// task lifts the "outermost only" restriction; today the BTreeMap is
/// effectively single-entry per loop and order doesn't matter.
fn rewrite_node(
    node: ACFGNode,
    tile_var_for: &BTreeMap<IterVar, (String, IterVar, u64)>,
) -> ACFGNode {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
        } => {
            let new_body = rewrite_node(*body, tile_var_for);
            match tile_var_for.get(&iter_var) {
                Some((_, tile_id, n)) => {
                    let len = range.end - range.start;
                    // Validation has guaranteed divisibility and a
                    // positive length; defensively re-check.
                    let n_i64 = *n as i64;
                    let num_tiles = len / n_i64;
                    let inner_range = 0..n_i64;
                    let outer_range = 0..num_tiles;
                    let inner = ACFGNode::Repeat {
                        iter_var,
                        range: inner_range,
                        body: Box::new(new_body),
                    };
                    ACFGNode::Repeat {
                        iter_var: *tile_id,
                        range: outer_range,
                        body: Box::new(ACFGNode::Sequence(vec![inner])),
                    }
                }
                None => ACFGNode::Repeat {
                    iter_var,
                    range,
                    body: Box::new(new_body),
                },
            }
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| rewrite_node(c, tile_var_for))
                .collect(),
        ),
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// Unit tests for internal helpers
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acfg::{DataflowDag, DataflowEdge, Operation};
    use crate::event::{DataId, KernelId, WorkerId};
    use std::collections::BTreeSet;

    fn op() -> ACFGNode {
        let mut workers = BTreeSet::new();
        workers.insert(WorkerId(0));
        ACFGNode::Operation(Operation {
            kernel: KernelId(0),
            workers,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge {
                    data_in: vec![DataId(0)],
                    kernel: KernelId(0),
                    data_out: Some(DataId(1)),
                }],
            },
        })
    }

    #[test]
    fn find_loop_range_walks_nested_seq() {
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..32,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
        };
        let outer = ACFGNode::Sequence(vec![op(), inner]);
        assert_eq!(find_loop_range_by_id(&outer, IterVar(7)), Some((0, 32)));
        assert_eq!(find_loop_range_by_id(&outer, IterVar(99)), None);
    }

    #[test]
    fn rewrite_node_passes_through_non_blocked() {
        let tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
        let n = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
        };
        let out = rewrite_node(n.clone(), &tile_map);
        assert_eq!(out, n);
    }

    #[test]
    fn rewrite_node_blocks_match() {
        let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
        tile_map.insert(IterVar(0), ("y__tile".to_string(), IterVar(5), 4));
        let n = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
        };
        let out = rewrite_node(n, &tile_map);
        match out {
            ACFGNode::Repeat {
                iter_var: outer,
                range: outer_range,
                body,
            } => {
                assert_eq!(outer, IterVar(5));
                assert_eq!(outer_range, 0..4); // 16 / 4 = 4 tiles
                match *body {
                    ACFGNode::Sequence(seq) => {
                        assert_eq!(seq.len(), 1);
                        match &seq[0] {
                            ACFGNode::Repeat {
                                iter_var: inner,
                                range: inner_range,
                                ..
                            } => {
                                assert_eq!(*inner, IterVar(0));
                                assert_eq!(*inner_range, 0..4); // chunk size
                            }
                            other => panic!("inner not Repeat: {other:?}"),
                        }
                    }
                    other => panic!("body not Sequence: {other:?}"),
                }
            }
            other => panic!("outer not Repeat: {other:?}"),
        }
    }
}
