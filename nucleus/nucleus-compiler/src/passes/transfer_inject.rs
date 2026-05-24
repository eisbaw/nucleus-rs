//! Transfer injection — TASK-0018.
//!
//! For every dataflow edge that crosses worker entities, insert a
//! matched pair of [`ACFGNode::Xfer`] placeholders carrying
//! [`XferRole::Push`] (on the producer's prior position) and
//! [`XferRole::Wait`] (immediately before the consumer). Both
//! endpoints share a fresh, unique [`SeqTag`] and the
//! [`TransferPolicy`] resolved from `LinkedIR.sched.transfers`.
//!
//! See PRD §6.3.4 (transfer directives), §8.3 (Push/Wait/seq), and
//! TASK-0018 for the contract this implements.
//!
//! Error convention (decision-0003): a cross-worker `Wait` that
//! escapes the ACFG with no producing `Operation` is a cross-pass
//! invariant violation `build_acfg` guarantees cannot occur for valid
//! IR, so this pass `panic!`s with context (the invariant side); it
//! never returns a user-facing error.
//!
//! ## Inputs and outputs
//!
//! Pure function:
//!
//! ```text
//! inject_transfers : (&LinkedIR, ACFG) -> ACFG
//! ```
//!
//! `&LinkedIR` is needed because:
//!
//! - `linked.data_producers` / `linked.data_consumers` tell us where
//!   each data symbol lives, so we know which dataflow edges cross
//!   workers.
//! - `linked.sched.transfers` tells us the policy declared in the
//!   schedule. Per the task spec, capability validation (e.g. async
//!   on a sync-only backend) is deferred — we just thread the policy
//!   through onto each [`XferPlaceholder`].
//!
//! The `ACFG` is consumed and a new ACFG is returned; the name-table
//! maps are forwarded unchanged.
//!
//! ## Algorithm
//!
//! Walk the tree recursively. Maintain a per-data "last known producer
//! worker" map; the link pass tells us each data symbol has at most
//! one producer kernel (single-assignment, PRD §6.2.1), so the
//! producer worker entity for `D` is uniquely identified by the
//! schedule's placement of that kernel.
//!
//! For every [`ACFGNode::Operation`] node O on consumer worker entity
//! `W_c` that reads data `D`:
//!
//! 1. Look up `D`'s producer worker entity `W_p` via
//!    `linked.data_producers`. If absent (e.g. top-level
//!    `load_image` result whose producer entity is the loader's
//!    placement — also recorded in `data_producers`) we skip.
//! 2. If `W_p == W_c`, nothing to inject (intra-worker transfer).
//! 3. Otherwise, allocate a fresh `SeqTag`, build a Push placeholder
//!    on the (src, dst, data, tile, policy) tuple, and a matching
//!    Wait. Insert the Wait immediately before O in O's enclosing
//!    sequence; insert the Push immediately after the producer's
//!    Operation. The producer Operation is located by a parallel
//!    walk that remembers, per data symbol, where it was last
//!    written.
//!
//! Worker entities in the link pass are `BTreeSet<String>` (a kernel
//! can be placed on multiple workers — `place k on {w0,w1,w2,w3}`).
//! The ACFG carries `BTreeSet<WorkerId>` on each Operation. Both sides
//! collapse the same way — we use the BTreeSet equality directly
//! when deciding "same entity?".
//!
//! For the `src` and `dst` fields on [`XferPlaceholder`] we need *a*
//! `WorkerId`, not a set. TASK-0117 fan-out (now landed) emits one
//! XferPlaceholder pair per (src-worker, dst-worker) member of the
//! cartesian product of the producer and consumer entities, skipping
//! same-worker pairs. Each pair carries a fresh `SeqTag` and a tile
//! that is the enclosing iteration tile with any axis named in the
//! ACFG's `partition_worker_ranges` sidecar (TASK-0212) rewritten to
//! the compute worker's slice. The {host} -> {single worker} shape
//! (M1/M2 single-distributed cases) lowers to exactly one pair — no
//! behavioural delta versus the pre-TASK-0117 canonical-collapse path
//! for those cases.
//!
//! ## Honest limitations (recorded for follow-up)
//!
//! - **N-to-M fan-out** (both sides multi-worker, e.g. an all-to-all
//!   shuffle) falls back to the "compute worker = dst" convention
//!   when constructing per-pair tiles. The 1-to-N (broadcast) and
//!   N-to-1 (gather) shapes — the only shapes the in-tree schedules
//!   exercise — pick the multi-worker side correctly. A coordinate-
//!   mapping policy for N-to-M is a follow-up not blocked by any
//!   current example.
//!
//! - **Per-point granularity outside `block=`.** When the consumer
//!   is inside a non-blocked `for`, we still fire one Push/Wait per
//!   *iteration* (per Operation visit). The producer for an
//!   `outer <-- load_input()` placed on `host` therefore gets *one*
//!   Push for every consuming iteration that crosses worker, even
//!   though a real backend would aggregate into one bulk send. The
//!   IterTile we record is the consumer Operation's enclosing-loop
//!   tile. General tile-coalescing across non-`block=` loops remains
//!   a follow-up.
//!
//! - **Per-tile hoist for `block=N`.** When the consumer sits inside
//!   a `block`-inner intra-tile Repeat (marked by
//!   [`ACFG::inner_block_iter_vars`] from
//!   [`crate::passes::block_transform`]), any Wait whose data was
//!   NOT produced in the same intra-tile sequence is hoisted up
//!   through every enclosing `block`-inner Repeat to the nearest
//!   per-tile body sequence and emitted there. The matching Push is
//!   spliced after the producer Operation at *the level the
//!   producer lives at* — same scoping rule the pass has used since
//!   M1. The hoisted Wait's `IterTile` is rewritten to the
//!   destination sequence's enclosing tile so the per-tile semantic
//!   (PRD §6.3.3, "transfers happen per tile") shows up on the
//!   placeholder. TASK-0143.
//!
//!   The hoist is loop-invariance-by-structure, not by index
//!   analysis: any data that crosses a worker boundary and isn't
//!   produced inside the intra-tile body is treated as invariant
//!   w.r.t. the intra-tile iteration. As of TASK-0150 the ACFG
//!   layer *does* now carry per-firing index expressions
//!   (`DataflowEdge::data_in_access` / `data_out_access`, see
//!   `acfg.rs`) — the data needed for a precise per-tile / halo-band
//!   check is present. This pass deliberately does **not** consume
//!   it yet: the precise check only changes behaviour once data is
//!   *partitioned* across workers (TASK-0117 distributed
//!   placement), and synthesising halo strips on top of the
//!   structural hoist is its own substantial pass. So TASK-0150
//!   plumbs the data and stops; the consumer is TASK-0158 (filed),
//!   coupled to TASK-0117. Until then the structural hoist remains
//!   the behaviour — conservatively safe (it can only over-transfer
//!   a full tile where a halo strip would suffice, never
//!   under-synchronise).
//!
//! - **Block-entangled non-block transfers — per-Wait classification
//!   (TASK-0267).** Pass A / Pass B used to treat any Repeat whose
//!   subtree contained a `block`-inner loop as opaque (the original
//!   TASK-0151 over-approximation). The conservative gate predates
//!   TASK-0263's halo-aware tile rewrite; once tiles carry the
//!   partition slice (`rewrite_partition_tiles`) + halo widths
//!   (`extend_xfer_tiles_for_halo`), the per-Wait stay-vs-bubble logic
//!   already in the recursive arms is sufficient. The rule is now
//!   per-Wait: a Wait whose `data` is produced INSIDE the enclosing
//!   Repeat's subtree stays inside (per-iteration rendezvous); a Wait
//!   whose `data` is produced OUTSIDE bubbles up and gets a
//!   whole-symbol Push spliced after the producer, regardless of
//!   whether some sibling block-inner loop coexists. This unblocks
//!   the M5 stencil/distributed shape (host-loaded `img_in`,
//!   partition=rows + inner `block=N`): the row-band slice carried in
//!   `tile` is the right amount to transfer, the inner block tile
//!   does not change what `img_in` data the worker needs, so hoisting
//!   the Wait past the block-governed outer Repeat is correct. Pinned
//!   by `block_nested_in_plain_loop_pairs_the_invariant_wait` and
//!   `mixed_block_and_nonblock_program_pairs_the_nonblock_transfer`.
//!   A future per-block-tile slice (the original TASK-0149/TASK-0158
//!   territory) — where a Repeat iteration genuinely needs a
//!   different sub-region per tile — is not exercised today by any
//!   shipped schedule; when it lands, `extend_xfer_tiles_for_halo`
//!   would need a per-tile counterpart or the Wait would need to
//!   refuse the hoist by inspecting `dataflow_edge.data_in_access`.
//!
//! - **Idempotence by structural skip.** Re-running the pass detects
//!   that a Wait already precedes the consumer Operation (and a Push
//!   already follows the producer Operation) by checking sibling
//!   `Xfer` nodes carrying the same `(src, dst, data, tile)`. It
//!   does NOT re-derive `seq` to be the same — the original
//!   placeholder is left in place. Tests cover this.
//!
//! - **Conflict detection happens upstream at sched-lower.** A schedule
//!   writing `transfer D : sync, async;` is rejected by `lower_transfer`
//!   in `crate::sched::lower` as `SchedLowerErrorKind::ConflictingTransferMode`
//!   (TASK-0193 / TASK-0119) BEFORE this pass runs. So the
//!   `policy_from_directive` helper here is reached only with at most
//!   one mode flag set — the last-option-wins shape is a legacy of when
//!   the conflict check lived only in this pass; the upstream reject
//!   makes it dead-code defensive. Pinned by
//!   `negative_mutually_exclusive_transfer_sync_async` in
//!   `nucleus-compiler/tests/sched_lower.rs`.
//!
//! - **Capability check deferred.** Per the task spec, the backend
//!   isn't chosen at this point. We carry the schedule's stated
//!   policy onto every placeholder; codegen-time TASK-0019+ rejects
//!   `async`/`buffer>1`/`notify=event` against backends whose
//!   `capabilities.toml` doesn't list them.

use std::collections::{BTreeMap, BTreeSet};

use std::num::NonZeroU64;

