//! Petri net (per-worker projection) -> per-worker `EventList`
//! (TASK-0027, PRD §8.1 and §8.3).
//!
//! ## What this pass does
//!
//! The scheduler in PRD §8.1 is typed as
//!
//! ```text
//! schedule : (AlgoIR, SchedIR) -> ( GlobalNet, { WorkerId -> EventList } )
//! ```
//!
//! [`crate::passes::acfg_to_petri::acfg_to_net`] already produces the
//! `GlobalNet`. This module produces the per-worker `EventList`s: for
//! each declared worker, the totally-ordered sequence of
//! [`crate::event::Event`]s that worker executes.
//!
//! ## Why we take the ACFG (and not just the `Net`) as input
//!
//! The task brief sketched the signature as `(net: &Net) ->
//! BTreeMap<WorkerId, Vec<Event>>`. In practice the [`crate::petri::Net`]
//! produced by `acfg_to_net` does not retain enough source-level
//! metadata to rematerialise the event payloads:
//!
//! - For an `Operation` lowering the transition `name` carries the
//!   kernel id as a string (`op_k{kid}`) but not the iteration tile,
//!   nor which workers participated in a distributed placement.
//! - For a `Push` / `Wait` lowering the transition `name` carries
//!   `seq` but neither `src`, `dst`, `data`, nor the iteration tile;
//!   `Transition::worker` only names the *owner* of each endpoint.
//! - For a `Sync` lowering the transition `name` is the literal
//!   string `"sync_barrier"`; the participant set is recoverable
//!   from arcs, but parsing arc structure to recover participants is
//!   strictly less direct than reading the ACFG node.
//!
//! Either of these is solvable: enrich the `Transition` payload, or
//! sidecar a "what produced this transition" map next to the `Net`.
//! Both are bigger changes than this M2 task warrants. The ACFG is
//! the source of truth that `acfg_to_net` projects from, and the
//! linearisation produced by walking the ACFG in source order is
//! *exactly* the per-worker control-place chain that
//! `acfg_to_net` builds (see its module docs). So walking the ACFG
//! tree directly produces the same per-worker firing order as
//! "project transitions of the Net by `worker` in their insertion
//! order would have".
//!
//! Pragmatically, the entry point is
//! [`acfg_to_events`] (the meat). A thin wrapper
//! [`petri_to_events`] takes a `(&ACFG, &Net)` pair to honour the
//! task's stated signature — the `&Net` is currently unused, but
//! threading it now means downstream consumers don't have to change
//! their call sites when later milestones make the `Net` load-bearing
//! (e.g. boundedness / liveness facts feeding back into event
//! synthesis). Filed as a follow-up in the task self-report.
//!
//! ## Linearisation
//!
//! Per PRD §8.6 the firing-order linearisation is "deterministic
//! greedy (source order + dataflow constraints)". The ACFG walk *is*
//! source order; the dataflow constraints are already discharged by
//! `acfg_to_petri`'s per-worker control-place chain (each worker's
//! k-th transition consumes the token its (k-1)-th produced).
//!
//! Concretely the walker visits the ACFG depth-first in source order:
//!
//! - `ACFGNode::Sequence(children)` — visit each child left-to-right.
//! - `ACFGNode::Repeat { iter_var, range, body }` —
//!   **structure-preserving** (TASK-0159): project `body` once into a
//!   scratch per-worker map and wrap each worker's slice in a single
//!   [`crate::event::Event::Loop`] carrying `iter_var` + `range`. We
//!   do NOT unroll the EventList. (The analysis Net produced by
//!   `acfg_to_petri` *does* still unroll — see the contrast note
//!   below.) A backend consuming only the EventList must be able to
//!   re-emit the rolled `for` loop verbatim; flattening to N copies
//!   destroys the loop variable, the bound and the for-structure.
//! - `ACFGNode::Operation(op)` — emit `Event::Fire { kernel, tile:
//!   empty, bindings }` on every worker in `op.workers`. The per-Fire
//!   tile is empty because the enclosing `Event::Loop` already names
//!   the iteration coordinate (`iter_var` + `range`); duplicating it
//!   on every `Fire` would be redundant. (Contrast: the analysis Net
//!   in `acfg_to_petri` unrolls and discards the iter-coord entirely
//!   — that path is unchanged; see its module docs and the
//!   Net-vs-EventList note below.) Filed as a follow-up if a backend
//!   ever needs the coordinate on the `Fire` itself. `bindings`
//!   (TASK-0156) is the per-firing value
//!   binding projected from the Operation's single `DataflowEdge`:
//!   the positional `args` (per kernel parameter) and the output
//!   `(DataId, slice)`. This is what lets a backend compute the
//!   firing's *value* from the EventList alone (unblocks TASK-0124);
//!   before TASK-0156 a `Fire` carried only `kernel` + `tile`.
//! - `ACFGNode::Sync(s)` — emit `Event::Sync { participants:
//!   s.participants.clone(), kind: SyncKind::Barrier, sync: s.sync }`
//!   on every participating worker. `s.sync` (the stable
//!   cross-worker barrier identity, TASK-0172) is copied verbatim
//!   into each participant's list — the `Sync` analogue of how
//!   `seq` is copied onto both endpoints of a Push/Wait pair — so
//!   disjoint per-worker `EventList`s agree on barrier identity with
//!   no global ACFG walk.
//! - `ACFGNode::Xfer { role: Push, src, dst, data, tile, seq, .. }`
//!   — emit `Event::Push { dst, data, tile, seq }` on `src`.
//! - `ACFGNode::Xfer { role: Wait, src, dst, data, tile, seq, .. }`
//!   — emit `Event::Wait { src, data, tile, seq }` on `dst`.
//!
//! Two runs of `acfg_to_events` over the same ACFG produce
//! byte-identical `EventList`s: the walker is deterministic, the
//! `BTreeMap<WorkerId, Vec<Event>>` keys iterate in `WorkerId`
//! numeric order (including the per-`Repeat` scratch map whose
//! `Event::Loop`s are appended in `WorkerId` order), and the ACFG's
//! `name_workers` is a `BTreeMap` so
//! the seeded worker set is itself sorted.
//!
//! ## What this pass deliberately does NOT do at M2
//!
//! - **`Event::Alloc` / `Event::Free` are NOT emitted.** PRD §8.3
//!   maps Region resolution to `place_data D in MEMORY_REGION`
//!   schedule directives. v2 M2 does not yet thread `Region`
//!   assignments through the IR; the pthreads-sync backend lowers
//!   each data symbol to a stack/heap allocation owned by the
//!   generated worker function and has no need for explicit Alloc /
//!   Free events. Synthesising "first use = Alloc, last use = Free"
//!   here would be guessing at semantics no consumer relies on.
//!   Filed as a follow-up.
//!
//! - **Iteration tile per-iteration is empty.** Each enclosed `Fire`
//!   still carries `tile: IterTile::empty()`. Structure preservation
//!   (TASK-0159) keeps the loop *nest* (`Event::Loop` carries
//!   `iter_var` + `range`), so a backend re-derives the per-iteration
//!   coordinate from the loop it is replaying; the per-`Fire` tile
//!   stays empty rather than duplicating the coordinate the enclosing
//!   `Loop` already names. (Contrast: the analysis Net in
//!   `acfg_to_petri` still unrolls and likewise discards the
//!   iter-coord — that path is unchanged because boundedness /
//!   deadlock consume the unrolled firing order.) Filed as a
//!   follow-up if a backend ever needs the coordinate on the `Fire`
//!   itself.
//!
//! ### Net (analysis) unrolls vs EventList (codegen) preserves
//!
//! `acfg_to_petri::acfg_to_net` (boundedness / deadlock /
//! determinism analyses) and this projection are SEPARATE walks of
//! the same ACFG. The analyses consume the *unrolled* Net firing
//! order, so `acfg_to_petri` deliberately still unrolls `Repeat` and
//! is **not** touched by TASK-0159. This projection is the codegen
//! contract and is structure-preserving. The two are decoupled by
//! design; do not "unify" them by unrolling here or by rolling
//! there.
//!
//! - **Distributed placement (`place k on { w0, w1, w2 }`).** The
//!   `acfg_to_petri` pass currently treats the worker set as one
//!   entity sharing a single transition; we mirror that here by
//!   emitting one `Fire` *per worker* in the set, all with the same
//!   (empty) tile. The eventual partition pass will replace this
//!   with per-tile partitioned fires; until then, the projection is
//!   "every participant fires the kernel" which matches the net's
//!   semantics.
//!
//! - **No `Event::Sync` elision for single-participant syncs.** The
//!   upstream sync-injection pass already elides sub-2-participant
//!   syncs; we honour whatever survives. If a one-participant sync
//!   somehow slipped through, we still emit the `Sync` event on
//!   that lone worker — the backend can no-op it.
//!
//! - **Push/Wait imbalance inherited from `transfer_inject`.** The
//!   upstream transfer-injection pass currently splices Pushes
//!   *within* one sequence only (see its `splice_pushes_for_waits`).
//!   When a producer lives at top level and the consumer lives
//!   inside a `for` (e.g. example 02-split-add: `load_input` on host,
//!   then `for i { add(a[i], b[i]) }` on `w0`), the ACFG ends up
//!   with Wait nodes on the consumer's side but no matching Push
//!   nodes on the producer's. This pass faithfully projects whatever
//!   it receives; the gap therefore surfaces in the EventList as
//!   "unmatched Waits". The pthreads-sync backend currently
//!   compensates by consuming the ACFG directly with shared-memory
//!   shortcuts; the fix for backends that consume EventLists is
//!   cross-scope splicing in `transfer_inject`, not in this pass.
//!   Recorded as a separate follow-up task.
//!
//! ## Output determinism
//!
//! The returned [`BTreeMap`] is keyed by [`WorkerId`]; iteration is
//! sorted by numeric id. Every worker named in `acfg.name_workers`
//! gets an entry even if its event list is empty (this matches the
//! contract a backend wants: "tell me what worker w17 does", and
//! the answer "nothing" is still an answer).
//!
//! Round-trip identity: `acfg_to_events(acfg)` and a fresh
//! `acfg_to_events(acfg)` produce structurally identical maps.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::acfg::{ACFGNode, Operation, SyncPlaceholder, XferPlaceholder, XferRole, ACFG};
use crate::event::{Event, FireBinding, IterTile, IterVar, SyncKind, WorkerId};
use crate::petri::Net;

