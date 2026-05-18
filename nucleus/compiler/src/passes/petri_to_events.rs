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
//! - `ACFGNode::Repeat { range, body }` — unroll: visit `body` once
//!   per iteration in `range`. Matches `acfg_to_petri` (PRD §8 says
//!   "statically determined firing order"; we unroll rather than
//!   emit one event with a multi-iteration tile).
//! - `ACFGNode::Operation(op)` — emit `Event::Fire { kernel, tile:
//!   empty, bindings }` on every worker in `op.workers`. The tile is
//!   empty because the unrolled Repeat does *not* preserve a
//!   per-iteration coordinate at M2 (`acfg_to_petri` discards the
//!   iter-coord too; see its module docs for the same trade). Filed
//!   as a follow-up. `bindings` (TASK-0156) is the per-firing value
//!   binding projected from the Operation's single `DataflowEdge`:
//!   the positional `args` (per kernel parameter) and the output
//!   `(DataId, slice)`. This is what lets a backend compute the
//!   firing's *value* from the EventList alone (unblocks TASK-0124);
//!   before TASK-0156 a `Fire` carried only `kernel` + `tile`.
//! - `ACFGNode::Sync(s)` — emit `Event::Sync { participants:
//!   s.participants.clone(), kind: SyncKind::Barrier }` on every
//!   participating worker.
//! - `ACFGNode::Xfer { role: Push, src, dst, data, tile, seq, .. }`
//!   — emit `Event::Push { dst, data, tile, seq }` on `src`.
//! - `ACFGNode::Xfer { role: Wait, src, dst, data, tile, seq, .. }`
//!   — emit `Event::Wait { src, data, tile, seq }` on `dst`.
//!
//! Two runs of `acfg_to_events` over the same ACFG produce
//! byte-identical `EventList`s: the walker is deterministic, the
//! `BTreeMap<WorkerId, Vec<Event>>` keys iterate in `WorkerId`
//! numeric order, and the ACFG's `name_workers` is a `BTreeMap` so
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
//! - **Iteration tile per-iteration is empty.** A `Repeat` with
//!   range `0..N` emits N copies of each enclosed `Fire`, all with
//!   `tile: IterTile::empty()`. The lowering pass `acfg_to_petri`
//!   makes the same trade — it unrolls without retaining the
//!   per-firing iter-coord. Once tile-carrying is added there (it's
//!   the bigger change), this pass will read it through. Filed as
//!   a follow-up.
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

use crate::acfg::{ACFGNode, Operation, SyncPlaceholder, XferPlaceholder, XferRole, ACFG};
use crate::event::{Event, FireBinding, IterTile, SyncKind, WorkerId};
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

    walk(&acfg.root, &mut out);
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

fn walk(node: &ACFGNode, out: &mut BTreeMap<WorkerId, Vec<Event>>) {
    match node {
        ACFGNode::Operation(op) => emit_operation(op, out),
        ACFGNode::Sync(s) => emit_sync(s, out),
        ACFGNode::Xfer(x) => emit_xfer(x, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                walk(c, out);
            }
        }
        ACFGNode::Repeat { range, body, .. } => {
            // Unroll. Match `acfg_to_petri`'s saturating arithmetic so
            // malformed empty / inverted ranges produce zero firings
            // rather than panicking. PRD §8.6's static-bounded-loops
            // restriction means real schedules always have a sensible
            // range.
            let count = range.end.saturating_sub(range.start).max(0) as u64;
            for _ in 0..count {
                walk(body, out);
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