use crate::acfg::{
    ACFGNode, NotifyMode, Operation, TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
use crate::event::{DataId, IterTile, IterVar, KernelId, SeqTag, WorkerId};
use crate::link::{LinkedIR, WorkerEntity};
use crate::sched::{ResolvedLoopOption, ResolvedTransferDirective, ResolvedTransferOption};

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Inject [`ACFGNode::Xfer`] placeholders into `acfg` per the rules in
/// the module docs. The original name-table maps on the [`ACFG`] are
/// forwarded unchanged.
///
/// Pure: same `(linked, acfg)` produces the same output across runs
/// (BTreeMap-based iteration order and a deterministic `SeqTag`
/// counter make this so).
///
/// Idempotent: re-running on the output yields the same tree
/// structurally. See `tests/transfer_inject.rs`.
pub fn inject_transfers(linked: &LinkedIR, acfg: ACFG) -> ACFG {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        // Forward through pre-existing pipeline depths. In the
        // standard driver pipeline this is always empty here (we are
        // the populator), but tests may inject their own values
        // before calling us — preserve them.
        pipeline_depth_for_seq: pre_existing_pipeline_depth,
        // TASK-0263 Stage 2: transfer_inject consumes halo_widths to
        // extend per-tile transfer ranges by the inferred stencil
        // halo widths. The sidecar is read below at the
        // `extend_xfer_tiles_for_halo` step and forwarded verbatim
        // afterwards — it remains the single source of truth (the
        // pass does not rewrite or invalidate halo entries; it only
        // applies them to Xfer tiles).
        halo_widths,
        // TASK-0261: transfer_inject currently DOES NOT read the
        // reuse-widths sidecar — Stage 2 (TASK-0265, backend walker
        // delay-line emit) is the wiring cycle. Forward verbatim.
        reuse_widths,
        // TASK-0264 cycle 113: transfer_inject consumes partition_pairs +
        // grid_shape_for_outer_iv (populated by partition_blocks2d) as
        // of TASK-0289 cycle 114a — see `inject_halo_strip_xfers` at the
        // tail of the finalisation chain below. Forward verbatim into
        // the returned ACFG: the consumer is read-only and never
        // mutates either map.
        partition_pairs,
        grid_shape_for_outer_iv,
    } = acfg;

    // Resolve the link pass's `WorkerEntity` (BTreeSet<String>) to a
    // `BTreeSet<WorkerId>` once, keyed by data symbol name. This is
    // the producer-side worker entity per data symbol.
    let producers_by_data: BTreeMap<DataId, BTreeSet<WorkerId>> = linked
        .data_producers
        .iter()
        .filter_map(|(data_name, entity)| {
            let data_id = name_data.get(data_name).copied()?;
            let workers = entity_to_workerid_set(entity, &name_workers);
            Some((data_id, workers))
        })
        .collect();

    // Resolve the schedule's transfer directives, keyed by DataId.
    // The schedule's `transfers` is keyed by data NAME; we translate
    // once.
    let policies_by_data: BTreeMap<DataId, TransferPolicy> = linked
        .sched
        .transfers
        .iter()
        .filter_map(|(data_name, dir)| {
            let data_id = name_data.get(data_name).copied()?;
            Some((data_id, policy_from_directive(dir)))
        })
        .collect();

    // Resolve `loop VAR : pipeline=D` directives into a per-IterVar
    // map (TASK-0134). The schedule's `loops` is keyed by var NAME;
    // we translate once via `name_iter_vars`. Loops not in
    // `name_iter_vars` (e.g. a `loop x : pipeline=N` referencing a
    // var the algorithm doesn't have) are already a link-step error
    // (LinkErrorKind::UnknownLoop), so we silently skip them here —
    // build_acfg will not have reached us if link rejected.
    let pipeline_depth_for_iter_var: BTreeMap<IterVar, NonZeroU64> = linked
        .sched
        .loops
        .iter()
        .filter_map(|(var_name, dir)| {
            let iv = name_iter_vars.get(var_name).copied()?;
            // PRD §6.3.3: only `pipeline=D` lowers to an initial
            // marking; `block=`, `vectorize=`, `unroll=`, `reuse`,
            // `partition=` all have other effects handled by other
            // passes.
            let depth = dir.options.iter().find_map(|opt| match opt {
                ResolvedLoopOption::Pipeline(d) => NonZeroU64::new(*d),
                _ => None,
            })?;
            Some((iv, depth))
        })
        .collect();

    let ctx = InjectCtx {
        producers_by_data: &producers_by_data,
        policies_by_data: &policies_by_data,
        inner_block_iter_vars: &inner_block_iter_vars,
    };

    // Counter state for SeqTag generation. A single monotonic counter
    // across the whole ACFG is simplest and meets the "unique per
    // pair" requirement. PRD §8.3 talks about per-(src,dst,data)
    // monotonic numbering — that ordering is also a strict subset of
    // a single global counter (per-triple subsequences are still
    // strictly increasing). The global counter has the property that
    // every Push/Wait pair in the whole program has a distinct seq,
    // which is the strongest reading of "unique".
    let mut state = State {
        next_seq: 0,
        last_writer: BTreeMap::new(),
    };

    let new_root = inject_in_node(root, &ctx, &mut state);

    // Cross-scope finalisation (TASK-0136 / TASK-0139). The single-
    // pass `inject_in_sequence` above only rendezvouses a Push with a
    // Wait when both live in the *same* sequence. When the consumer is
    // inside a plain `for` loop and the producer is in the enclosing
    // sequence (example 02-split: `load_input` on host at top level,
    // `add` on w0 inside `for i`), the Wait is trapped inside the
    // Repeat body and never gets a Push — the net deadlocks.
    //
    // The correct model is *whole-symbol* transfer: a datum that is
    // loop-invariant w.r.t. a Repeat crosses the worker boundary once,
    // not once per iteration. Pass A hoists such Waits out of the loop
    // body; Pass B places the matching Push (after the producer, or
    // after the producer's enclosing loop when the consumer sits
    // outside it — the dual case, e.g. `c` produced inside `for i` and
    // read by `save_output` after the loop).
    //
    // Scope: PER-WAIT (TASK-0267). Pass A's stay-vs-bubble decision is
    // already keyed on whether the Wait's `data` is produced inside
    // the enclosing Repeat's subtree, so a block-governed nest does
    // not need a separate opacity gate: a host-loaded image (producer
    // outside) bubbles up and gets a whole-symbol Push at the
    // producer's scope, while an in-loop produced datum (producer
    // inside the loop body) stays for per-iteration rendezvous. The
    // per-worker partition slice is added later by
    // `rewrite_partition_tiles` (TASK-0117/TASK-0212) and halo widths
    // by `extend_xfer_tiles_for_halo` (TASK-0263), so the Wait carries
    // the right slice regardless of which Repeat it crossed. A future
    // per-block-tile slice (the original TASK-0149/TASK-0158
    // territory) would need a per-tile counterpart to halo extension;
    // no shipped schedule exercises that shape today.
    let new_root = {
        let (hoisted, escaped_at_root) = hoist_invariant_waits(new_root, &[]);
        // A Wait that bubbled all the way out of the tree had no
        // producing scope anywhere (its data is produced by no
        // Operation in the whole ACFG). For a well-formed ACFG this
        // never happens — a cross-worker Wait is only emitted because
        // the schedule records a producer, which `build_acfg` lowers
        // to an Operation. Dropping it silently here would hand a
        // downstream pass an unpaired Wait to mis-diagnose. Fail loud
        // with context instead (TASK-0152).
        if let Some(w) = escaped_at_root.first() {
            let data_name = name_data
                .iter()
                .find_map(|(n, id)| (*id == w.data).then_some(n.as_str()))
                .unwrap_or("<unknown>");
            panic!(
                "transfer_inject invariant violation: cross-worker Wait for \
                 data `{data_name}` (id {:?}, seq {:?}, {:?} -> {:?}) escaped \
                 the whole ACFG with no producing Operation. A Wait is only \
                 emitted when the schedule records a producer for the symbol, \
                 so this is a malformed ACFG (producer kernel not lowered), \
                 not a partial test input.",
                w.data, w.seq, w.src, w.dst
            );
        }
        let spliced = splice_pushes_global(hoisted, &name_data);
        // TASK-0117 per-worker tile finalisation. The hoist + splice
        // passes above may rewrite a Wait/Push tile to the
        // post-hoist enclosing tile (e.g. a loop-invariant `input`
        // Wait at top level lands with `[]`). For a fan-out pair the
        // tile must reflect the COMPUTE worker's partition slice so
        // the backend host-side gather can slice-paste. We do the
        // rewrite as a final ACFG walk keyed by the pair's
        // src/dst against `partition_worker_ranges`; pairs whose
        // neither endpoint is partitioned (the 1:1 host↔single-worker
        // shape from examples 01..07) survive unchanged because the
        // map has no entry for either side.
        let partitioned = rewrite_partition_tiles(spliced, &partition_worker_ranges);
        // TASK-0263 Stage 2 halo extension. For each XferPlaceholder
        // whose tile axis carries a non-zero halo entry (the data
        // symbol's consumer kernel's halo widths along that
        // iter-var), extend the axis range by halo on both sides.
        // Clamp against the iter-var's algorithm source range
        // expanded by halo — i.e. the data extent the (unpartitioned)
        // loop would have read. No-op when halo_widths is empty
        // (every pre-Stage-2 example).
        let with_halo = extend_xfer_tiles_for_halo(partitioned, &halo_widths);
        // TASK-0289 cycle 114a halo-strip Push/Wait synthesis. For each
        // `(outer_iv, inner_iv)` pair recorded in `partition_pairs`
        // (populated by partition_blocks2d), synthesise cross-worker
        // halo-strip Push/Wait pairs between N/S/E/W neighbours in the
        // 2D worker grid. Corner pairs (NE/NW/SE/SW) are excluded in
        // this first cut per the TASK-0289 brief. Runs AFTER
        // `rewrite_partition_tiles` and `extend_xfer_tiles_for_halo`
        // so the carefully-crafted halo-strip tiles we emit are not
        // clobbered by either pass — both rewrite tiles in-place by
        // walking every `Xfer`. Short-circuits on empty
        // `partition_pairs` (AC#3 additive-only contract: every shipped
        // schedule today has empty pairs and therefore sees no change
        // in injected XferPlaceholders).
        inject_halo_strip_xfers(
            with_halo,
            &halo_widths,
            &partition_pairs,
            &grid_shape_for_outer_iv,
            &partition_worker_ranges,
            &policies_by_data,
            &mut state,
        )
    };

    // TASK-0134 — populate the pipeline-depth sidecar AFTER all the
    // hoist/splice/rewrite passes have run. Doing it now (rather than
    // at `fresh_seq` time) is load-bearing: `hoist_invariant_waits`
    // moves Waits OUT of the loop body for loop-invariant whole-symbol
    // transfers, AND rewrites their `tile` to the post-hoist
    // enclosing tile. The seq's pipeline depth must reflect WHERE the
    // Push/Wait pair LANDED, not where it was born — otherwise we'd
    // pre-seed a buffer place with `D` tokens when only one
    // Push/Wait fires for the whole loop and the buffer place would
    // overflow at construction.
    //
    // Walk each `Xfer`'s `tile` (post-hoist enclosing iteration tile)
    // against `pipeline_depth_for_iter_var`; the innermost matching
    // entry wins. If neither endpoint of the pair sits inside a
    // pipelined loop, the seq has no entry (-> `initial_marking = 0`,
    // the default). Pairs whose Push and Wait have differing tiles
    // are kept in sync because: (1) the Wait's tile is the
    // authoritative enclosing context (it is the consumer side; the
    // pair fires per-consumer-tile), and (2) for whole-symbol
    // hoisting both sides are moved together so tiles agree post-
    // splice. We aggregate per seq using "any endpoint inside a
    // pipelined loop" as the trigger, but in practice the two
    // endpoints share the same enclosing tile by construction.
    let mut pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64> = pre_existing_pipeline_depth;
    if !pipeline_depth_for_iter_var.is_empty() {
        annotate_pipeline_depth_for_seq(
            &new_root,
            &pipeline_depth_for_iter_var,
            &mut pipeline_depth_for_seq,
        );
    }

    ACFG {
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
    }
}

/// Walk the final ACFG (post-hoist, post-splice, post-rewrite-tiles)
/// and populate `out` with (SeqTag, depth) for every `Xfer`
/// placeholder whose enclosing iteration tile contains an iter-var
/// with a `pipeline=D` directive. **Innermost wins.**
///
/// The "innermost wins" semantic rests on [`IterTile::bounds`]'s
/// documented convention (event.rs ~line 227): "Outer-most iteration
/// variable first." Walking `bounds.iter().rev()` therefore visits
/// innermost-to-outermost, and `find_map` stops at the first
/// (= innermost) pipelined iter-var. All construction sites build
/// `bounds` in nest order — the enclosing-tile stack in
/// `inject_in_sequence`, the fan-out path, and (since TASK-0224) the
/// partition-rewrite path, which walks the enclosing `Repeat` stack
/// directly rather than relying on `IterVar` id ordering.
///
/// Why we use the Xfer's `tile` (not the live walk's enclosing-tile
/// stack): after `hoist_invariant_waits` runs, a Wait may live at a
/// shallower nesting than the Repeat it was born in, but its `tile`
/// is rewritten to that shallower nesting (see line ~733 in
/// `inject_in_sequence`). The tile is therefore the authoritative
/// "in which loop does this transfer fire" record.
///
/// Determinism: the walk is depth-first source-order; `BTreeMap`
/// insertion preserves stable iteration. If Push and Wait of the
/// same seq disagree on depth (shouldn't happen in well-formed
/// inputs — `splice_pushes_global` places the Push at the same
/// tile-scope as the Wait), **last-visited wins**: source-order
/// traversal visits Push before Wait inside a Sequence, so in
/// practice the Wait's annotation is what lands. In well-formed
/// inputs they agree, so the distinction is academic; documented
/// here so a future invariant change doesn't surprise the reader.
fn annotate_pipeline_depth_for_seq(
    node: &ACFGNode,
    pipeline_for_iv: &BTreeMap<IterVar, NonZeroU64>,
    out: &mut BTreeMap<SeqTag, NonZeroU64>,
) {
    match node {
        ACFGNode::Xfer(x) => {
            let depth = x
                .tile
                .bounds
                .iter()
                .rev()
                .find_map(|(iv, _)| pipeline_for_iv.get(iv).copied());
            if let Some(d) = depth {
                out.insert(x.seq, d);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Sequence(children) => {
            for c in children {
                annotate_pipeline_depth_for_seq(c, pipeline_for_iv, out);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            annotate_pipeline_depth_for_seq(body, pipeline_for_iv, out);
        }
    }
}

// --------------------------------------------------------------------
// Context and mutable state
// --------------------------------------------------------------------

/// Immutable inputs threaded through the walk.
struct InjectCtx<'a> {
    /// Producer worker set for each data symbol.
    producers_by_data: &'a BTreeMap<DataId, BTreeSet<WorkerId>>,
    /// Transfer policy for each data symbol that the schedule named.
    policies_by_data: &'a BTreeMap<DataId, TransferPolicy>,
    /// Iter-var IDs that the block-transform pass marked as inner
    /// (intra-tile) loops. Push/Wait pairs emitted inside one of
    /// these get hoisted to the enclosing per-tile body — TASK-0143.
    inner_block_iter_vars: &'a BTreeSet<IterVar>,
}

/// Mutable state threaded through the walk.
struct State {
    /// Next SeqTag to hand out.
    next_seq: u64,
    /// For every data symbol that has been written so far in the walk,
    /// the worker set that wrote it. Updated as we encounter
    /// `Operation`s that produce data. Used to confirm the consumer's
    /// view of where data came from (a sanity tie-back against
    /// `linked.data_producers`; the two should agree).
    last_writer: BTreeMap<DataId, BTreeSet<WorkerId>>,
}

impl State {
    fn fresh_seq(&mut self) -> SeqTag {
        let s = SeqTag(self.next_seq);
        self.next_seq += 1;
        s
    }
}

// --------------------------------------------------------------------
// Recursive walker
// --------------------------------------------------------------------

/// Inject into one ACFGNode. Returns the (possibly rewritten) node.
fn inject_in_node(node: ACFGNode, ctx: &InjectCtx<'_>, state: &mut State) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            ACFGNode::Sequence(inject_in_sequence(children, ctx, state, &[], None))
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // Build the enclosing-tile contribution from this Repeat.
            // We re-walk the body with the contribution pushed onto a
            // local stack; the helper `inject_in_node_with_tile` does
            // the bookkeeping.
            let outer_tile = vec![(iter_var, range.clone())];
            let new_body = inject_in_node_with_tile(*body, ctx, state, &outer_tile, None);
            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(new_body),
                // transfer_inject only injects Push/Wait/Xfer into the
                // body; the strip-mine rebinding tag is structural and
                // survives verbatim (TASK-0180).
                block_tag,
            }
        }
        // Leaves: nothing to inject inside.
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => {
            // Record any side-effect on `last_writer` for non-nested
            // top-level ops in the Sequence walker; here we only see
            // leaves *outside* a Sequence (shouldn't really happen
            // from `build_acfg`, but defensive). Update the writer map
            // if it's an Operation.
            if let ACFGNode::Operation(op) = &leaf {
                update_writer(state, op);
            }
            leaf
        }
    }
}

