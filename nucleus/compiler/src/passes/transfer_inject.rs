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
//! `WorkerId`, not a set. We pick the lexicographically-first worker
//! in the entity's BTreeSet as the canonical representative. This is
//! a known simplification — see "Honest limitations" below.
//!
//! ## Honest limitations (recorded for follow-up)
//!
//! - **Distributed placements treated as a single entity.** The link
//!   pass already collapses `place k on {w0,w1,w2,w3}` into one
//!   `WorkerEntity`. Transfer injection inherits this: a transfer
//!   between `host` and `{w0..w3}` becomes one Push/Wait pair, not
//!   four; the canonical `src`/`dst` we record is the BTreeSet's
//!   first element. A future partition pass (TASK-0016+) will
//!   replicate the pair across the named workers per the partition
//!   policy.
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
//!   w.r.t. the intra-tile iteration. The ACFG layer doesn't carry
//!   per-firing index expressions today (`DataflowEdge::data_in` is
//!   a `Vec<DataId>`, see `acfg.rs`), so a precise check would
//!   require re-plumbing AlgoIR indices through ACFG. Filed as a
//!   follow-up if a future schedule wants opt-out granularity.
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

use crate::acfg::{
    ACFGNode, NotifyMode, Operation, TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
use crate::event::{DataId, IterTile, IterVar, SeqTag, WorkerId};
use crate::link::{LinkedIR, WorkerEntity};
use crate::sched::{ResolvedTransferDirective, ResolvedTransferOption};

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
        let (hoisted, _escaped_at_root) =
            hoist_invariant_waits(new_root, &[], &inner_block_iter_vars);
        splice_pushes_global(hoisted, &inner_block_iter_vars)
    };

    ACFG {
        root: new_root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
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
        ACFGNode::Repeat {
            iter_var, body, ..
        } => block_inner.contains(iter_var) || contains_block_inner(body, block_inner),
        ACFGNode::Sequence(children) => children
            .iter()
            .any(|c| contains_block_inner(c, block_inner)),
    }
}

fn hoist_invariant_waits(
    node: ACFGNode,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    block_inner: &BTreeSet<IterVar>,
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
        // it untouched; nothing escapes.
        node @ ACFGNode::Repeat { .. } if contains_block_inner(&node, block_inner) => {
            (node, Vec::new())
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
        } => {
            let mut nested: Vec<(IterVar, std::ops::Range<i64>)> = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            let (body2, escaped) = hoist_invariant_waits(*body, &nested, block_inner);

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
                        let (c2, esc) =
                            hoist_invariant_waits(other, enclosing_tile, block_inner);
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
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_producer(*body, push)),
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
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_repeat(*body, cut_iv, push)),
        },
        leaf => leaf,
    }
}

fn collect_push_seqs(
    node: &ACFGNode,
    seqs: &mut BTreeSet<u64>,
) {
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
    out: &mut Vec<XferPlaceholder>,
) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Wait => out.push(x.clone()),
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => {
            if contains_block_inner(node, block_inner) {
                // Opaque block nest — TASK-0149 owns its Pushes.
                return;
            }
            collect_waits(body, block_inner, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_waits(c, block_inner, out);
            }
        }
    }
}

fn splice_pushes_global(mut root: ACFGNode, block_inner: &BTreeSet<IterVar>) -> ACFGNode {
    let mut have_seqs: BTreeSet<u64> = BTreeSet::new();
    collect_push_seqs(&root, &mut have_seqs);

    let mut waits: Vec<XferPlaceholder> = Vec::new();
    collect_waits(&root, block_inner, &mut waits);

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
            // No producer Operation for this data (e.g. a synthetic
            // partial ACFG in a unit test). Nothing to pair; leave the
            // Wait — downstream analysis will report the gap honestly.
            None => continue,
        };
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
            let src = canonical_worker(producer_workers);
            let dst = canonical_worker(&consumer_workers);
            // Skip if canonicalisation collapsed src == dst (this
            // would happen only if both sets were empty, which the
            // earlier `if` already eliminates; defensive).
            if src == dst {
                continue;
            }
            let policy = ctx
                .policies_by_data
                .get(&data_id)
                .copied()
                .unwrap_or_default();
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

    out
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Pick a canonical `WorkerId` from a worker set. We take the
/// lexicographically-first (smallest by `Ord`) member. BTreeSet
/// iteration is sorted, so `.iter().next()` is the canonical choice.
///
/// Panics on an empty set, which would be a builder-pass invariant
/// violation (every Operation has at least one worker, see
/// `acfg::resolve_worker_set`).
fn canonical_worker(set: &BTreeSet<WorkerId>) -> WorkerId {
    *set.iter()
        .next()
        .expect("worker entity must be non-empty (acfg invariant)")
}

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