// --------------------------------------------------------------------
// Public entry points
// --------------------------------------------------------------------

/// Project an ACFG into per-worker event lists.
///
/// The returned map contains one entry per worker declared in
/// `acfg.name_workers`, even if that worker emits zero events.
///
/// Determinism: identical input ACFGs produce byte-identical maps.
pub fn acfg_to_events(acfg: &ACFG) -> BTreeMap<WorkerId, Vec<Event>> {
    let mut out: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // Seed every declared worker with an empty list so backends can
    // look up any declared worker without having to special-case
    // "this worker contributed nothing".
    for wid in acfg.name_workers.values() {
        out.entry(*wid).or_default();
    }

    walk(&acfg.root, &mut out, &acfg.partition_worker_ranges);
    out
}

/// Project the Petri net (alongside the source ACFG) into per-worker
/// event lists. The `_net` argument is presently unused — see the
/// module docs for why we accept it. Callers that already have a
/// `Net` lying around can route through this entry point so that
/// later milestones (where the `Net` becomes load-bearing) don't
/// require a call-site change.
pub fn petri_to_events(acfg: &ACFG, _net: &Net) -> BTreeMap<WorkerId, Vec<Event>> {
    acfg_to_events(acfg)
}

// --------------------------------------------------------------------
// Walker
// --------------------------------------------------------------------