/// Same as [`inject_in_node`] but with an enclosing-tile context
/// passed in. Used inside `Repeat` bodies so that any
/// `XferPlaceholder` created carries the iteration tile of the
/// enclosing loop(s).
///
/// The optional `hoist_sink` is set when this node sits *inside* a
/// `block`-inner intra-tile loop; instead of emitting Waits in-line
/// at the consumer Operation and Pushes after the producer
/// Operation, the walker forwards them to the sink so the
/// enclosing per-tile body sequence can place them around the inner
/// `Repeat`. TASK-0143.
fn inject_in_node_with_tile(
    node: ACFGNode,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    hoist_sink: Option<&mut HoistSink>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => ACFGNode::Sequence(inject_in_sequence(
            children,
            ctx,
            state,
            enclosing_tile,
            hoist_sink,
        )),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            let mut nested = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            // Forward whatever sink we received into the body. The
            // decision of WHERE a sink originates and WHERE it
            // drains lives entirely in `inject_in_sequence` (which
            // creates one when it encounters a `block`-inner Repeat
            // child and parent_sink is None). Repeats themselves
            // are transparent: they neither create sinks nor drop
            // them, regardless of whether the current Repeat is
            // block-inner or not. This is what lets a 2D-blocked
            // schedule hoist through the (outer-j, inner-i) layers
            // all the way up to the outer-i tile body. TASK-0143.
            let new_body = inject_in_node_with_tile(*body, ctx, state, &nested, hoist_sink);
            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(new_body),
                // Transparent reconstruction — preserve the strip-mine
                // rebinding tag verbatim (TASK-0180).
                block_tag,
            }
        }
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => {
            if let ACFGNode::Operation(op) = &leaf {
                update_writer(state, op);
            }
            leaf
        }
    }
}

/// Per-tile hoist sink: collected Wait placeholders that the inner
/// block-loop body would have emitted, plus a record of producer
/// indices observed at the parent (per-tile) sequence so the matching
/// Push can be spliced after the right Operation.
///
/// One sink instance is created by `inject_in_sequence` whenever it
/// is about to recurse into a `block`-inner `Repeat` child; the sink
/// is then drained back into the parent sequence's `out` vector and
/// the parent's local-producer index map, around the inner Repeat
/// node.
#[derive(Default)]
struct HoistSink {
    /// Waits generated inside the inner block-loop, to be placed in
    /// the parent (per-tile body) sequence *before* the inner
    /// `Repeat` node. Idempotence dedup happens at drain time.
    waits: Vec<XferPlaceholder>,
}

/// Walk a sequence of children, injecting Wait placeholders before
/// each Operation that reads cross-worker data and Push placeholders
/// after each Operation that produces data that someone downstream
/// will read on a different worker.
///
/// Implementation note: we emit *both* Wait (before the consumer) and
/// Push (after the producer) in a single linear scan. For each
/// consumer-side read of `D` whose producer entity differs from the
/// consumer's, we walk back through the children we've already
/// emitted to find the most recent producer of `D`, and splice a
/// Push placeholder immediately after it. That keeps the rewrite
/// local — no second pass needed.
fn inject_in_sequence(
    children: Vec<ACFGNode>,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    // If non-None, this sequence sits *inside* a `block`-inner
    // intra-tile loop chain: any non-locally-produced Wait we
    // would emit gets diverted into the sink instead of staying
    // here, so the eventual destination sequence (the nearest
    // ancestor whose `parent_sink` was None) places the Wait at
    // per-tile granularity.
    mut parent_sink: Option<&mut HoistSink>,
) -> Vec<ACFGNode> {
    let mut out: Vec<ACFGNode> = Vec::with_capacity(children.len());

    // For each data symbol we've already produced in this sequence,
    // remember the index of the producing Operation in `out`. We use
    // this to splice a Push placeholder immediately after it.
    // BTreeMap for deterministic iteration; only the per-key insert
    // matters semantically.
    let mut local_producer_idx: BTreeMap<DataId, usize> = BTreeMap::new();

    // Buffer of (insert-before-index-in-out, Wait) for Xfers hoisted
    // out of an inner block-loop sibling that THIS sequence is the
    // destination for. Only populated when `parent_sink` is None at
    // the point we encounter a block-inner Repeat child. Drained into
    // `out` at the end of the walk.
    let mut hoisted_waits_to_place: Vec<(usize, XferPlaceholder)> = Vec::new();
    // Local sink (one per block-inner child we are draining for).
    // Held outside the loop so its lifetime spans the loop.
    let mut local_sink: HoistSink = HoistSink::default();
    let parent_sink_is_some = parent_sink.is_some();

    for child in children {
        // Peek at the child's shape before the recursion.
        let child_is_block_inner_repeat = matches!(
            &child,
            ACFGNode::Repeat { iter_var, .. }
                if ctx.inner_block_iter_vars.contains(iter_var)
        );

        let child = if child_is_block_inner_repeat && parent_sink_is_some {
            // We're not the destination; forward hoisted Waits
            // through to our parent's sink. `as_deref_mut` reborrows
            // for the duration of the recursion.
            let sink_ref: &mut HoistSink = parent_sink.as_deref_mut().expect("checked");
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(sink_ref))
        } else if child_is_block_inner_repeat {
            // We are the per-tile destination. Pass our `local_sink`
            // to the recursion; record the slot where the rewritten
            // Repeat will land so we can place hoisted Waits
            // immediately before it; then move the sink's contents
            // into `hoisted_waits_to_place` for the post-walk drain.
            let rewritten =
                inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(&mut local_sink));
            let slot = out.len();
            // Drain local_sink between block-inner siblings.
            for w in std::mem::take(&mut local_sink.waits) {
                hoisted_waits_to_place.push((slot, w));
            }
            rewritten
        } else if parent_sink_is_some {
            // Non-block-inner child inside an inner-block context:
            // propagate parent_sink so Waits emitted further down
            // can still hoist past this scope.
            let sink_ref: &mut HoistSink = parent_sink.as_deref_mut().expect("checked");
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(sink_ref))
        } else {
            // Plain non-block-inner child outside any inner-block
            // context: recurse with no sink. Standard pre-TASK-0143
            // path.
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, None)
        };

        match &child {
            ACFGNode::Operation(op) => {
                // ---- Wait injection (consumer side) ----
                let waits = build_waits_for_op(op, ctx, state, enclosing_tile);
                for w in waits {
                    // Idempotence: if the immediately-preceding
                    // element is already a Wait with matching
                    // (src, dst, data, tile), skip pushing it.
                    if !is_duplicate_xfer(out.last(), &w) {
                        out.push(ACFGNode::Xfer(w));
                    }
                }

                // ---- Update local_producer_idx for any data this op
                //      writes. We record the index AFTER pushing the
                //      Operation so the Push insertion targets the
                //      right slot. ----
                let idx_after_push = out.len();
                update_writer(state, op);
                out.push(child.clone());
                if let Some(data_out) = output_data(op) {
                    local_producer_idx.insert(data_out, idx_after_push);
                }

                // ---- Push injection (producer side) for any data
                //      this op produces whose downstream consumer
                //      will be cross-worker. We can't know future
                //      consumers up-front, so we add Pushes lazily
                //      when a future consumer is seen (see the
                //      Wait-handling branch above): the moment a
                //      Wait is emitted for (src, dst, D, tile), we
                //      also splice a Push at `local_producer_idx[D]
                //      + 1` if not already present. That happens in
                //      the next iteration; see below. ----
            }
            _ => {
                // Non-Operation nodes (Repeat, Sequence, Sync, Xfer):
                // already rewritten by recursion. Push as-is.
                out.push(child);
            }
        }
    }

    // Drain hoisted Waits: insert each (slot, Wait) into `out` so
    // the Wait sits immediately before the inner Repeat. Inserting
    // shifts later indices, so we process by descending slot. For a
    // given slot, multiple Waits keep their relative order — same
    // semantics as the in-line emission path.
    //
    // Before placement, rewrite each hoisted Wait's `IterTile` to
    // match THIS sequence's enclosing tile. The consumer's Wait was
    // built deeper in the nest and carries axes for every loop
    // between the consumer and the original block-inner; once
    // hoisted to the per-tile destination, only the axes enclosing
    // this destination should remain. Replacing the tile wholesale
    // is simpler than tracking which axes were crossed during the
    // hoist forward, and matches the PRD §6.3.3 "transfers happen
    // per tile" semantic: the Wait fires at this sequence's
    // enclosing-tile granularity.
    for (_, w) in hoisted_waits_to_place.iter_mut() {
        w.tile = IterTile::new(enclosing_tile.to_vec());
    }
    if !hoisted_waits_to_place.is_empty() {
        // Stable sort by slot ascending so later (larger-slot)
        // inserts can be applied first without invalidating earlier
        // slots' positions.
        hoisted_waits_to_place.sort_by_key(|(s, _)| *s);
        // Group by slot, then for each group insert in reverse slot
        // order. Within a group, preserve original order by
        // inserting in reverse.
        let mut by_slot: BTreeMap<usize, Vec<XferPlaceholder>> = BTreeMap::new();
        for (s, w) in hoisted_waits_to_place {
            by_slot.entry(s).or_default().push(w);
        }
        // Iterate in *reverse* slot order. Iter rev on BTreeMap.
        let mut shifts: Vec<(usize, Vec<XferPlaceholder>)> = by_slot.into_iter().collect();
        shifts.sort_by_key(|(s, _)| *s);
        for (slot, waits) in shifts.into_iter().rev() {
            // Insert each Wait at `slot`, in reverse so the first
            // listed Wait ends up first in `out`.
            for w in waits.into_iter().rev() {
                // Idempotence: skip if `out` already contains an
                // identical Wait *anywhere*. Re-running the pass on
                // a hoisted ACFG re-derives the same Wait from the
                // consumer Op; without this dedup we would emit a
                // second copy at the same slot. We check against
                // the entire sequence (not just `out[slot]`) because
                // the previously-hoisted Wait may sit at a slot that
                // shifted as siblings were rewritten.
                let already_present = out.iter().any(|n| {
                    matches!(
                        n,
                        ACFGNode::Xfer(existing)
                            if existing.role == XferRole::Wait
                                && existing.src == w.src
                                && existing.dst == w.dst
                                && existing.data == w.data
                                && existing.tile == w.tile
                    )
                });
                if already_present {
                    continue;
                }
                out.insert(slot, ACFGNode::Xfer(w));
            }
        }
        // Recompute `local_producer_idx` -- insertions shifted some
        // recorded indices. Cheap to walk `out` once.
        local_producer_idx.clear();
        for (i, n) in out.iter().enumerate() {
            if let ACFGNode::Operation(op) = n {
                if let Some(data_out) = output_data(op) {
                    local_producer_idx.insert(data_out, i);
                }
            }
        }
    }

    // Second pass: for every Wait we emitted (in-line *or* hoisted)
    // in this sequence, walk back and splice a matching Push
    // immediately after the producer Operation (which is in `out`
    // because the producer's Operation appeared earlier in the same
    // sequence) — *unless* the producer is in a different scope
    // (outer sequence, sibling sequence). In that case the
    // outer-scope walker handles it. We detect "different scope" by
    // checking `local_producer_idx`: if the Wait names a data symbol
    // not in `local_producer_idx`, the producer is not in this
    // sequence and we leave the Push to the caller.
    splice_pushes_for_waits(&mut out, &local_producer_idx);

    // If this sequence is *itself* sitting inside a block-inner
    // Repeat chain (i.e. our caller passed a `parent_sink`), forward
    // any Waits we just placed up to the parent so they land at the
    // eventual destination's per-tile granularity. We only forward
    // Waits whose producer was NOT in this sequence — in-sequence
    // Push/Wait pairs already form a complete intra-tile rendezvous
    // and need not propagate further.
    //
    // Tile rewrite happens at the destination (see the drain block
    // above); here we simply hand the Wait off unchanged.
    if let Some(sink) = parent_sink {
        let mut kept: Vec<ACFGNode> = Vec::with_capacity(out.len());
        for node in out.into_iter() {
            match node {
                ACFGNode::Xfer(x)
                    if x.role == XferRole::Wait && !local_producer_idx.contains_key(&x.data) =>
                {
                    sink.waits.push(x);
                }
                other => kept.push(other),
            }
        }
        out = kept;
    }

    out
}

