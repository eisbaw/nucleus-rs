//! Block-loop transformation — TASK-0030.
//!
//! Error convention (decision-0003): this pass is on the
//! typed-`Result` side — it returns
//! `Result<_, BlockTransformError>` (a `pub enum` whose variants
//! carry diagnostic context) rather than `panic!`ing, so any failure
//! is surfaced by the driver as a clean `nucleus: error:` line. (Its
//! one live variant, `UnknownLoopVar`, is a fail-closed guard the
//! linker normally pre-rejects; `NotDivisible` is retired per
//! TASK-0142 — see those variants' docs.)
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
//! - `(HI - LO)` need **not** be evenly divisible by `N`. When it is
//!   not, the rewrite emits the full-tile nest *plus* one explicit
//!   trailing partial tile (TASK-0142). Crucially this is done with
//!   **only static-range `Repeat`/`Sequence` nodes** — the
//!   `ACFGNode::Repeat::range` shape stays a single `Range<i64>` and
//!   no new IR variant is introduced. The PRD §6.3.3
//!   `min(y_outer+N, H)` clamp is realised structurally: instead of a
//!   dynamic inner upper bound, the trailing partial tile is a
//!   separate `Repeat` whose static length is exactly the remainder.
//!   See `rewrite_node` and the `tile_nest` helper.
//!
//!   Why static decomposition and not a dynamic
//!   `ACFGNode::Repeat` upper bound: `acfg_to_petri` /
//!   `petri_to_events` unroll every `Repeat` by
//!   `range.end - range.start`, assuming static bounds, and
//!   boundedness / deadlock consume that unrolled net. A dynamic
//!   upper bound (a function of an outer iter var) would ripple into
//!   all of those passes plus the backend plus determinism. The
//!   schedules that need remainder tiles today are single-worker
//!   (05-stencil/blocked), so the trailing partial tile is fully
//!   expressible with existing static nodes and the downstream
//!   passes stay correct *by construction* (total unrolled firing
//!   count is `num_full * N + remainder == HI - LO`, identical to the
//!   untiled loop). `BlockTransformError::NotDivisible` is therefore
//!   retired (kept only as a now-unconstructed variant for ABI
//!   stability of the error enum).
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
//!   Specifically: `pipeline=D` is consumed by
//!   `transfer_inject::annotate_pipeline_depth_for_seq` (post-pass) and
//!   `acfg_to_petri` (sets buffer-place `initial_marking=D`); see
//!   [`crate::acfg::ACFG::pipeline_depth_for_seq`] (TASK-0134).
//!   `partition=workers` is consumed by `partition_workers` (TASK-0212).
//!   Combining `block=N` with `pipeline=D` on the same loop is a
//!   silent under-tested area (TASK-0215).

use std::collections::BTreeMap;