fn walk(
    node: &ACFGNode,
    out: &mut BTreeMap<WorkerId, Vec<Event>>,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>,
) {
    match node {
        ACFGNode::Operation(op) => emit_operation(op, out),
        ACFGNode::Sync(s) => emit_sync(s, out),
        ACFGNode::Xfer(x) => emit_xfer(x, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                walk(c, out, partition_ranges);
            }
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // STRUCTURE-PRESERVING (TASK-0159). The analysis Net
            // (`acfg_to_petri`) still unrolls — boundedness / deadlock
            // consume the unrolled order and are deliberately left
            // alone. The EventList is the *codegen* contract: a backend
            // consuming only the EventList (TASK-0124) must be able to
            // re-emit the rolled `for` loop verbatim, so we project the
            // `Repeat` to one `Event::Loop` per worker that carries the
            // loop nest instead of flattening it.
            //
            // We project the body ONCE into a scratch per-worker map
            // (one iteration), then wrap each worker's produced slice
            // in a single `Event::Loop` and append it to that worker's
            // real list. A nested `Repeat` recurses naturally, so a
            // nested loop projects to a nested `Loop`.
            //
            // The `range` is carried verbatim — including degenerate
            // empty / inverted ranges. We do NOT do the old
            // `saturating_sub` firing-count math: the backend replays
            // the body `range.len()` times, and an empty/inverted range
            // simply yields zero replays. Carrying the raw range keeps
            // the contract faithful to the source `for` bounds (a
            // backend re-emits `for v in lo..hi` exactly).
            //
            // A worker that contributes NOTHING to the body gets no
            // `Loop` at all (not an empty-bodied one): the old unroll
            // likewise added nothing for such a worker, and a backend
            // wants "this worker does nothing in this scope", not "this
            // worker spins an empty loop".
            let mut scratch: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
            walk(body, &mut scratch, partition_ranges);
            // Deterministic: `scratch` is a BTreeMap, iterated in
            // WorkerId order.
            //
            // Per-worker range override (TASK-0212). If the schedule
            // attached `partition=workers` to this loop AND the
            // partition pass recorded a per-worker range for this
            // worker, use that worker's exclusive slice as the
            // emitted `Event::Loop.range`; otherwise fall back to the
            // source range (the pre-TASK-0212 behaviour). The override
            // map is keyed by the loop's `iter_var` so two loops with
            // the same body workers but different iter vars stay
            // independent. A worker NOT listed in the per-iter-var
            // map (e.g. host, which doesn't participate in
            // partition=workers) falls back to the source range —
            // that worker would normally contribute no body events
            // anyway, but emitting it with the source range matches
            // the pre-TASK-0212 contract for any non-participating
            // worker that happens to project a body event.
            let per_worker_override = partition_ranges.get(iter_var);
            for (wid, body_events) in scratch {
                if body_events.is_empty() {
                    continue;
                }
                let projected_range = match per_worker_override
                    .and_then(|m| m.get(&wid))
                {
                    Some(r) => r.clone(),
                    None => range.clone(),
                };
                // Thread the per-occurrence strip-mine rebinding tag
                // (TASK-0180) verbatim onto the projected loop. It is
                // `None` for source / tile loops and `Some` only for a
                // `block_transform`-produced inner loop; the backend
                // rebinds the loop variable from this tag ALONE (no
                // global EventList occurrence count).
                out.entry(wid).or_default().push(Event::Loop {
                    iter_var: *iter_var,
                    range: projected_range,
                    body: body_events,
                    block_tag: *block_tag,
                });
            }
        }
    }
}