/// Walk `out` left-to-right; for every Wait placeholder, look up the
/// producer Operation in `local_producer_idx` and splice a Push
/// placeholder right after it (with the same `seq` as the Wait).
///
/// Splicing later items shifts indices, so we collect insertion
/// requests first and then apply them in reverse order to keep the
/// recorded indices valid.
fn splice_pushes_for_waits(out: &mut Vec<ACFGNode>, local_producer_idx: &BTreeMap<DataId, usize>) {
    // (insert_at, placeholder)
    let mut inserts: Vec<(usize, XferPlaceholder)> = Vec::new();

    for n in out.iter() {
        if let ACFGNode::Xfer(x) = n {
            if x.role == XferRole::Wait {
                if let Some(&producer_idx) = local_producer_idx.get(&x.data) {
                    // Push goes immediately after the producer
                    // Operation, i.e. at producer_idx + 1.
                    let insert_at = producer_idx + 1;
                    let push = XferPlaceholder {
                        role: XferRole::Push,
                        src: x.src,
                        dst: x.dst,
                        data: x.data,
                        tile: x.tile.clone(),
                        seq: x.seq,
                        policy: x.policy,
                    };
                    inserts.push((insert_at, push));
                }
            }
        }
    }

    // Apply in reverse index order so earlier insertions don't
    // invalidate the later ones.
    inserts.sort_by_key(|(i, _)| *i);
    for (insert_at, push) in inserts.into_iter().rev() {
        // Idempotence: if the slot immediately after the producer is
        // already a Push with matching (src, dst, data, tile), skip.
        let already = out.get(insert_at).and_then(|n| match n {
            ACFGNode::Xfer(x) => Some(x.clone()),
            _ => None,
        });
        if let Some(existing) = already {
            if existing.role == XferRole::Push
                && existing.src == push.src
                && existing.dst == push.dst
                && existing.data == push.data
                && existing.tile == push.tile
            {
                continue;
            }
        }
        out.insert(insert_at, ACFGNode::Xfer(push));
    }
}

// --------------------------------------------------------------------
// Pass A — hoist loop-invariant Waits out of plain Repeat bodies
// --------------------------------------------------------------------
//
// TASK-0136 / TASK-0139. `inject_in_sequence` emits a Wait immediately
// before the consumer Operation. When the consumer sits inside a
// `for` loop but the data it reads is produced *outside* the loop
// (loop-invariant by structure: the data symbol is not written by any
// Operation inside the loop body), the Wait should fire once before
// the loop, not once per iteration. The Petri lowering unrolls Repeat
// bodies (`acfg_to_petri::walk`), so a per-iteration Wait against a
// single whole-symbol Push deadlocks at iteration 2 (one token, N
// consumers). Hoisting the Wait out of the body fixes this and matches
// what a real backend does (transfer the whole symbol once).
//
// Returns the rewritten node plus the Waits that escaped past the top
// of `node` and must be placed (or further hoisted) by the caller.

/// Collect every data symbol written by some Operation anywhere in the
/// subtree rooted at `node`.
fn produced_data_set(node: &ACFGNode, acc: &mut BTreeSet<DataId>) {
    match node {
        ACFGNode::Operation(op) => {
            if let Some(d) = output_data(op) {
                acc.insert(d);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
        ACFGNode::Repeat { body, .. } => produced_data_set(body, acc),
        ACFGNode::Sequence(children) => {
            for c in children {
                produced_data_set(c, acc);
            }
        }
    }
}

/// Number of Operations in the subtree that write `data`. Used only
/// by a debug-assert guarding the single-assignment invariant
/// (TASK-0153); not on the release hot path.
fn count_producers(node: &ACFGNode, data: DataId) -> usize {
    match node {
        ACFGNode::Operation(op) => usize::from(output_data(op) == Some(data)),
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        ACFGNode::Repeat { body, .. } => count_producers(body, data),
        ACFGNode::Sequence(children) => children.iter().map(|c| count_producers(c, data)).sum(),
    }
}

fn hoist_invariant_waits(
    node: ACFGNode,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
) -> (ACFGNode, Vec<XferPlaceholder>) {
    match node {
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => (leaf, Vec::new()),
        // A bare Wait/Xfer outside a Sequence shouldn't occur from the
        // builder, but if it does we cannot decide invariance without
        // its sibling context — leave it in place.
        leaf @ ACFGNode::Xfer(_) => (leaf, Vec::new()),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            let mut nested: Vec<(IterVar, std::ops::Range<i64>)> = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            let (body2, escaped) = hoist_invariant_waits(*body, &nested);

            let mut produced = BTreeSet::new();
            produced_data_set(&body2, &mut produced);

            // A Wait that escaped the body but whose data IS produced
            // inside this loop is NOT loop-invariant w.r.t. this
            // Repeat: it belongs inside, as a per-iteration rendezvous.
            // Re-inject it at the front of the body. Otherwise it keeps
            // bubbling up.
            let mut stay: Vec<XferPlaceholder> = Vec::new();
            let mut bubble: Vec<XferPlaceholder> = Vec::new();
            for w in escaped {
                if produced.contains(&w.data) {
                    stay.push(w);
                } else {
                    bubble.push(w);
                }
            }

            let body3 = if stay.is_empty() {
                body2
            } else {
                let mut children: Vec<ACFGNode> = stay.into_iter().map(ACFGNode::Xfer).collect();
                match body2 {
                    ACFGNode::Sequence(cs) => children.extend(cs),
                    other => children.push(other),
                }
                ACFGNode::Sequence(children)
            };

            (
                ACFGNode::Repeat {
                    iter_var,
                    range,
                    body: Box::new(body3),
                    // Wait-hoisting only reshuffles the body; the
                    // strip-mine rebinding tag is preserved verbatim
                    // (TASK-0180).
                    block_tag,
                },
                bubble,
            )
        }
        ACFGNode::Sequence(children) => {
            // Recurse non-Wait children; hold direct Wait Xfers aside
            // so we can decide per-Wait whether it stays here (its data
            // is produced in this sequence: an intra-scope rendezvous)
            // or bubbles up (loop-invariant, escape the enclosing
            // Repeat).
            enum Slot {
                Node(ACFGNode),
                Wait(XferPlaceholder),
            }
            let mut slots: Vec<(Slot, Vec<XferPlaceholder>)> = Vec::new();
            for child in children {
                match child {
                    ACFGNode::Xfer(x) if x.role == XferRole::Wait => {
                        slots.push((Slot::Wait(x), Vec::new()));
                    }
                    other => {
                        let (c2, esc) = hoist_invariant_waits(other, enclosing_tile);
                        slots.push((Slot::Node(c2), esc));
                    }
                }
            }

            // Data produced anywhere in this sequence (post-recursion).
            let mut produced = BTreeSet::new();
            for (slot, _) in &slots {
                if let Slot::Node(n) = slot {
                    produced_data_set(n, &mut produced);
                }
            }

            let mut out: Vec<ACFGNode> = Vec::new();
            let mut escaped_up: Vec<XferPlaceholder> = Vec::new();

            let place_or_bubble =
                |w: XferPlaceholder,
                 out: &mut Vec<ACFGNode>,
                 escaped_up: &mut Vec<XferPlaceholder>| {
                    if produced.contains(&w.data) {
                        // Lands here: rewrite tile to this sequence's
                        // enclosing-tile granularity and dedup against
                        // an already-placed equivalent Wait (keeps the
                        // pass idempotent on re-run — the regenerated
                        // Wait carries a fresh seq, but the structural
                        // (src,dst,data) key is stable, so we keep the
                        // first and drop the duplicate).
                        let mut w = w;
                        w.tile = IterTile::new(enclosing_tile.to_vec());
                        let dup = out.iter().any(|n| {
                            matches!(n, ACFGNode::Xfer(x)
                                if x.role == XferRole::Wait
                                    && x.src == w.src
                                    && x.dst == w.dst
                                    && x.data == w.data)
                        });
                        if !dup {
                            out.push(ACFGNode::Xfer(w));
                        }
                    } else {
                        escaped_up.push(w);
                    }
                };

            for (slot, esc) in slots {
                // Escaped Waits from this child (a Repeat) are placed
                // immediately *before* the child.
                for w in esc {
                    place_or_bubble(w, &mut out, &mut escaped_up);
                }
                match slot {
                    Slot::Node(n) => out.push(n),
                    Slot::Wait(x) => {
                        if produced.contains(&x.data) {
                            // Intra-scope rendezvous: producer is a
                            // sibling here. Keep the Wait where it is.
                            out.push(ACFGNode::Xfer(x));
                        } else {
                            // Loop-invariant w.r.t. the Repeat that
                            // encloses this sequence: hoist it out.
                            escaped_up.push(x);
                        }
                    }
                }
            }

            (ACFGNode::Sequence(out), escaped_up)
        }
    }
}

// --------------------------------------------------------------------
// Pass B — global Push finaliser (cross-scope)
// --------------------------------------------------------------------
//
// After Pass A every Wait is positioned. Each Wait still needs a
// matching Push after its producer. `splice_pushes_for_waits` already
// did this for in-sequence pairs during the single-pass walk; this
// pass finalises the cross-scope ones it could not see, idempotently
// (keyed on the unique `seq`, with a structural (src,dst,data) guard).
//
// Whole-symbol placement rule, given producer P of D and the Wait W:
//
//   * If P sits inside one or more Repeats that W is NOT inside, the
//     Push goes immediately after the *outermost* such Repeat (D is a
//     loop output, available once the loop completes — the dual of
//     Pass A's loop-input hoist). Example 02: `c` produced by `add`
//     inside `for i`, read by `save_output` after the loop.
//
//   * Otherwise the Push goes immediately after P itself (same scope,
//     or P and W co-resident in the same loop = per-iteration
//     rendezvous).

/// Outer→inner list of Repeat iter-vars enclosing the (unique,
/// single-assignment) producer Operation of `data`.
fn producer_repeat_path(
    node: &ACFGNode,
    data: DataId,
    path: &mut Vec<IterVar>,
    found: &mut Option<Vec<IterVar>>,
) {
    if found.is_some() {
        return;
    }
    match node {
        ACFGNode::Operation(op) => {
            if output_data(op) == Some(data) {
                *found = Some(path.clone());
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
        ACFGNode::Repeat { iter_var, body, .. } => {
            path.push(*iter_var);
            producer_repeat_path(body, data, path, found);
            path.pop();
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                producer_repeat_path(c, data, path, found);
            }
        }
    }
}

/// Outer→inner list of Repeat iter-vars enclosing the Wait carrying
/// `seq`.
fn wait_repeat_path(
    node: &ACFGNode,
    seq: SeqTag,
    path: &mut Vec<IterVar>,
    found: &mut Option<Vec<IterVar>>,
) {
    if found.is_some() {
        return;
    }
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Xfer(x) => {
            if x.role == XferRole::Wait && x.seq == seq {
                *found = Some(path.clone());
            }
        }
        ACFGNode::Repeat { iter_var, body, .. } => {
            path.push(*iter_var);
            wait_repeat_path(body, seq, path, found);
            path.pop();
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                wait_repeat_path(c, seq, path, found);
            }
        }
    }
}

