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
//! - **Block-entangled non-block transfers are stranded (TASK-0151
//!   over-approximation).** The cross-scope finaliser (Pass A / Pass
//!   B) skips a Repeat subtree as soon as it *contains* a
//!   `block`-inner loop (`contains_block_inner`), not just the
//!   block-inner loop itself. So a genuinely loop-invariant,
//!   non-block cross-worker Wait that lives **inside or under** a
//!   Repeat that also encloses a block nest is NOT finalised: it
//!   keeps no whole-symbol Push and will deadlock unless TASK-0149
//!   (per-tile cross-scope Push) covers it. Only transfers that are
//!   *structurally disjoint* from every block nest (e.g. a sibling
//!   plain `for` loop — see
//!   `mixed_block_and_nonblock_program_pairs_the_nonblock_transfer`)
//!   are paired here. This is a deliberate conservative choice: it
//!   never *collapses* a per-tile halo transfer (no 05/07-blocked
//!   regression), but it *defers* an entangled non-block transfer.
//!   The deferral is **traceable, not invisible**: each skipped
//!   symbol/seq is reported via the `NUC_TRACE`-gated `nuc_trace!`
//!   facility (TASK-0154, closes TASK-0151 AC#2) at both skip sites
//!   (Pass A's opaque-Repeat arm and Pass B's `collect_waits`
//!   exclusion). It is silent on the default path — setting
//!   `NUC_TRACE=1` surfaces every deferred `(symbol, seq)`. No
//!   example schedule hits the entangled shape today (single-`block=`
//!   programs put the block nest and any plain loop as siblings). The
//!   precise per-Wait classification needs TASK-0150 (index-based
//!   invariance); the deferral is owned by TASK-0149. Pinned by
//!   `block_nested_in_plain_loop_strands_the_invariant_wait`;
//!   trace coverage by `block_deferral_is_traceable_under_nuc_trace`.
//!
//! - **Idempotence by structural skip.** Re-running the pass detects
//!   that a Wait already precedes the consumer Operation (and a Push
//!   already follows the producer Operation) by checking sibling
//!   `Xfer` nodes carrying the same `(src, dst, data, tile)`. It
//!   does NOT re-derive `seq` to be the same — the original
//!   placeholder is left in place. Tests cover this.
//!
//! - **No conflict detection between `sync` and `async` options on
//!   one directive.** If the schedule wrote `transfer D : sync, async;`,
//!   the last option in source order wins for the `synchronous` flag.
//!   The schedule lowering pass already flags this as a follow-up
//!   linker concern (grammar §2 note 7); we mirror that choice here.
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
use crate::event::{DataId, IterTile, IterVar, SeqTag, WorkerId};
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
    // (LinkError::UnknownLoop), so we silently skip them here —
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
    // Scope: PER-SUBTREE (TASK-0151). `block=N` per-tile transfers are
    // sliced sub-regions, not loop-invariant whole symbols; the
    // existing TASK-0143 HoistSink owns that path and the matching
    // per-tile Push is TASK-0149/TASK-0150 follow-up territory
    // (precise index-based invariance). Running Pass A/B inside a
    // block-governed Repeat nest would wrongly collapse per-tile halo
    // transfers into one. So Pass A treats any Repeat whose subtree
    // contains a `block`-inner loop as OPAQUE (no hoist across it),
    // and Pass B excludes Waits living inside such a nest — while
    // still finalising every non-blocked cross-scope transfer
    // elsewhere in the same program. This is strictly more precise
    // than the previous whole-program gate (which did nothing if any
    // block loop existed anywhere): a program mixing a block=N loop
    // with an unrelated non-blocked cross-worker `for` no longer
    // silently re-deadlocks on the non-blocked part. When
    // `inner_block_iter_vars` is empty, `contains_block_inner` is
    // always false, so behaviour is identical to the non-blocked
    // M2-acceptance path (example 02-split).
    let new_root = {
        let (hoisted, escaped_at_root) =
            hoist_invariant_waits(new_root, &[], &inner_block_iter_vars, &name_data);
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
        let spliced = splice_pushes_global(hoisted, &inner_block_iter_vars, &name_data);
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
        rewrite_partition_tiles(spliced, &partition_worker_ranges)
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
    let mut pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64> =
        pre_existing_pipeline_depth;
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
    }
}