fn emit_operation(op: &Operation, out: &mut BTreeMap<WorkerId, Vec<Event>>) {
    // Per-firing value binding (TASK-0156). At M1/M2 a firing has
    // exactly one `DataflowEdge` (see `acfg::DataflowDag` docs); we
    // project its positional `args` and output access onto a
    // `FireBinding` so the EventList carries enough to reconstruct
    // the kernel call WITHOUT walking the AlgoIR.
    //
    // An edge-less Operation must NOT silently yield an empty binding
    // (review Q4.2): the whole point of TASK-0156 is that the
    // EventList carries the value payload — an empty one would let a
    // backend (TASK-0124) mis-codegen or fail far from the cause.
    // `build_acfg` always emits exactly one edge per Operation, so a
    // missing edge is a malformed ACFG / compiler bug; fail loud at
    // the seam with context.
    let edge = op.dataflow.edges.first().unwrap_or_else(|| {
        panic!(
            "petri_to_events: Operation for kernel {:?} has no DataflowEdge; \
             build_acfg emits exactly one per Operation — malformed ACFG, \
             not a tolerable empty binding (TASK-0156)",
            op.kernel
        )
    });
    let bindings = FireBinding {
        inputs: edge.args.clone(),
        output: edge.data_out_access.clone(),
    };

    // Distributed placement: emit one Fire per participating worker.
    // See module docs ("Distributed placement") for the M2 trade.
    // The binding is cloned per worker so each EventList stays
    // self-contained (a backend reads one event, no sidecar join).
    let event = Event::Fire {
        kernel: op.kernel,
        tile: IterTile::empty(),
        bindings,
    };
    for wid in &op.workers {
        out.entry(*wid).or_default().push(event.clone());
    }
}

fn emit_sync(s: &SyncPlaceholder, out: &mut BTreeMap<WorkerId, Vec<Event>>) {
    // Every participant records the barrier in its own EventList.
    // The `participants` set is cloned per-worker so each EventList
    // is self-contained (backends can read it without borrowing the
    // ACFG).
    for wid in &s.participants {
        let ev = Event::Sync {
            participants: s.participants.clone(),
            kind: SyncKind::Barrier,
            // Stable cross-worker barrier identity (TASK-0172):
            // copied verbatim into EVERY participant's list, so all
            // participants of this barrier carry the same `SyncTag`
            // (the cross-worker join key). Mirrors `seq: x.seq` on
            // Push/Wait in `emit_xfer`.
            sync: s.sync,
        };
        out.entry(*wid).or_default().push(ev);
    }
}

fn emit_xfer(x: &XferPlaceholder, out: &mut BTreeMap<WorkerId, Vec<Event>>) {
    match x.role {
        XferRole::Push => {
            let ev = Event::Push {
                dst: x.dst,
                data: x.data,
                tile: x.tile.clone(),
                seq: x.seq,
            };
            out.entry(x.src).or_default().push(ev);
        }
        XferRole::Wait => {
            let ev = Event::Wait {
                src: x.src,
                data: x.data,
                tile: x.tile.clone(),
                seq: x.seq,
            };
            out.entry(x.dst).or_default().push(ev);
        }
    }
}