fn subtree_produces(node: &ACFGNode, data: DataId) -> bool {
    let mut s = BTreeSet::new();
    produced_data_set(node, &mut s);
    s.contains(&data)
}

/// Insert `push` immediately after the (unique) producer Operation of
/// `push.data` wherever it directly resides.
fn splice_after_producer(node: ACFGNode, push: &XferPlaceholder) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out = Vec::with_capacity(children.len() + 1);
            for c in children {
                let is_producer = matches!(&c, ACFGNode::Operation(op)
                    if output_data(op) == Some(push.data));
                let c = splice_after_producer(c, push);
                out.push(c);
                if is_producer {
                    out.push(ACFGNode::Xfer(push.clone()));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_producer(*body, push)),
            block_tag,
        },
        leaf => leaf,
    }
}

/// Insert `push` immediately after the Repeat whose iter-var is
/// `cut_iv` and which (transitively) produces `push.data`.
fn splice_after_repeat(node: ACFGNode, cut_iv: IterVar, push: &XferPlaceholder) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out = Vec::with_capacity(children.len() + 1);
            for c in children {
                let is_cut = matches!(&c, ACFGNode::Repeat { iter_var, .. }
                    if *iter_var == cut_iv && subtree_produces(&c, push.data));
                if is_cut {
                    out.push(c);
                    out.push(ACFGNode::Xfer(push.clone()));
                } else {
                    out.push(splice_after_repeat(c, cut_iv, push));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_repeat(*body, cut_iv, push)),
            block_tag,
        },
        leaf => leaf,
    }
}

fn collect_push_seqs(node: &ACFGNode, seqs: &mut BTreeSet<u64>) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Push => {
            seqs.insert(x.seq.0);
        }
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => collect_push_seqs(body, seqs),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_push_seqs(c, seqs);
            }
        }
    }
}

/// Collect Waits eligible for whole-symbol Push finalisation.
/// TASK-0267: per-Wait classification — every Wait in the tree is
/// eligible; the per-Wait stay-vs-bubble decision in Pass A
/// (`hoist_invariant_waits`) has already placed each Wait at the
/// scope where its `data` becomes producer-relative, so Pass B's
/// producer-path / wait-path cut handles all shapes uniformly.
fn collect_waits(node: &ACFGNode, out: &mut Vec<XferPlaceholder>) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Wait => out.push(x.clone()),
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => {
            collect_waits(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_waits(c, out);
            }
        }
    }
}

fn splice_pushes_global(mut root: ACFGNode, name_data: &BTreeMap<String, DataId>) -> ACFGNode {
    let mut have_seqs: BTreeSet<u64> = BTreeSet::new();
    collect_push_seqs(&root, &mut have_seqs);

    let mut waits: Vec<XferPlaceholder> = Vec::new();
    collect_waits(&root, &mut waits);

    for w in waits {
        // Idempotence keyed on `seq` ALONE — deliberately not on
        // (src,dst,data). `seq` is unique per Push/Wait pair (global
        // monotonic counter, see the SeqTag note in `inject_transfers`),
        // so "a Push with this seq exists" is the exact "this transfer
        // is already paired" predicate.
        //
        // We must NOT also skip on (src,dst,data): single-assignment
        // data can have *several* cross-worker consumers on the same
        // dst worker (e.g. `d` produced on host, read by two distinct
        // Operations on w0). Each consumer gets its own Wait with its
        // own seq and its own seq-keyed buffer place in the Petri
        // lowering; suppressing the second because (host,w0,d) was
        // already seen would leave its buffer place unfilled and
        // deadlock that consumer. Idempotence on re-run is still
        // guaranteed because Pass A collapses the regenerated
        // fresh-seq duplicate Wait against the surviving original
        // (by (src,dst,data) at the destination sequence) BEFORE this
        // pass runs, so only the original seq reaches here and its
        // Push is already in `have_seqs`.
        if have_seqs.contains(&w.seq.0) {
            continue;
        }

        // Locate producer and its loop nesting vs the Wait's.
        let mut pp = None;
        producer_repeat_path(&root, w.data, &mut Vec::new(), &mut pp);
        let producer_path = match pp {
            Some(p) => p,
            // DEFENSE-IN-DEPTH behind the root-boundary check in
            // `inject_transfers` (the `escaped_at_root` panic). That
            // check should catch every producerless cross-worker Wait
            // BEFORE this pass runs: a Wait with no producing scope
            // bubbles out of Pass A's hoist and is rejected there.
            // This branch is therefore *expected* unreachable in
            // practice — but it is NOT proven unreachable: the two
            // guards key off different structures (Pass A's
            // whole-symbol-hoist root residue vs a per-`w.data`
            // producer walk of the post-hoist tree), and a future
            // hoist/block-transform change could let a Wait reach here
            // unpaired. Keep it as a loud `panic!` (not `unreachable!`,
            // which would assert a proof we do not have; not
            // `continue`, which would resurrect the silent-drop bug).
            // TASK-0152.
            //
            // Invariant rationale: a cross-worker Wait only exists
            // because `build_waits_for_op` found this symbol in
            // `producers_by_data` (mirrors `linked.data_producers`);
            // therefore a producing Operation MUST exist somewhere in
            // the ACFG (`build_acfg` places the producing kernel).
            // `None` here means the ACFG is malformed — a Wait whose
            // producer kernel was never lowered to an Operation. Fail
            // loud with full context per the `acfg.rs` fail-fast
            // precedent rather than silently leaving an unpaired Wait
            // for a downstream pass to mis-diagnose as a deadlock.
            None => {
                let data_name = name_data
                    .iter()
                    .find_map(|(n, id)| (*id == w.data).then_some(n.as_str()))
                    .unwrap_or("<unknown>");
                panic!(
                    "transfer_inject invariant violation: cross-worker Wait \
                     for data `{data_name}` (id {:?}, seq {:?}, {:?} -> {:?}) \
                     has no producing Operation in the ACFG. A Wait is only \
                     emitted when the schedule records a producer for the \
                     symbol, so this is a malformed ACFG (producer kernel not \
                     lowered), not a partial test input.",
                    w.data, w.seq, w.src, w.dst
                );
            }
        };

        // Single-assignment invariant (PRD §6.2.1, TASK-0153):
        // `producer_repeat_path` takes the FIRST Operation in walk
        // order that writes `w.data`. That is only well-defined if
        // there is exactly one. v2 data is single-assignment, so two
        // writers would be a front-end/lowering bug that would
        // mis-place the Push silently. Assert it in debug builds; no
        // release-build cost.
        debug_assert_eq!(
            count_producers(&root, w.data),
            1,
            "single-assignment violated: data id {:?} has multiple producing \
             Operations; producer_repeat_path would mis-place the Push",
            w.data
        );

        let mut wp = None;
        wait_repeat_path(&root, w.seq, &mut Vec::new(), &mut wp);
        let wait_path = wp.unwrap_or_default();

        let push = XferPlaceholder {
            role: XferRole::Push,
            src: w.src,
            dst: w.dst,
            data: w.data,
            tile: w.tile.clone(),
            seq: w.seq,
            policy: w.policy,
        };

        // The cut: outermost Repeat enclosing the producer that does
        // NOT also enclose the Wait. If present, the producer is a
        // loop output the consumer reads after the loop -> Push goes
        // after that Repeat. Otherwise Push goes right after producer.
        let cut = producer_path
            .iter()
            .find(|iv| !wait_path.contains(iv))
            .copied();

        root = match cut {
            Some(cut_iv) => splice_after_repeat(root, cut_iv, &push),
            None => splice_after_producer(root, &push),
        };

        have_seqs.insert(w.seq.0);
    }

    root
}

// --------------------------------------------------------------------
// TASK-0117 per-worker tile finalisation
// --------------------------------------------------------------------

/// Walk the ACFG and rewrite every `Xfer`'s `tile` to its
/// fan-out compute worker's partition slice.
///
/// "Compute worker" rule:
/// - If `src` appears in `partition_ranges` for any iter-var:
///   compute_worker = src (the N:1 gather direction, e.g. output).
/// - Else if `dst` appears: compute_worker = dst (the 1:N broadcast
///   direction, e.g. input).
/// - Else: no partition involvement — leave the tile unchanged
///   (the 1:1 host↔single-worker shape from examples 01..07).
///
/// The new tile is built by iterating the partitioned iter-vars in
/// NEST ORDER (outer-to-inner) and appending
/// `(iv, partition_ranges[iv][compute_worker])` for each iv where the
/// compute worker has a partition entry. The nest order itself is
/// derived from the ACFG topology via a single DFS pre-order pass over
/// the `Repeat` nodes — outer Repeats are visited before their
/// children — restricted to iter-vars that have a `partition_ranges`
/// entry. The resulting `bounds` is naturally OUTER-to-INNER, the
/// convention load-bearing for downstream passes
/// (`annotate_pipeline_depth_for_seq` walks `bounds.iter().rev()` for
/// "innermost wins").
///
/// TASK-0224 replaced an earlier per-Xfer `for (iv, ...) in
/// partition_ranges` loop that iterated in BTreeMap key-ascending =
/// IterVar-id ascending order. That coincided with nest order for
/// schedules where outer-loop iter-vars happened to have lower ids
/// (every in-tree schedule today) but is not guaranteed by the
/// `IterTile::bounds` convention. Pinned by
/// `rewrite_partition_tiles_bounds_in_nest_order_not_itervar_id_order`
/// and `rewrite_partition_tiles_three_level_nest_order` (synthetic
/// fixtures with non-monotonic IterVar ids).
///
/// Why we derive nest order from the ACFG rather than from the live
/// "enclosing-Repeat stack" at the Xfer's current position: by the
/// time this pass runs, `hoist_invariant_waits` has already moved
/// loop-invariant whole-symbol Waits OUT of their birth-position
/// Repeats (TASK-0151). For a fan-out broadcast like `host -> {w1..w4}`
/// over a partitioned `for n`, the Wait now sits at top-level, with an
/// empty live-stack — but we still want the per-worker partition slice
/// of `for n` recorded in its tile (the test
/// `transfer_fanout_composes_with_partition_sidecar` pins exactly
/// that). The Repeat node still exists in the ACFG, just at a sibling
/// position; a DFS pre-order over `Repeat::iter_var` captures the
/// program-wide nest order regardless of where any one Xfer happens
/// to have been hoisted to.
///
/// Determinism: DFS pre-order is a pure function of the ACFG; the
/// resulting `partition_axis_order` is the same for the same input.
///
/// We do this AFTER `hoist_invariant_waits` + `splice_pushes_global`
/// rather than at `build_waits_for_op` time because the hoist
/// rewrites the tile to the post-hoist enclosing tile (TASK-0151), so
/// a tile set at construction time would be clobbered for any Wait
/// hoisted out of the partitioned loop body. Setting the tile here
/// is the single sink for the contract.
fn rewrite_partition_tiles(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
) -> ACFGNode {
    if partition_ranges.is_empty() {
        return node;
    }
    // Derive the program-wide nest order of partitioned iter-vars via
    // a DFS pre-order over `Repeat::iter_var`, filtered to those in
    // `partition_ranges`. Outer Repeats appear before their children,
    // so the resulting Vec is OUTER-to-INNER. The same iter-var
    // appearing twice (e.g. via `block_transform` strip-mining that
    // reuses an IterVar across a full/partial split) is recorded only
    // on its first visit — the partition slice is keyed per iter-var,
    // and a second copy at deeper nest would double the bounds entry
    // for the same logical axis.
    let mut partition_axis_order: Vec<IterVar> = Vec::new();
    collect_partitioned_iter_var_nest_order(&node, partition_ranges, &mut partition_axis_order);
    rewrite_partition_tiles_inner(node, partition_ranges, &partition_axis_order)
}

