//! Block-loop transformation — TASK-0030.
//!
//! Error convention (decision-0003): this pass is on the
//! typed-`Result` side — it returns
//! `Result<_, BlockTransformError>` (a `pub enum` whose variants
//! carry diagnostic context) rather than `panic!`ing, so any failure
//! is surfaced by the driver as a clean `nucleus: error:` line. Live
//! (constructed) variants:
//!   - `UnknownLoopVar` — a fail-closed guard the linker normally
//!     pre-rejects.
//!   - `BlockOnUntilLoop` — `block=` on a `for..until` early-exit loop
//!     (epic S4); rejected loud rather than dropping the break
//!     predicate.
//!   - `SyntheticTileVarCollision` — the synthetic `<var>__tile`
//!     outer-loop name collides with a user-declared iter var
//!     (TASK-0456); rejected loud rather than aliasing two loops onto
//!     one id.
//!
//! `NotDivisible` is retired per TASK-0142 (no longer constructed) —
//! see those variants' docs.
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
//!   The `block=N + pipeline=D` combination on the same loop is
//!   REJECTED at sched-lower via `SchedLowerErrorKind::BlockPipelineConflict`
//!   (TASK-0215 — closed). Per PRD §6.3.3 the semantics would be
//!   ambiguous (per-tile vs per-iter pipelining); the user picks one.

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
    /// `block=N` targets a `for..until` bounded early-exit loop (epic S4,
    /// TASK-0341.02.01.05.01). Strip-mining rebinds the source iter var
    /// to `tile*N + inner`, but the `until COND` halt predicate references
    /// the source iter var directly, so tiling the loop would require
    /// re-deriving the predicate against the rebound index — codegen work
    /// that does not exist yet. Rather than silently DROP the break
    /// predicate when `tile_nest` synthesises fresh loops (the
    /// `feedback-option-none-skip-arm-silent-drop` anti-pattern), reject
    /// the combination loudly with a typed error. `block=` on a plain
    /// fixed-iteration loop is unaffected.
    BlockOnUntilLoop { var: String },
    /// `block=N` on loop `tiled_var` requires synthesising an outer
    /// tile-loop named `<tiled_var>__tile`, but the algorithm already
    /// declares an iteration variable with that exact name. The
    /// algorithm grammar's identifier rule permits a user variable
    /// literally named `<var>__tile`, so this is a valid (if obscure)
    /// program (TASK-0456).
    ///
    /// The pass does NOT mangle the synthetic name uniquely: silently
    /// reusing the colliding id would alias two distinct loops onto one
    /// `IterVar`, corrupting every downstream pass that keys on it. Per
    /// the pass's typed-`Result` convention (decision-0003) we reject
    /// loudly instead, naming both the schedule's `block=` target loop
    /// (`tiled_var`) and the pre-existing user variable (`tile_var`)
    /// whose name collides, so the user can rename one of them.
    SyntheticTileVarCollision {
        /// The `block=N` target loop variable that triggered the
        /// `<var>__tile` synthesis.
        tiled_var: String,
        /// The pre-existing user iter var whose name is exactly
        /// `<tiled_var>__tile`.
        tile_var: String,
    },
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
            BlockTransformError::BlockOnUntilLoop { var } => write!(
                f,
                "schedule has `loop {var} : block=...` on a `for..until` bounded early-exit \
                 loop; strip-mining a loop with an `until COND` halt predicate is not yet \
                 supported (the predicate references the source iteration variable, which \
                 tiling rebinds). Remove the `block=` directive on `{var}`, or use a plain \
                 fixed-iteration loop. (Tracked under the data-dependent loop-termination \
                 epic, TASK-0341.02.01.)"
            ),
            BlockTransformError::SyntheticTileVarCollision { tiled_var, tile_var } => write!(
                f,
                "schedule has `loop {tiled_var} : block=...`, whose strip-mine synthesises an \
                 outer tile-loop named `{tile_var}` (`{tiled_var}__tile`), but the algorithm \
                 already declares an iteration variable named `{tile_var}`. Rename the user \
                 iteration variable `{tile_var}` (or the `block=` target loop `{tiled_var}`) so \
                 the synthetic `{tiled_var}__tile` name is free. (TASK-0456.)"
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
        // Reject `block=` on a `for..until` loop (epic S4). Tiling
        // synthesises fresh loops via `tile_nest`, which would SILENTLY
        // DROP the break predicate (the source loop is replaced, not
        // rewritten in place). Fail loud here rather than downstream — the
        // predicate references the source iter var and tiling rebinds it,
        // so this is genuinely unsupported until a later codegen slice.
        if loop_has_break_cond_by_id(&acfg.root, iter_var) {
            return Err(BlockTransformError::BlockOnUntilLoop { var: var.clone() });
        }
    }

    // ---- 3. Allocate fresh IterVar ids for the synthetic outer
    //         (tile) iter vars. ----
    //
    // Names: `<var>__tile`. Collision with an existing iter var name
    // is possible if the user happened to declare such a variable in
    // the algorithm; the algorithm grammar's identifier rule allows
    // it (TASK-0456). We don't try to mangle uniquely — silently
    // reusing the colliding id would alias two distinct loops onto one
    // `IterVar` (wrong). Per decision-0003 the pass fails loud with a
    // typed `SyntheticTileVarCollision` naming both variables, so the
    // user can rename one. See the collision guard inside the loop.
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
        // TASK-0260: block_transform runs BEFORE halo_inference so this
        // map is always empty here; forward verbatim. (Aside: even if
        // halo_inference moved earlier, block_transform splits an
        // IterVar into outer-tile + inner-intra-tile pair — a halo
        // entry keyed by the original inner IterVar would need
        // re-binding to the new pair, which is a Stage 2/3 concern.)
        halo_widths,
        // TASK-0261: block_transform runs BEFORE reuse_inference; same
        // pre-populator forwarding stance as halo_widths. (And the same
        // forward-carry hazard: if the pass order ever moves reuse
        // earlier, an IterVar-keyed reuse entry would need re-binding
        // to the synthesised outer-tile / intra-tile pair.)
        reuse_widths,
        // TASK-0264 cycle 113: block_transform runs BEFORE
        // partition_blocks2d in the driver pipeline so partition_pairs
        // / grid_shape_for_outer_iv are always empty here; forward
        // verbatim. (Same forward-carry hazard as halo / reuse: if the
        // pass order ever moves partition_blocks2d earlier, an
        // IterVar-keyed pair entry would need re-binding to the
        // synthesised outer-tile / intra-tile pair.)
        partition_pairs,
        grid_shape_for_outer_iv,
    } = acfg;

    let mut next_id: u64 = name_iter_vars
        .values()
        .map(|v| v.0)
        .max()
        .map(|v| v + 1)
        .unwrap_or(0);

    let mut tile_var_for: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    for (var, n) in &block_by_var {
        // Genuinely unreachable: step 2 validated `name_iter_vars` has
        // an entry for every `var` in `block_by_var`, and the map has
        // not been mutated (only the local `next_id` cursor has) between
        // that validation and here. A `panic!` here would be a true
        // invariant violation, not user-reachable input.
        let inner_id = name_iter_vars.get(var).copied().expect("validated above");
        let tile_name = format!("{var}__tile");
        if name_iter_vars.contains_key(&tile_name) {
            // The user declared an iter var literally named
            // `<var>__tile`, colliding with the synthetic outer
            // tile-loop name. Fail loud with a typed diagnostic naming
            // both variables instead of aliasing two loops onto one id
            // (TASK-0456).
            return Err(BlockTransformError::SyntheticTileVarCollision {
                tiled_var: var.clone(),
                tile_var: tile_name,
            });
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
        halo_widths,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
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

/// `true` iff the first `Repeat` in `node`'s subtree whose `iter_var`
/// equals `target` carries a `for..until` break predicate
/// (`break_cond.is_some()`). Used by [`apply_block_transforms`] to reject
/// a `block=` directive on a `for..until` loop (epic S4) BEFORE the
/// rewrite, so the silent break-predicate drop in `tile_nest` cannot
/// occur. Mirrors [`find_loop_range_by_id`]'s "first match wins,
/// outermost only" traversal.
fn loop_has_break_cond_by_id(node: &ACFGNode, target: IterVar) -> bool {
    match node {
        ACFGNode::Repeat {
            iter_var,
            body,
            break_cond,
            ..
        } => {
            if *iter_var == target {
                break_cond.is_some()
            } else {
                loop_has_break_cond_by_id(body, target)
            }
        }
        ACFGNode::Sequence(children) => children
            .iter()
            .any(|c| loop_has_break_cond_by_id(c, target)),
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => false,
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
        // Synthesised tile loops never carry a `for..until` predicate
        // (the source loop being tiled is guaranteed not to be one — see
        // the `BlockOnUntilLoop` guard in `apply_block_transforms`).
        break_cond: None,
    };
    // The synthesised tile loop's variable never appears in a body
    // index (block_transform only renames the source loop into a
    // tile/inner pair); it needs no rebinding tag.
    ACFGNode::Repeat {
        iter_var: tile_id,
        range: outer_range,
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
        // Synthesised loop — no halt predicate (see above).
        break_cond: None,
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
            break_cond,
        } => {
            // INVARIANT (epic S4): a `for..until` loop (break_cond.is_some)
            // can NOT reach the tiling `Some` arm below — `block=` on such
            // a loop is rejected up front in `apply_block_transforms`
            // (`BlockOnUntilLoop`). So `break_cond` is threaded ONLY through
            // the two pass-through arms (degenerate + non-target); the
            // tiling arm rebuilds via `tile_nest` (break_cond: None on the
            // synthesised loops) and never carries one.
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
                            // Pass-through: preserve the halt predicate.
                            // (Unreachable for a `for..until` given the
                            // BlockOnUntilLoop guard, but keep it sound.)
                            break_cond,
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
                            *tile_id,
                            0..num_full,
                            iter_var,
                            n_i64,
                            &new_body,
                            full_tag,
                        ));
                    }
                    if rem != 0 {
                        tiles.push(tile_nest(
                            *tile_id,
                            0..1,
                            iter_var,
                            rem,
                            &new_body,
                            partial_tag,
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
                    // ... and its halt predicate, unchanged. A
                    // non-targeted `for..until` (e.g. nested under a
                    // tiled loop) survives the rewrite verbatim.
                    break_cond,
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
mod tests;