use crate::acfg::{ACFGNode, ACFG};
use crate::event::{BlockTag, IterVar};
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
    /// evenly divisible by `N`.
    ///
    /// **Retired (TASK-0142).** Non-divisible ranges are now
    /// supported: the rewrite emits the full-tile nest plus one
    /// trailing partial tile. This variant is intentionally kept (no
    /// longer constructed) so the public error enum's shape stays
    /// stable for any caller pattern-matching on it; the
    /// `#[allow(dead_code)]` documents that the fields are no longer
    /// produced.
    #[allow(dead_code)]
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
            // Retired (TASK-0142): never constructed. The message is
            // kept accurate in case a future caller reintroduces it.
            BlockTransformError::NotDivisible { var, lo, hi, block } => write!(
                f,
                "loop `{var}` has range {lo}..{hi} (length {len}) not evenly divisible by \
                 `block={block}` (this error is retired — non-divisible ranges are now \
                 rewritten into a trailing partial tile, TASK-0142)",
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
/// On error, no partial rewrite is applied — the function validates
/// up front that every `block=N` target loop exists in the ACFG, and
/// only then walks the tree. (Divisibility is no longer an error,
/// TASK-0142: a non-divisible range is rewritten into the full-tile
/// nest plus one trailing partial tile.) Failing loud at the
/// validation stage gives the user a single actionable diagnostic
/// instead of a half-rewritten IR.
pub fn apply_block_transforms(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, BlockTransformError> {
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

    // ---- 2. Validate every target loop exists. ----
    //
    // Walk the tree once for validation before mutating anything.
    // `find_loop_range_by_id` returns the resolved (lo, hi) for the
    // first `Repeat` matching the given variable id; if the same
    // variable shadows itself in nested scopes (not legal under the
    // algorithm grammar today, but the IR allows it structurally)
    // only the outermost match is checked. That matches the "only
    // outermost loops blocked" limitation above.
    //
    // Divisibility is NO LONGER checked here (TASK-0142): a
    // non-divisible range is rewritten into the full-tile nest plus a
    // trailing partial tile in step 4. Only the existence of the
    // target loop is validated, so the pass still fails closed on an
    // inconsistently-constructed `(LinkedIR, ACFG)` pair.
    for var in block_by_var.keys() {
        let iter_var = match acfg.name_iter_vars.get(var) {
            Some(v) => *v,
            None => return Err(BlockTransformError::UnknownLoopVar { var: var.clone() }),
        };
        if find_loop_range_by_id(&acfg.root, iter_var).is_none() {
            // Schedule directive on a loop var that the algorithm
            // doesn't loop over. The linker should reject this, but
            // fail closed.
            return Err(BlockTransformError::UnknownLoopVar { var: var.clone() });
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
        mut inner_block_iter_vars,
        partition_worker_ranges,
        // TASK-0134: block_transform runs BEFORE transfer_inject so
        // this map is always empty here; we forward verbatim.
        // block_transform splits one iter_var into two (outer tile +
        // inner intra-tile) but does NOT create any Push/Wait pairs —
        // those are transfer_inject's product, so there is no
        // SeqTag remapping to do here.
        pipeline_depth_for_seq,
    } = acfg;

    let mut next_id: u64 = name_iter_vars
        .values()
        .map(|v| v.0)
        .max()
        .map(|v| v + 1)
        .unwrap_or(0);

    let mut tile_var_for: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    for (var, n) in &block_by_var {
        let inner_id = name_iter_vars.get(var).copied().expect("validated above");
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
        // Mark the (reused) inner-loop iter var so transfer_inject
        // (TASK-0143) can hoist Push/Wait placeholders out of the
        // intra-tile body up to per-tile granularity.
        inner_block_iter_vars.insert(inner_id);
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
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
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
            ..
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

/// Build one `(outer tile-loop, inner intra-tile-loop)` nest.
///
/// Shape: `Repeat(tile_id, outer_range) { Sequence[ Repeat(inner_id,
/// 0..inner_len) { body } ] }`. This is the canonical block-tile
/// shape every downstream consumer expects (the per-tile hoist looks
/// for `tile -> Sequence -> inner`). Both the full-tile nest and the
/// trailing partial tile go through here so the two are structurally
/// identical apart from their static ranges.
///
/// `body` is cloned because a non-divisible loop produces two nests
/// (full + partial) that each need their own copy of the body. The
/// clone is a deep `ACFGNode` clone — acceptable: block-transform runs
/// once per compile on a tree whose size is bounded by the source
/// program, not by iteration counts (unrolling happens later in
/// `acfg_to_petri`).
fn tile_nest(
    tile_id: IterVar,
    outer_range: std::ops::Range<i64>,
    inner_id: IterVar,
    inner_len: i64,
    body: &ACFGNode,
    inner_tag: BlockTag,
) -> ACFGNode {
    // The inner (intra-tile) loop reuses the SOURCE iter var id and
    // iterates `0..inner_len` (NOT `LO..LO+N`). Codegen must rebind it
    // to the absolute source value; `inner_tag` carries exactly the
    // per-occurrence facts to do so (TASK-0180) — full vs partial,
    // block width `N`, and `num_full` for the partial base offset.
    let inner = ACFGNode::Repeat {
        iter_var: inner_id,
        range: 0..inner_len,
        body: Box::new(body.clone()),
        block_tag: Some(inner_tag),
    };
    // The synthesised tile loop's variable never appears in a body
    // index (block_transform only renames the source loop into a
    // tile/inner pair); it needs no rebinding tag.
    ACFGNode::Repeat {
        iter_var: tile_id,
        range: outer_range,
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
    }
}

/// Recursively rewrite the tree. A `Repeat` whose `iter_var` appears
/// in `tile_var_for` is replaced by an outer `Repeat` over the tile
/// index and an inner `Repeat` over `0..N` (plus a trailing partial
/// tile when the range is not evenly divisible by `N`).
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
            block_tag,
        } => {
            let new_body = rewrite_node(*body, tile_var_for);
            match tile_var_for.get(&iter_var) {
                Some((_, tile_id, n)) => {
                    let len = range.end - range.start;
                    let n_i64 = *n as i64;
                    // An empty / malformed loop (lo >= hi) has nothing
                    // to tile — pass it through unchanged so the
                    // unroll-by-length downstream passes see exactly
                    // the same (zero-firing) structure they would
                    // without the directive.
                    if len <= 0 {
                        return ACFGNode::Repeat {
                            iter_var,
                            range,
                            body: Box::new(new_body),
                            // Degenerate loop passed through unchanged:
                            // preserve whatever tag it already carried
                            // (it has none unless a future nested-block
                            // pass re-enters; never invent one here).
                            block_tag,
                        };
                    }
                    let num_full = len / n_i64; // whole tiles of width N
                    let rem = len % n_i64; // trailing partial tile width

                    // Each tile is a `(outer tile-loop, inner
                    // intra-tile-loop)` nest. The full tiles share one
                    // outer `Repeat` over `0..num_full` with an inner
                    // `Repeat` over `0..N`. The trailing partial tile
                    // (when `rem != 0`) is its OWN nest: a degenerate
                    // outer `Repeat` over `0..1` wrapping an inner
                    // `Repeat` over `0..rem`. Keeping the partial tile
                    // shaped like a tile (rather than a bare inner
                    // loop) means:
                    //   * the per-tile transfer-hoist
                    //     (`inner_block_iter_vars`, TASK-0143) sees a
                    //     uniform `tile -> per-tile-seq -> inner`
                    //     structure for the tail too, and
                    //   * the outer tile-loop iteration count is
                    //     `num_full + 1`, matching the PRD §6.3.3
                    //     "ceil(len / N)" tile count.
                    //
                    // All ranges are static `Range<i64>`; the total
                    // unrolled firing count is
                    // `num_full * N + rem == len`, identical to the
                    // untiled loop, so `acfg_to_petri` /
                    // `petri_to_events` / boundedness / deadlock /
                    // determinism are unaffected by construction.
                    // Per-occurrence rebinding facts (TASK-0180). The
                    // full nest rebinds `LO + tile*N + inner` (its tile
                    // loop runs `0..num_full`); the trailing partial
                    // tile rebinds `LO + num_full*N + inner` (its tile
                    // loop is `0..1`, so `tile*N` would wrongly be 0).
                    // Both carry the SAME block width `N` (the absolute
                    // stride) and `num_full`; `is_partial` selects the
                    // base. The backend reads ONLY these — no global
                    // EventList occurrence count (the root-cause fix:
                    // a loop-var name reused across N divisible passes
                    // now rebinds each occurrence instead of being
                    // dropped by the old `counts==1` guard).
                    let full_tag = BlockTag {
                        block_n: n_i64,
                        num_full,
                        is_partial: false,
                    };
                    let partial_tag = BlockTag {
                        block_n: n_i64,
                        num_full,
                        is_partial: true,
                    };
                    let mut tiles: Vec<ACFGNode> = Vec::with_capacity(2);
                    if num_full > 0 {
                        tiles.push(tile_nest(
                            *tile_id, 0..num_full, iter_var, n_i64, &new_body, full_tag,
                        ));
                    }
                    if rem != 0 {
                        tiles.push(tile_nest(
                            *tile_id, 0..1, iter_var, rem, &new_body, partial_tag,
                        ));
                    }

                    match tiles.len() {
                        // `rem == 0` (the historical divisible path):
                        // emit EXACTLY the previous single-nest shape
                        // so existing snapshots / hoist tests /
                        // 07-matmul-blocked stay byte-identical.
                        1 => tiles.into_iter().next().expect("len==1"),
                        // Full tiles followed by the trailing partial
                        // tile, sequenced.
                        _ => ACFGNode::Sequence(tiles),
                    }
                }
                None => ACFGNode::Repeat {
                    iter_var,
                    range,
                    body: Box::new(new_body),
                    // Not a block= target: passes through with its
                    // existing tag (none for a source loop).
                    block_tag,
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
                edges: vec![DataflowEdge::new(
                    vec![DataId(0)],
                    KernelId(0),
                    Some(DataId(1)),
                )],
            },
        })
    }

    #[test]
    fn find_loop_range_walks_nested_seq() {
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..32,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
            block_tag: None,
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
            block_tag: None,
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
            block_tag: None,
        };
        let out = rewrite_node(n, &tile_map);
        match out {
            ACFGNode::Repeat {
                iter_var: outer,
                range: outer_range,
                body,
                block_tag: outer_tag,
            } => {
                assert_eq!(outer, IterVar(5));
                assert_eq!(outer_range, 0..4); // 16 / 4 = 4 tiles
                // The synthesised TILE loop carries no rebinding tag —
                // its variable never indexes the body (TASK-0180).
                assert_eq!(outer_tag, None, "tile loop must NOT be tagged");
                match *body {
                    ACFGNode::Sequence(seq) => {
                        assert_eq!(seq.len(), 1);
                        match &seq[0] {
                            ACFGNode::Repeat {
                                iter_var: inner,
                                range: inner_range,
                                block_tag: inner_tag,
                                ..
                            } => {
                                assert_eq!(*inner, IterVar(0));
                                assert_eq!(*inner_range, 0..4); // chunk size
                                // The strip-mined INNER loop is tagged
                                // per-occurrence: divisible single nest
                                // => full (not partial), N=4, num_full=4
                                // (16/4). This is exactly what the
                                // backend rebinds `LO + tile*N + inner`
                                // from (TASK-0180 AC#1).
                                assert_eq!(
                                    *inner_tag,
                                    Some(BlockTag {
                                        block_n: 4,
                                        num_full: 4,
                                        is_partial: false,
                                    }),
                                    "divisible inner loop must carry full-nest BlockTag"
                                );
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

    /// TASK-0142: a non-divisible range produces `Sequence[ full-tile
    /// nest, trailing-partial-tile nest ]`. Shape mirrors the
    /// 05-stencil/blocked case (`for y : 1..15` is length 14, `block=4`
    /// -> 3 full tiles of 4 + a trailing tile of 2).
    #[test]
    fn rewrite_node_emits_trailing_partial_tile() {
        let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
        tile_map.insert(IterVar(0), ("y__tile".to_string(), IterVar(5), 4));
        // length 14 (e.g. range 1..15), block=4 -> num_full=3, rem=2.
        let n = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 1..15,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
            block_tag: None,
        };
        let out = rewrite_node(n, &tile_map);

        let seq = match out {
            ACFGNode::Sequence(s) => s,
            other => panic!("non-divisible must be a Sequence, got {other:?}"),
        };
        assert_eq!(seq.len(), 2, "full-tile nest + trailing partial tile");

        // Helper: assert a nest has outer (tile_id, outer_range), no
        // tag on the tile loop, and an inner (IterVar(0), 0..inner_len)
        // carrying exactly `expect_tag` (TASK-0180 / TASK-0173: the
        // non-divisible full and partial nests get DISTINCT
        // per-occurrence tags so the backend rebinds each correctly —
        // `LO+tile*N+inner` vs `LO+num_full*N+inner`).
        let check_nest = |node: &ACFGNode,
                          outer_range: std::ops::Range<i64>,
                          inner_len: i64,
                          expect_tag: BlockTag| {
            match node {
                ACFGNode::Repeat {
                    iter_var: outer,
                    range,
                    body,
                    block_tag: outer_tag,
                } => {
                    assert_eq!(*outer, IterVar(5), "outer is the tile var");
                    assert_eq!(*range, outer_range);
                    assert_eq!(*outer_tag, None, "tile loop must NOT be tagged");
                    match &**body {
                        ACFGNode::Sequence(inner_seq) => {
                            assert_eq!(inner_seq.len(), 1);
                            match &inner_seq[0] {
                                ACFGNode::Repeat {
                                    iter_var: inner,
                                    range: ir,
                                    block_tag: inner_tag,
                                    ..
                                } => {
                                    assert_eq!(*inner, IterVar(0), "inner keeps source var");
                                    assert_eq!(*ir, 0..inner_len);
                                    assert_eq!(
                                        *inner_tag,
                                        Some(expect_tag),
                                        "inner loop's per-occurrence BlockTag"
                                    );
                                }
                                o => panic!("inner not Repeat: {o:?}"),
                            }
                        }
                        o => panic!("nest body not Sequence: {o:?}"),
                    }
                }
                o => panic!("nest not Repeat: {o:?}"),
            }
        };

        // length 14, block=4 -> num_full=3, rem=2. Both nests carry
        // block_n=4 and num_full=3; only `is_partial` differs (it
        // selects the rebinding base in the backend).
        // Full tiles: 3 tiles of width 4 -> full-nest tag.
        check_nest(
            &seq[0],
            0..3,
            4,
            BlockTag {
                block_n: 4,
                num_full: 3,
                is_partial: false,
            },
        );
        // Trailing partial tile: a single (0..1) tile of width rem=2
        // -> partial tag (rebinds `LO + num_full*N + inner`).
        check_nest(
            &seq[1],
            0..1,
            2,
            BlockTag {
                block_n: 4,
                num_full: 3,
                is_partial: true,
            },
        );
    }

    /// TASK-0142 AC#2 verbatim: `block=64` on `0..100` produces an
    /// outer loop of 2 tiles, an inner of 64 for tile 0, and an inner
    /// of 36 for tile 1.
    #[test]
    fn rewrite_node_ac2_64_then_36() {
        let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
        tile_map.insert(IterVar(0), ("y__tile".to_string(), IterVar(9), 64));
        let n = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..100,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
            block_tag: None,
        };
        let out = rewrite_node(n, &tile_map);
        let seq = match out {
            ACFGNode::Sequence(s) => s,
            o => panic!("expected Sequence (non-divisible), got {o:?}"),
        };
        // "an outer loop of 2 tiles": the full-tile nest (tile 0) and
        // the trailing partial tile (tile 1), in order.
        assert_eq!(seq.len(), 2, "an outer loop of 2 tiles");

        let inner = |node: &ACFGNode| -> std::ops::Range<i64> {
            match node {
                ACFGNode::Repeat {
                    iter_var: ov,
                    body,
                    ..
                } => {
                    assert_eq!(*ov, IterVar(9), "outer is the tile var");
                    match &**body {
                        ACFGNode::Sequence(s) => match &s[0] {
                            ACFGNode::Repeat {
                                iter_var: iv,
                                range,
                                ..
                            } => {
                                assert_eq!(*iv, IterVar(0));
                                range.clone()
                            }
                            o => panic!("inner not Repeat: {o:?}"),
                        },
                        o => panic!("body not Sequence: {o:?}"),
                    }
                }
                o => panic!("nest not Repeat: {o:?}"),
            }
        };
        // 100 / 64 = 1 full tile of 64 (tile 0); 100 % 64 = 36 (tile 1).
        assert_eq!(inner(&seq[0]), 0..64, "inner of 64 for tile 0");
        assert_eq!(inner(&seq[1]), 0..36, "inner of 36 for tile 1");
    }

    /// TASK-0173 AC#3 shape: the exact strip-mine emitted for
    /// 04-prefix-sum/blocked-nondiv's `loop j : block=6` over Pass 3's
    /// within-block-scan ACCUMULATION axis `for j : 0 .. BS` (BS=64).
    ///
    /// 64 is NOT divisible by 6 (64 = 6*10 + 4): `num_full = 10`,
    /// `rem = 4`. This pins the per-occurrence `BlockTag`s the backend
    /// rebinds from for a NON-IDEMPOTENT accumulator axis — the full
    /// nest gets `is_partial=false` (backend emits abs
    /// `LO + j__tile*6 + j`) and the trailing partial tile gets
    /// `is_partial=true` (backend emits the CONSTANT base abs
    /// `LO + num_full*6 + j` = `LO + 10*6 + j`). A wrong tag here
    /// would make the non-divisible accumulator e2e cell diverge from
    /// `reference.bin`; this is the structural companion to that e2e
    /// differential proof.
    #[test]
    fn rewrite_node_prefix_sum_nondiv_j_block6() {
        let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
        tile_map.insert(IterVar(0), ("j__tile".to_string(), IterVar(7), 6));
        // 04-prefix-sum Pass-3 `for j : 0 .. 64`, block=6.
        let n = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..64,
            body: Box::new(ACFGNode::Sequence(vec![op()])),
            block_tag: None,
        };
        let out = rewrite_node(n, &tile_map);

        let seq = match out {
            ACFGNode::Sequence(s) => s,
            other => panic!("non-divisible must be a Sequence, got {other:?}"),
        };
        assert_eq!(
            seq.len(),
            2,
            "block=6 over 0..64 -> full-tile nest + trailing partial tile"
        );

        // Reuse the same nest-shape contract as the 05-stencil shape
        // test: outer is the tile var (untagged), inner keeps the
        // source var and carries exactly the expected per-occurrence
        // BlockTag.
        let check_nest = |node: &ACFGNode,
                          outer_range: std::ops::Range<i64>,
                          inner_len: i64,
                          expect_tag: BlockTag| {
            match node {
                ACFGNode::Repeat {
                    iter_var: outer,
                    range,
                    body,
                    block_tag: outer_tag,
                } => {
                    assert_eq!(*outer, IterVar(7), "outer is the tile var");
                    assert_eq!(*range, outer_range);
                    assert_eq!(*outer_tag, None, "tile loop must NOT be tagged");
                    match &**body {
                        ACFGNode::Sequence(inner_seq) => {
                            assert_eq!(inner_seq.len(), 1);
                            match &inner_seq[0] {
                                ACFGNode::Repeat {
                                    iter_var: inner,
                                    range: ir,
                                    block_tag: inner_tag,
                                    ..
                                } => {
                                    assert_eq!(*inner, IterVar(0), "inner keeps source var");
                                    assert_eq!(*ir, 0..inner_len);
                                    assert_eq!(
                                        *inner_tag,
                                        Some(expect_tag),
                                        "inner loop's per-occurrence BlockTag"
                                    );
                                }
                                o => panic!("inner not Repeat: {o:?}"),
                            }
                        }
                        o => panic!("nest body not Sequence: {o:?}"),
                    }
                }
                o => panic!("nest not Repeat: {o:?}"),
            }
        };

        // 64 / 6 = 10 full tiles of width 6 -> full-nest tag
        // (backend: abs j = LO + j__tile*6 + j).
        check_nest(
            &seq[0],
            0..10,
            6,
            BlockTag {
                block_n: 6,
                num_full: 10,
                is_partial: false,
            },
        );
        // 64 % 6 = 4: a single (0..1) trailing tile of width 4 ->
        // partial tag. Backend rebinds the CONSTANT base
        // abs j = LO + num_full*6 + j = LO + 10*6 + j (NOT
        // tile*6 which would be 0 — the wrong base for an
        // accumulator).
        check_nest(
            &seq[1],
            0..1,
            4,
            BlockTag {
                block_n: 6,
                num_full: 10,
                is_partial: true,
            },
        );
    }
}