/// DFS pre-order walk of the ACFG recording each `Repeat::iter_var`
/// that has a `partition_ranges` entry, OUTER-to-INNER. Each iter-var
/// is recorded at most once even if a `Repeat` for the same `IterVar`
/// appears multiple times (e.g. strip-mined full/partial split sharing
/// one `IterVar`); first-occurrence wins, which is the outermost
/// occurrence under DFS pre-order.
fn collect_partitioned_iter_var_nest_order(
    node: &ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    out: &mut Vec<IterVar>,
) {
    match node {
        ACFGNode::Repeat { iter_var, body, .. } => {
            if partition_ranges.contains_key(iter_var) && !out.contains(iter_var) {
                out.push(*iter_var);
            }
            collect_partitioned_iter_var_nest_order(body, partition_ranges, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_partitioned_iter_var_nest_order(c, partition_ranges, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Recursive worker for [`rewrite_partition_tiles`].
///
/// `partition_axis_order` is the OUTER-to-INNER nest-order vec of
/// partitioned iter-vars precomputed by the caller; we consult it to
/// keep `IterTile::bounds` in nest order regardless of where the Xfer
/// currently sits in the tree (post-hoist Xfers may live outside their
/// birth-position Repeats — see [`rewrite_partition_tiles`] doc).
fn rewrite_partition_tiles_inner(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    partition_axis_order: &[IterVar],
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Pick the compute worker per the rule above.
            let compute_worker = if partition_ranges.values().any(|m| m.contains_key(&x.src)) {
                Some(x.src)
            } else if partition_ranges.values().any(|m| m.contains_key(&x.dst)) {
                Some(x.dst)
            } else {
                None
            };
            if let Some(w) = compute_worker {
                // Iterate the precomputed OUTER-to-INNER nest-order
                // axis vec and append the per-worker partition slice
                // for each axis where this compute worker has an
                // entry. Replaces a pre-TASK-0224 BTreeMap-key-order
                // iteration that coincidentally produced nest order
                // only because every in-tree schedule's outer
                // iter-vars happened to have lower IterVar ids.
                let mut bounds: Vec<(IterVar, std::ops::Range<i64>)> = Vec::new();
                for iv in partition_axis_order {
                    if let Some(per_worker) = partition_ranges.get(iv) {
                        if let Some(range) = per_worker.get(&w) {
                            bounds.push((*iv, range.clone()));
                        }
                    }
                }
                if !bounds.is_empty() {
                    x.tile = IterTile::new(bounds);
                }
            }
            ACFGNode::Xfer(x)
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| rewrite_partition_tiles_inner(c, partition_ranges, partition_axis_order))
                .collect(),
        ),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(rewrite_partition_tiles_inner(
                *body,
                partition_ranges,
                partition_axis_order,
            )),
            block_tag,
        },
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// TASK-0263 Stage 2 halo extension
// --------------------------------------------------------------------

/// Extend each [`XferPlaceholder`]'s `tile` axis range by the halo
/// width inferred for `(consumer_kernel, iter_var)` along that axis
/// (TASK-0263 Stage 2).
///
/// ### Why this is the right sink for halo widths
///
/// By the time `rewrite_partition_tiles` has run, every Xfer's tile
/// reflects the COMPUTE worker's partition slice (or the source range
/// when no partition applies). The halo is an additive correction:
/// "the kernel reading the slice ALSO reads `halo` rows of
/// neighbouring data". Applied here, the halo extension composes with
/// any partition policy (rows/blocks2d/workers) without each policy
/// having to know about halo.
///
/// ### Sidecar shape and the (DataId -> consumer KernelIds) join
///
/// `halo_widths` is keyed by `(KernelId, IterVar) -> u64`. An
/// XferPlaceholder carries `(DataId, IterVar)` in its tile. We bridge
/// the two by walking the ACFG once to collect, for each `DataId`, the
/// set of kernel ids that READ it (the `data_in` of every
/// `DataflowEdge`). The halo on a `(DataId, IterVar)` is then the
/// MAX of `halo_widths[kid][iv]` across all consumer kids — the union
/// of every reader's halo requirement. This is the only sound merge:
/// each transfer is one slice, used by every consumer at the
/// destination; the slice must cover the union of reads.
///
/// In practice every in-tree example has at most one consumer kernel
/// per data symbol; the max-across-consumers reduces to the single
/// entry. The variant is documented for correctness when a future
/// algorithm reads the same symbol from two different kernels.
///
/// ### Clamp policy
///
/// Extension is `lo' = lo - halo, hi' = hi + halo`. We clamp against
/// the iter-var's algorithm-source loop range expanded by halo —
/// `[source_lo - halo, source_hi + halo)`. That bounds the legitimate
/// data extent the loop would read in absence of partitioning. For
/// schedules where the per-worker band already abuts the source range
/// (the corner workers), the clamp is a no-op; for non-corner workers
/// it is always a no-op (the extension stays strictly inside the source
/// range expanded by halo).
///
/// Why not clamp against the data symbol's `ResolvedType::dims`: that
/// is the right semantic for a STRICTER clamp (don't read beyond data
/// extent), and is a natural follow-up. For every in-tree example, the
/// algorithm declares data wide enough that the (loop range ± halo)
/// stays within the data extent, so the looser clamp here is correct.
/// Filed as a future tightening on TASK-0263.
///
/// ### Bare-iv halo = 0 contract (forward-carry from TASK-0260)
///
/// `halo_widths` records an explicit `0` entry for every
/// `(kernel, iv)` the detector inspected with a bare-iv index. We
/// treat `0` as "no extension needed" (the natural arithmetic
/// identity). This honours the TASK-0260 Stage 1 lenient contract.
///
/// ### No-op when `halo_widths` is empty
///
/// An empty sidecar — every pre-Stage-2 example — short-circuits at
/// the top: no walk, no allocation, the input is returned unchanged.
fn extend_xfer_tiles_for_halo(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
) -> ACFGNode {
    if halo_widths.is_empty() {
        return node;
    }
    // Build the (DataId -> Set<KernelId>) consumer index by walking
    // the ACFG once. Operations' DataflowEdge::data_in lists every
    // DataId the edge.kernel reads.
    let mut consumers: BTreeMap<DataId, BTreeSet<KernelId>> = BTreeMap::new();
    collect_data_consumers(&node, &mut consumers);

    // Build the (IterVar -> source-loop-range) index by walking the
    // ACFG's Repeat nodes. The first occurrence wins (matches
    // `rewrite_partition_tiles`'s nest-order convention); a future
    // strip-mined inner Repeat that reuses the same IterVar would not
    // overwrite the outer source range.
    let mut source_ranges: BTreeMap<IterVar, std::ops::Range<i64>> = BTreeMap::new();
    collect_iter_var_source_ranges(&node, &mut source_ranges);

    extend_xfer_tiles_inner(node, halo_widths, &consumers, &source_ranges)
}

/// Walk `node` and union every `Operation`'s `data_in` into the
/// `(DataId -> KernelId)` consumer index. Read-only walk.
///
/// The kernel id we attribute to a read is the EDGE's kernel
/// (`edge.kernel`), not the enclosing `Operation.kernel` — they are
/// the same in the M1 single-edge-per-Operation shape, but explicit
/// is safer if a future multi-edge DAG lands.
fn collect_data_consumers(node: &ACFGNode, out: &mut BTreeMap<DataId, BTreeSet<KernelId>>) {
    match node {
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                for d in &edge.data_in {
                    out.entry(*d).or_default().insert(edge.kernel);
                }
            }
        }
        ACFGNode::Repeat { body, .. } => collect_data_consumers(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_data_consumers(c, out);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Walk `node` and record, for each `Repeat::iter_var`, its source
/// `range`. First occurrence wins. Read-only walk.
///
/// "First" is the outermost occurrence under DFS pre-order. A future
/// `block_transform` strip-mine that reuses an IterVar across full +
/// partial split would land its inner-tile Repeats deeper in the tree;
/// taking the outer occurrence keeps the halo clamp anchored on the
/// SOURCE loop range (the algorithm's `for y : 1..H-1`), not on a
/// downstream-synthesised inner-tile range.
fn collect_iter_var_source_ranges(
    node: &ACFGNode,
    out: &mut BTreeMap<IterVar, std::ops::Range<i64>>,
) {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            ..
        } => {
            out.entry(*iter_var).or_insert_with(|| range.clone());
            collect_iter_var_source_ranges(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_iter_var_source_ranges(c, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Recursive worker for [`extend_xfer_tiles_for_halo`].
fn extend_xfer_tiles_inner(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    consumers: &BTreeMap<DataId, BTreeSet<KernelId>>,
    source_ranges: &BTreeMap<IterVar, std::ops::Range<i64>>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Look up the consumer kernels for this data symbol. If
            // none recorded (the data is never read in the ACFG —
            // e.g. an output written but not re-consumed), no halo
            // applies. Skip.
            let Some(kernel_ids) = consumers.get(&x.data) else {
                return ACFGNode::Xfer(x);
            };
            let mut new_bounds: Vec<(IterVar, std::ops::Range<i64>)> =
                Vec::with_capacity(x.tile.bounds.len());
            for (iv, range) in &x.tile.bounds {
                // Max halo across consumer kernels for this iv.
                let halo: u64 = kernel_ids
                    .iter()
                    .filter_map(|kid| halo_widths.get(kid))
                    .filter_map(|per_iv| per_iv.get(iv))
                    .copied()
                    .max()
                    .unwrap_or(0);
                if halo == 0 {
                    new_bounds.push((*iv, range.clone()));
                    continue;
                }
                // halo is u64; convert to i64. For all in-tree
                // examples halo widths are small (1 to a few), so the
                // overflow path is academic — we still guard with
                // try_into + saturating fallback to keep the function
                // total.
                let halo_i: i64 = halo.try_into().unwrap_or(i64::MAX);
                // Extend.
                let ext_lo = range.start.saturating_sub(halo_i);
                let ext_hi = range.end.saturating_add(halo_i);
                // Clamp to the iter-var's source loop range expanded by
                // halo. (See module docs for the rationale on this
                // looser clamp vs the data-extent clamp.)
                let (clamp_lo, clamp_hi) = match source_ranges.get(iv) {
                    Some(src) => (
                        src.start.saturating_sub(halo_i),
                        src.end.saturating_add(halo_i),
                    ),
                    // No source range recorded for this iv — happens
                    // if an Xfer carries an iv that has no Repeat in
                    // the ACFG (synthetic test fixtures). Skip the
                    // clamp; the extension is still applied.
                    None => (i64::MIN, i64::MAX),
                };
                let new_lo = ext_lo.max(clamp_lo);
                let new_hi = ext_hi.min(clamp_hi);
                new_bounds.push((*iv, new_lo..new_hi));
            }
            x.tile = IterTile::new(new_bounds);
            ACFGNode::Xfer(x)
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| extend_xfer_tiles_inner(c, halo_widths, consumers, source_ranges))
                .collect(),
        ),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(extend_xfer_tiles_inner(
                *body,
                halo_widths,
                consumers,
                source_ranges,
            )),
            block_tag,
        },
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// Wait construction
// --------------------------------------------------------------------

/// For each cross-worker read in `op`, produce a Wait placeholder.
/// Multiple distinct reads of the same data symbol in one op yield
/// ONE Wait — the consumer needs one rendezvous per (op, data, src)
/// triple, not one per read.
fn build_waits_for_op(
    op: &Operation,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
) -> Vec<XferPlaceholder> {
    let consumer_workers: BTreeSet<WorkerId> = op.workers.clone();
    let mut seen: BTreeSet<DataId> = BTreeSet::new();
    let mut out = Vec::new();

    for edge in &op.dataflow.edges {
        for &data_id in &edge.data_in {
            if !seen.insert(data_id) {
                continue;
            }
            let producer_workers = match ctx.producers_by_data.get(&data_id) {
                Some(p) => p,
                None => continue, // No recorded producer (e.g. unread const data).
            };
            if producer_workers == &consumer_workers {
                continue; // Same entity — intra-worker dataflow.
            }
            let policy = ctx
                .policies_by_data
                .get(&data_id)
                .copied()
                .unwrap_or_default();

            // TASK-0117 fan-out: emit one Wait per (src-worker,
            // dst-worker) member of the cartesian product of the
            // producer and consumer worker entities, skipping any
            // same-worker pair. Each pair gets its own fresh `seq` so
            // the projection's per-worker EventLists carry distinct
            // Pushes/Waits, and the backend can allocate per-pair
            // slots without collision.
            //
            // The pair's `tile` is the enclosing iteration tile with
            // any partitioned axis (TASK-0212) rewritten to the
            // *compute* worker's slice. The "compute worker" for a
            // pair is the worker in the multi-worker side: for a
            // (host -> {w0..w3}) pair to w_i, compute = w_i; for a
            // ({w0..w3} -> host) pair from w_i, compute = w_i. When
            // both sides are size 1, the source range survives
            // unchanged (no partition meaning) — this is exactly the
            // pre-TASK-0117 behaviour for {host} -> {single worker}
            // transfers.
            //
            // Determinism: BTreeSet iterates in sorted WorkerId
            // order; the cartesian-product enumeration is therefore
            // a deterministic function of (producer_workers,
            // consumer_workers). Seq tags are assigned monotonically
            // by `state.fresh_seq()` — same input ⇒ same seqs.
            // Emit one Wait per (src, dst) member of the cartesian
            // product. The per-pair `tile` is initialised here from
            // `enclosing_tile` (the construction-site semantic the
            // pre-TASK-0117 single-pair path used); the final
            // per-worker tile is set by `rewrite_partition_tiles`
            // at the end of `inject_transfers`, after the hoist +
            // splice passes have positioned the placeholders. Doing
            // the partition rewrite as a separate sink keeps the
            // hoist invariant (TASK-0151's loop-invariant tile
            // rewrite) untouched.
            for &src in producer_workers.iter() {
                for &dst in consumer_workers.iter() {
                    if src == dst {
                        continue;
                    }
                    out.push(XferPlaceholder {
                        role: XferRole::Wait,
                        src,
                        dst,
                        data: data_id,
                        tile: IterTile::new(enclosing_tile.to_vec()),
                        seq: state.fresh_seq(),
                        policy,
                    });
                }
            }
        }
    }

    out
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Convert a [`WorkerEntity`] (BTreeSet<String>) to a
/// `BTreeSet<WorkerId>` using the ACFG's name table. Skip names that
/// aren't in the table — that would be a link-pass invariant
/// violation; we don't loudly panic so downstream tests that build
/// synthetic ACFGs can still feed an empty/partial name table.
fn entity_to_workerid_set(
    entity: &WorkerEntity,
    name_workers: &BTreeMap<String, WorkerId>,
) -> BTreeSet<WorkerId> {
    entity
        .0
        .iter()
        .filter_map(|name| name_workers.get(name).copied())
        .collect()
}

/// Convert a [`ResolvedTransferDirective`]'s option list into a
/// [`TransferPolicy`]. Multiple options in one directive are walked
/// in order; later options override earlier ones for the same field.
///
/// The defaults match [`TransferPolicy::default`]: synchronous, buffer
/// 1, notify-default.
fn policy_from_directive(dir: &ResolvedTransferDirective) -> TransferPolicy {
    let mut p = TransferPolicy::default();
    for opt in &dir.options {
        match opt {
            ResolvedTransferOption::Sync => p.synchronous = true,
            ResolvedTransferOption::Async => p.synchronous = false,
            ResolvedTransferOption::Buffer(n) => p.buffer = *n,
            ResolvedTransferOption::Notify(k) => p.notify = NotifyMode::from(*k),
        }
    }
    p
}

/// Return the data symbol this Operation writes, if any.
/// At M1 a [`crate::acfg::DataflowDag`] holds one edge per firing; we
/// take the first edge's `data_out`. If a future multi-edge DAG lands,
/// this helper widens to a `Vec<DataId>` and callers adjust.
fn output_data(op: &Operation) -> Option<DataId> {
    op.dataflow.edges.first().and_then(|e| e.data_out)
}

/// Update the `last_writer` map with `op`'s output, if any.
fn update_writer(state: &mut State, op: &Operation) {
    if let Some(d) = output_data(op) {
        state.last_writer.insert(d, op.workers.clone());
    }
}

/// True iff `prev` is an `Xfer` placeholder whose `(role, src, dst,
/// data, tile)` matches `cand`. Used for idempotence: re-running the
/// pass must not duplicate an already-present placeholder.
fn is_duplicate_xfer(prev: Option<&ACFGNode>, cand: &XferPlaceholder) -> bool {
    match prev {
        Some(ACFGNode::Xfer(existing)) => {
            existing.role == cand.role
                && existing.src == cand.src
                && existing.dst == cand.dst
                && existing.data == cand.data
                && existing.tile == cand.tile
        }
        _ => false,
    }
}

// --------------------------------------------------------------------
// TASK-0289 cycle 114a halo-strip Push/Wait synthesis
// --------------------------------------------------------------------

/// Synthesise cross-worker halo-strip Push/Wait pairs between N/S/E/W
/// neighbour cells in a 2D worker grid established by
/// `partition=blocks2d` (TASK-0289 AC#1).
///
/// ### Why this lives at the tail of `inject_transfers`
///
/// `rewrite_partition_tiles` walks every `Xfer` and unconditionally
/// rewrites the tile from the partition sidecar (compute-worker rule);
/// `extend_xfer_tiles_for_halo` walks every `Xfer` and extends each tile
/// axis by the consumer kernel's halo width. Both rewrite in-place. A
/// synthesised halo-strip transfer carries a precisely-bounded
/// `[lo .. hi)` per axis (the strip-side of the neighbour's band, NOT
/// the worker's own partition slice + halo); if it ran BEFORE those
/// passes, the partition rewrite would replace its tile with the
/// src-worker's full band and the halo extension would re-pad it from
/// the wrong base. Running AFTER lets the strip-tile survive verbatim.
///
/// ### Why we synthesise BOTH Push and Wait pre-paired (instead of
/// only the Wait and letting `splice_pushes_global` produce the Push)
///
/// `splice_pushes_global` finds the matching Push location by walking
/// for the data symbol's PRODUCER Operation in the ACFG — but for a
/// halo strip on `img_in` (host-loaded), the producer is the host's
/// `load_image`, not the neighbour worker. Letting splice infer the
/// Push location would mis-route the Push to the host's scope; the
/// per-worker projection then emits a Push on `host`, not on the
/// neighbour worker. Synthesising both endpoints with a shared
/// `SeqTag` BEFORE splice runs would also be wrong (splice already
/// ran above by this point in the chain). So we run here, post-splice,
/// and create the pair with a `state.fresh_seq()` SeqTag pulled from
/// the shared monotonic counter — guaranteeing no SeqTag collision
/// with any pair already in the tree.
///
/// ### Per-Wait routing
///
/// The per-worker projection (`crate::passes::petri_to_events::emit_xfer`)
/// emits a Push event into `x.src`'s EventList and a Wait event into
/// `x.dst`'s EventList. So placing both Push and Wait nodes in the
/// SAME ACFG `Sequence` is fine: each worker's EventList picks up only
/// the endpoint matching its WorkerId. We insert the pairs as
/// siblings prepended to whatever `Sequence` contains the outer
/// Repeat for `outer_iv`.
///
/// ### Strip tile math
///
/// For a worker at grid `(row, col)` with y-band `[y_lo, y_hi)`,
/// x-band `[x_lo, x_hi)`, and per-axis halo `h_y` / `h_x`:
///
/// - **N-strip** (received FROM `(row-1, col)`):
///   y in `[y_lo - h_y, y_lo)`, x in `[x_lo, x_hi)`
/// - **S-strip** (received FROM `(row+1, col)`):
///   y in `[y_hi, y_hi + h_y)`, x in `[x_lo, x_hi)`
/// - **W-strip** (received FROM `(row, col-1)`):
///   y in `[y_lo, y_hi)`, x in `[x_lo - h_x, x_lo)`
/// - **E-strip** (received FROM `(row, col+1)`):
///   y in `[y_lo, y_hi)`, x in `[x_hi, x_hi + h_x)`
///
/// Workers on an edge of the grid skip the off-grid neighbour.
/// Corners (NE/NW/SE/SW) are excluded in this first cut per the
/// TASK-0289 brief — the diagonal halo cell would require a separate
/// (worker, neighbour-diagonal) pair to be sound; filed as a follow-up.
///
/// ### Honest limitations (this cycle)
///
/// - **Placement**: pairs are PREPENDED to the parent Sequence that
///   contains the outer Repeat. For a single-pass stencil (no time
///   loop) this lands them at top-level. For a multi-pass / time-step
///   stencil, a future task should refine placement to "inside the
///   timestep Repeat, before the partitioned outer Repeat" so the
///   halo exchange fires per timestep. The current placement is
///   correct for the synthetic unit test and not yet exercised by
///   any shipped schedule (AC#3 short-circuit holds).
///
/// - **Not idempotent on re-run** when partition_pairs is non-empty.
///   On a re-run of `inject_transfers`, (1)
///   `rewrite_partition_tiles` clobbers strip tiles before this
///   synthesis sees them (its compute-worker rule replaces strip
///   with src's full partition slice) and (2)
///   `splice_pushes_for_waits` (inside `inject_in_sequence`)
///   splices a NEW Push for every halo-strip Wait it sees, because
///   the existing Pushes sit BEFORE the producer load Op (outside
///   its immediate-successor dedupe window). No production driver
///   path re-runs `inject_transfers`; the shipped-schedule
///   idempotence test (`tests/transfer_inject.rs::
///   idempotent_on_synthetic_two_worker_case`) stays green because
///   partition_pairs is empty for every shipped schedule and the
///   AC#3 guard short-circuits this entire function. Filed
///   forward-carried on TASK-0290.
///
/// - **No deduplication against existing halo Pushes**: if a later
///   pass adds halo pairs from a different join, two pairs for the
///   same `(src, dst, data, tile)` could co-exist. Today no such
///   later pass exists; the unit test asserts the exact pair count.
///
/// - **Policy**: each synthesised pair inherits the policy of its
///   data symbol from `policies_by_data` (the schedule's
///   `transfer D : ...` directive), falling back to
///   `TransferPolicy::default()` (sync, buffer=1, notify-default) when
///   the symbol has no directive. Matches the convention used by
///   `build_waits_for_op` for regular Push/Wait pairs.
///
/// ### Determinism
///
/// All iteration is over `BTreeMap` / `BTreeSet` (numeric order). The
/// per-axis emit order is fixed (N, S, W, E). SeqTags come from the
/// shared `state` counter in DFS-deterministic visit order. Same
/// input ⇒ byte-identical output.
fn inject_halo_strip_xfers(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    partition_pairs: &BTreeMap<IterVar, IterVar>,
    grid_shape_for_outer_iv: &BTreeMap<IterVar, (u32, u32)>,
    partition_worker_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    policies_by_data: &BTreeMap<DataId, TransferPolicy>,
    state: &mut State,
) -> ACFGNode {
    // AC#3 additive-only short-circuit. Every shipped schedule has an
    // empty `partition_pairs` (verified by the
    // `shipped_examples_without_blocks2d_leave_maps_empty` test in
    // `tests/sidecar_partition_blocks2d.rs`); the early return is the
    // structural guarantee that no existing Xfer count or shape
    // changes today.
    if partition_pairs.is_empty() {
        return node;
    }

    // Re-run dedupe is INTENTIONALLY OMITTED today. Per the comment
    // block in `tests/halo_strip_synth.rs` (search for IDEMPOTENCE /
    // RE-RUN CAVEAT), `inject_transfers` is not idempotent when
    // partition_pairs is non-empty: `rewrite_partition_tiles` clobbers
    // halo-strip tiles on a re-run, and `splice_pushes_for_waits`
    // splices a new Push for every halo-strip Wait it sees outside the
    // immediate-producer-neighbour dedupe window. Adding a 4-tuple
    // dedupe in this function alone would not restore structural
    // idempotence (the tile would still be wrong, and the new pushes
    // from splice_pushes_for_waits would still land). The full fix is
    // forward-carried to TASK-0290.
    //
    // No production code path re-runs `inject_transfers` on its own
    // output (the driver pipeline calls it once), so this is a
    // first-cut concession rather than a behavioural regression for
    // any shipped schedule.

    // Build (DataId -> Set<KernelId>) consumer index — same shape +
    // walk as `extend_xfer_tiles_for_halo`'s `collect_data_consumers`.
    // We need this to translate `halo_widths[kid][iv]` (keyed by
    // kernel id) into "which data symbols cross the halo boundary for
    // this consumer kernel".
    let mut consumers: BTreeMap<DataId, BTreeSet<KernelId>> = BTreeMap::new();
    collect_data_consumers(&node, &mut consumers);

    // For each (outer_iv, inner_iv) pair, build the list of
    // (data, src_worker, dst_worker, tile) halo-strip transfers to
    // synthesise. Group by outer_iv: each group is prepended to the
    // parent Sequence containing the outer Repeat for that outer_iv.
    //
    // `BTreeMap<IterVar, Vec<XferPlaceholder>>` so the insertion order
    // matches outer_iv numeric order — deterministic.
    let mut to_insert: BTreeMap<IterVar, Vec<XferPlaceholder>> = BTreeMap::new();

    for (&outer_iv, &inner_iv) in partition_pairs {
        // Look up the grid shape recorded by partition_blocks2d.
        let Some(&(grid_rows, grid_cols)) = grid_shape_for_outer_iv.get(&outer_iv) else {
            // partition_pairs entry without a matching grid_shape entry
            // would be a partition_blocks2d invariant violation. Skip
            // defensively: the AC#3 short-circuit covers the "no pairs"
            // case; this is the belt-and-braces guard for malformed
            // sidecars on the per-pair path.
            continue;
        };
        let grid_cols_usize = grid_cols as usize;
        let grid_rows_usize = grid_rows as usize;

        // Per-worker y-band (from outer_iv) + x-band (from inner_iv).
        let Some(per_worker_y) = partition_worker_ranges.get(&outer_iv) else {
            continue;
        };
        let Some(per_worker_x) = partition_worker_ranges.get(&inner_iv) else {
            continue;
        };
        // Workers in row-major order — matches partition_blocks2d's
        // assignment (`for (i, wid) in body_workers.iter().enumerate()
        // { row = i / cols; col = i % cols }`). BTreeMap key iteration
        // is numeric WorkerId order, equivalent to the BTreeSet
        // iteration partition_blocks2d used on the write side.
        let body_workers: Vec<WorkerId> = per_worker_y.keys().copied().collect();
        if body_workers.len() != grid_rows_usize * grid_cols_usize {
            // Defensive: the row/col inversion only makes sense when
            // body_workers.len() == grid_rows * grid_cols. A mismatch
            // is a partition_blocks2d invariant violation; skip rather
            // than synthesise wrong pairs.
            continue;
        }

        // For each consumer kernel with a halo entry on outer_iv or
        // inner_iv, find every data symbol that kernel consumes. That
        // is the set of data symbols needing a strip transfer.
        //
        // `data_with_halo_y[data] = h_y` is the y-axis halo for the
        // FIRST consumer kernel matching that data (single-consumer
        // case in every in-tree example; documented as a limit for
        // multi-consumer cases — same caveat the per-tile halo
        // extension carries).
        let mut data_with_halo_y: BTreeMap<DataId, u64> = BTreeMap::new();
        let mut data_with_halo_x: BTreeMap<DataId, u64> = BTreeMap::new();
        for (data, kids) in &consumers {
            for kid in kids {
                if let Some(per_iv) = halo_widths.get(kid) {
                    if let Some(&h) = per_iv.get(&outer_iv) {
                        if h > 0 {
                            // Take MAX across consumer kernels — same
                            // merge rule `extend_xfer_tiles_for_halo`
                            // uses. The map's `entry().and_modify().or_insert`
                            // gives "max across kids" naturally.
                            data_with_halo_y
                                .entry(*data)
                                .and_modify(|cur| *cur = (*cur).max(h))
                                .or_insert(h);
                        }
                    }
                    if let Some(&h) = per_iv.get(&inner_iv) {
                        if h > 0 {
                            data_with_halo_x
                                .entry(*data)
                                .and_modify(|cur| *cur = (*cur).max(h))
                                .or_insert(h);
                        }
                    }
                }
            }
        }

        // Union of data symbols touched on either axis. Iteration in
        // BTreeSet numeric order — deterministic.
        let mut all_halo_data: BTreeSet<DataId> = BTreeSet::new();
        all_halo_data.extend(data_with_halo_y.keys().copied());
        all_halo_data.extend(data_with_halo_x.keys().copied());

        // No data symbols with halo on either partitioned axis ⇒
        // nothing to synthesise for this pair. Skip without inserting
        // an empty group (`to_insert.entry(outer_iv).or_default()`
        // would needlessly noise the placement walk).
        if all_halo_data.is_empty() {
            continue;
        }

        let group = to_insert.entry(outer_iv).or_default();

        // For each worker, compute (row, col), enumerate cardinal
        // neighbours, synthesise one Push+Wait pair per (data,
        // direction).
        //
        // Direction emit order is FIXED as (N, S, W, E) — the exact
        // order is a stability contract for the unit test's pair-
        // counting predicate. SeqTags monotonically increase in that
        // visit order, matching the per-worker BTreeMap iteration.
        for (i, &this_w) in body_workers.iter().enumerate() {
            let row = i / grid_cols_usize;
            let col = i % grid_cols_usize;
            let Some(y_band) = per_worker_y.get(&this_w) else {
                continue;
            };
            let Some(x_band) = per_worker_x.get(&this_w) else {
                continue;
            };
            let (y_lo, y_hi) = (y_band.start, y_band.end);
            let (x_lo, x_hi) = (x_band.start, x_band.end);

            // Helper: build the XferPlaceholder pair for a single
            // (src_worker, strip-tile, data) and push both Push and
            // Wait into `group`. No re-run dedupe today — see the
            // re-run-caveat comment near the top of this function.
            let mut emit_pair =
                |src: WorkerId, dst: WorkerId, data: DataId, tile: IterTile, state: &mut State| {
                    let policy = policies_by_data.get(&data).copied().unwrap_or_default();
                    let seq = state.fresh_seq();
                    group.push(XferPlaceholder {
                        role: XferRole::Push,
                        src,
                        dst,
                        data,
                        tile: tile.clone(),
                        seq,
                        policy,
                    });
                    group.push(XferPlaceholder {
                        role: XferRole::Wait,
                        src,
                        dst,
                        data,
                        tile,
                        seq,
                        policy,
                    });
                };

            // ---- N neighbour: row-1, col. ----
            if row > 0 {
                let neighbour_idx = (row - 1) * grid_cols_usize + col;
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_y) = data_with_halo_y.get(&data) {
                        let h_y_i: i64 = h_y.try_into().unwrap_or(i64::MAX);
                        let tile = IterTile::new(vec![
                            (outer_iv, (y_lo - h_y_i)..y_lo),
                            (inner_iv, x_lo..x_hi),
                        ]);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- S neighbour: row+1, col. ----
            if row + 1 < grid_rows_usize {
                let neighbour_idx = (row + 1) * grid_cols_usize + col;
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_y) = data_with_halo_y.get(&data) {
                        let h_y_i: i64 = h_y.try_into().unwrap_or(i64::MAX);
                        let tile = IterTile::new(vec![
                            (outer_iv, y_hi..(y_hi + h_y_i)),
                            (inner_iv, x_lo..x_hi),
                        ]);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- W neighbour: row, col-1. ----
            if col > 0 {
                let neighbour_idx = row * grid_cols_usize + (col - 1);
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_x) = data_with_halo_x.get(&data) {
                        let h_x_i: i64 = h_x.try_into().unwrap_or(i64::MAX);
                        let tile = IterTile::new(vec![
                            (outer_iv, y_lo..y_hi),
                            (inner_iv, (x_lo - h_x_i)..x_lo),
                        ]);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- E neighbour: row, col+1. ----
            if col + 1 < grid_cols_usize {
                let neighbour_idx = row * grid_cols_usize + (col + 1);
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_x) = data_with_halo_x.get(&data) {
                        let h_x_i: i64 = h_x.try_into().unwrap_or(i64::MAX);
                        let tile = IterTile::new(vec![
                            (outer_iv, y_lo..y_hi),
                            (inner_iv, x_hi..(x_hi + h_x_i)),
                        ]);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }
        }
    }

    if to_insert.is_empty() {
        return node;
    }

    // Walk the ACFG once and prepend each group's pairs to the parent
    // Sequence containing the outer Repeat. The walker returns
    // (rewritten_node, set_of_outer_ivs_still_to_place); after the
    // walk, any outer_iv that did not match a Repeat in the tree is a
    // sidecar/tree mismatch we tolerate silently (a synthetic test or
    // a future composition where the partition_pairs entry survives a
    // tree restructure). The behaviour is: prepend at the closest
    // ancestor Sequence we can find; if none found, the synthesis is
    // a no-op for that pair.
    let mut to_insert_mut = to_insert;
    prepend_strip_pairs(node, &mut to_insert_mut)
}

/// Walk `node` and prepend each group's `Vec<XferPlaceholder>` to the
/// Sequence that contains the Repeat for the matching `outer_iv`.
///
/// "Prepend to the parent Sequence" semantics: we rewrite the Sequence
/// in-place, drain the matching group out of `to_insert`, and emit the
/// new Xfer nodes BEFORE the existing children. Groups not matched at
/// the top-level Sequence are tried recursively in the children.
///
/// `to_insert` is consumed: when an outer_iv's group is placed, its
/// entry is removed. After the walk, any entries left in `to_insert`
/// were unmatched — the synthesis is silently dropped for those (a
/// malformed tree-vs-sidecar disagreement we don't loudly panic on,
/// matching the defensive skip in the synthesis loop above).
fn prepend_strip_pairs(
    node: ACFGNode,
    to_insert: &mut BTreeMap<IterVar, Vec<XferPlaceholder>>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            // Find every direct child Repeat whose iter_var is a key
            // in `to_insert`; collect the placeholders to prepend
            // BEFORE the existing children. Multiple matching outer_ivs
            // at the same Sequence drain in BTreeMap order
            // (deterministic).
            let mut prepend: Vec<XferPlaceholder> = Vec::new();
            for child in &children {
                if let ACFGNode::Repeat { iter_var, .. } = child {
                    if let Some(group) = to_insert.remove(iter_var) {
                        prepend.extend(group);
                    }
                }
            }
            // Recurse into each child so a nested partitioned Repeat
            // can drain its own group. Recurse FIRST, then assemble:
            // recursing on already-rewritten siblings is unnecessary
            // because we drained any outer-iv group BEFORE the
            // recursion; the recursion handles only groups whose
            // matching Repeat lies strictly DEEPER in the tree.
            let rewritten_children: Vec<ACFGNode> = children
                .into_iter()
                .map(|c| prepend_strip_pairs(c, to_insert))
                .collect();
            let mut out: Vec<ACFGNode> = Vec::with_capacity(prepend.len() + rewritten_children.len());
            for p in prepend {
                out.push(ACFGNode::Xfer(p));
            }
            out.extend(rewritten_children);
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // A bare Repeat (not wrapped in a Sequence) whose iter_var
            // matches a group: wrap it. In practice `build_acfg` always
            // wraps top-level statements in a Sequence, so this branch
            // is defensive; recursing into the body lets deeper
            // matches still drain.
            let new_body = Box::new(prepend_strip_pairs(*body, to_insert));
            ACFGNode::Repeat {
                iter_var,
                range,
                body: new_body,
                block_tag,
            }
        }
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// Inspection helpers (used by tests)
// --------------------------------------------------------------------

impl ACFGNode {
    /// Count [`ACFGNode::Xfer`] nodes anywhere in this subtree.
    /// Sister to `count_operations` / `count_syncs`.
    pub fn count_xfers(&self) -> usize {
        match self {
            ACFGNode::Xfer(_) => 1,
            ACFGNode::Repeat { body, .. } => body.count_xfers(),
            ACFGNode::Sequence(children) => children.iter().map(ACFGNode::count_xfers).sum(),
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => 0,
        }
    }

    /// Count [`ACFGNode::Xfer`] nodes of a specific role.
    pub fn count_xfers_role(&self, role: XferRole) -> usize {
        match self {
            ACFGNode::Xfer(x) => {
                if x.role == role {
                    1
                } else {
                    0
                }
            }
            ACFGNode::Repeat { body, .. } => body.count_xfers_role(role),
            ACFGNode::Sequence(children) => children.iter().map(|c| c.count_xfers_role(role)).sum(),
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => 0,
        }
    }

    /// Collect every [`XferPlaceholder`] in source order.
    pub fn collect_xfers(&self) -> Vec<XferPlaceholder> {
        let mut out = Vec::new();
        self.collect_xfers_into(&mut out);
        out
    }

    fn collect_xfers_into(&self, out: &mut Vec<XferPlaceholder>) {
        match self {
            ACFGNode::Xfer(x) => out.push(x.clone()),
            ACFGNode::Repeat { body, .. } => body.collect_xfers_into(out),
            ACFGNode::Sequence(children) => {
                for c in children {
                    c.collect_xfers_into(out);
                }
            }
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        }
    }
}

impl ACFG {
    /// Total Xfer count across the whole ACFG.
    pub fn xfer_count(&self) -> usize {
        self.root.count_xfers()
    }

    /// Total Push count across the whole ACFG.
    pub fn push_count(&self) -> usize {
        self.root.count_xfers_role(XferRole::Push)
    }

    /// Total Wait count across the whole ACFG.
    pub fn wait_count(&self) -> usize {
        self.root.count_xfers_role(XferRole::Wait)
    }
}