/// Walk the final ACFG (post-hoist, post-splice, post-rewrite-tiles)
/// and populate `out` with (SeqTag, depth) for every `Xfer`
/// placeholder whose enclosing iteration tile contains an iter-var
/// with a `pipeline=D` directive. Innermost wins.
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
/// tile-scope as the Wait), the Wait wins (we visit it last by
/// convention, but in practice they agree).
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

/// True if `node`'s subtree contains a `block`-inner Repeat (its own
/// iter-var, or any descendant Repeat's, is in `block_inner`). Such a
/// Repeat nest is owned by the TASK-0143 HoistSink / TASK-0149 per-tile
/// path; the whole-symbol passes treat it as opaque (TASK-0151).
fn contains_block_inner(node: &ACFGNode, block_inner: &BTreeSet<IterVar>) -> bool {
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => false,
        ACFGNode::Repeat { iter_var, body, .. } => {
            block_inner.contains(iter_var) || contains_block_inner(body, block_inner)
        }
        ACFGNode::Sequence(children) => children
            .iter()
            .any(|c| contains_block_inner(c, block_inner)),
    }
}

/// Resolve a `DataId` back to its source symbol name for diagnostics.
/// Mirrors the existing reverse-lookup idiom used by the TASK-0152
/// escaped-Wait panic. `<unknown>` only if the id is absent from the
/// name table (a malformed ACFG, not normal input).
fn data_symbol(name_data: &BTreeMap<String, DataId>, id: DataId) -> &str {
    name_data
        .iter()
        .find_map(|(n, d)| (*d == id).then_some(n.as_str()))
        .unwrap_or("<unknown>")
}

/// Collect every Xfer placeholder (Push or Wait) inside an *opaque*
/// block-governed subtree, so the cross-scope finaliser can name what
/// it is deferring instead of skipping it invisibly (TASK-0154 /
/// TASK-0151 AC#2). This walks the whole subtree (including nested
/// Repeats) because the deferral applies to the entire opaque nest.
fn collect_deferred_xfers(node: &ACFGNode, out: &mut Vec<XferPlaceholder>) {
    match node {
        ACFGNode::Xfer(x) => out.push(x.clone()),
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => collect_deferred_xfers(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_deferred_xfers(c, out);
            }
        }
    }
}

/// Emit one `NUC_TRACE` line per Xfer placeholder left for the
/// TASK-0149/0150 per-tile path by an opaque block-governed subtree.
/// Silent unless `NUC_TRACE` is set (zero output on the default path —
/// determinism/e2e safe). Worded as a deliberate deferral, not an
/// error: the per-tile Push/Wait is owned by TASK-0149's per-tile
/// finalisation, not a bug in this pass.
fn trace_block_deferral(
    pass: &str,
    subtree: &ACFGNode,
    name_data: &BTreeMap<String, DataId>,
) {
    // Cheap structural guard mirrors the macro guard: skip the walk
    // entirely on the default (silent) path.
    if !(crate::trace::trace_enabled() || crate::trace::test_sink_active()) {
        return;
    }
    let mut xfers = Vec::new();
    collect_deferred_xfers(subtree, &mut xfers);
    for x in xfers {
        crate::nuc_trace!(
            "transfer_inject({pass}): cross-scope finalisation deferred for \
             block-governed symbol `{}` (data {:?}, seq {:?}, {:?}, {:?} -> \
             {:?}) — per-tile Push/Wait is owned by TASK-0149/0150, not \
             skipped in error",
            data_symbol(name_data, x.data),
            x.data,
            x.seq,
            x.role,
            x.src,
            x.dst
        );
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
    block_inner: &BTreeSet<IterVar>,
    // Threaded purely for the deferral trace (TASK-0154): the skip
    // decision is the only place that knows *what* is being deferred,
    // so the trace is emitted at the decision site (fail/trace where
    // the choice is made), not reconstructed elsewhere.
    name_data: &BTreeMap<String, DataId>,
) -> (ACFGNode, Vec<XferPlaceholder>) {
    match node {
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => (leaf, Vec::new()),
        // A bare Wait/Xfer outside a Sequence shouldn't occur from the
        // builder, but if it does we cannot decide invariance without
        // its sibling context — leave it in place.
        leaf @ ACFGNode::Xfer(_) => (leaf, Vec::new()),
        // Block-governed Repeat nest: opaque to whole-symbol hoisting
        // (TASK-0151). The TASK-0143 HoistSink already positioned its
        // per-tile Waits during `inject_in_node`; lifting them further
        // would collapse per-tile sub-region transfers into one. Leave
        // it untouched; nothing escapes. The deferral is *traceable*
        // via NUC_TRACE (TASK-0154, closes TASK-0151 AC#2) — silent on
        // the default path, so determinism/e2e output is unchanged.
        node @ ACFGNode::Repeat { .. } if contains_block_inner(&node, block_inner) => {
            trace_block_deferral("hoist/PassA", &node, name_data);
            (node, Vec::new())
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            let mut nested: Vec<(IterVar, std::ops::Range<i64>)> = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            let (body2, escaped) = hoist_invariant_waits(*body, &nested, block_inner, name_data);

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
                        let (c2, esc) = hoist_invariant_waits(other, enclosing_tile, block_inner, name_data);
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

/// Collect Waits eligible for whole-symbol Push finalisation. Waits
/// inside a block-governed Repeat nest are excluded (TASK-0151): their
/// per-tile Push is TASK-0149's job, not this pass's.
fn collect_waits(
    node: &ACFGNode,
    block_inner: &BTreeSet<IterVar>,
    name_data: &BTreeMap<String, DataId>,
    out: &mut Vec<XferPlaceholder>,
) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Wait => out.push(x.clone()),
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => {
            if contains_block_inner(node, block_inner) {
                // Opaque block nest — TASK-0149 owns its Pushes. The
                // exclusion is traceable via NUC_TRACE (TASK-0154,
                // closes TASK-0151 AC#2): silent by default so the
                // determinism/e2e snapshot is byte-unchanged.
                trace_block_deferral("collect_waits/PassB", node, name_data);
                return;
            }
            collect_waits(body, block_inner, name_data, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_waits(c, block_inner, name_data, out);
            }
        }
    }
}

fn splice_pushes_global(
    mut root: ACFGNode,
    block_inner: &BTreeSet<IterVar>,
    name_data: &BTreeMap<String, DataId>,
) -> ACFGNode {
    let mut have_seqs: BTreeSet<u64> = BTreeSet::new();
    collect_push_seqs(&root, &mut have_seqs);

    let mut waits: Vec<XferPlaceholder> = Vec::new();
    collect_waits(&root, block_inner, name_data, &mut waits);

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
/// The new tile is `[(iv, partition_ranges[iv][compute_worker])]`
/// for each iv where compute_worker has an entry; BTreeMap iteration
/// keeps the axis order deterministic.
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
    rewrite_partition_tiles_inner(node, partition_ranges)
}

fn rewrite_partition_tiles_inner(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Pick the compute worker per the rule above.
            let compute_worker = if partition_ranges
                .values()
                .any(|m| m.contains_key(&x.src))
            {
                Some(x.src)
            } else if partition_ranges
                .values()
                .any(|m| m.contains_key(&x.dst))
            {
                Some(x.dst)
            } else {
                None
            };
            if let Some(w) = compute_worker {
                let mut bounds: Vec<(IterVar, std::ops::Range<i64>)> = Vec::new();
                for (iv, per_worker) in partition_ranges {
                    if let Some(range) = per_worker.get(&w) {
                        bounds.push((*iv, range.clone()));
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
                .map(|c| rewrite_partition_tiles_inner(c, partition_ranges))
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
            body: Box::new(rewrite_partition_tiles_inner(*body, partition_ranges)),
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
