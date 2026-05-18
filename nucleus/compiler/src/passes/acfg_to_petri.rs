//! ACFG -> global Petri net (TASK-0026).
//!
//! Lowers an injected ACFG into a [`crate::petri::Net`] per PRD §8.2.
//!
//! ## What this pass does (and why)
//!
//! The ACFG, after `sync_inject` and `transfer_inject` have run, is a
//! tree of [`ACFGNode`]s describing per-worker firings, barriers, and
//! matched Push/Wait pairs. This pass walks the tree and emits a
//! place/transition net that mirrors PRD §8.2's mapping table:
//!
//! | ACFG concept                                | Petri net                                       |
//! | ------------------------------------------- | ----------------------------------------------- |
//! | `Operation` on worker(s) W                  | one `Transition` consuming/producing W's control places |
//! | `Repeat { range }` (static bounds)          | body unrolled `range.len()` times              |
//! | `Sync(participants)`                        | one `Transition` spanning each participant's control |
//! | `Xfer{role=Push, src, dst, data, seq, ...}` | `Transition` on `src`, plus deposit into buffer place keyed by `seq` |
//! | `Xfer{role=Wait, src, dst, data, seq, ...}` | `Transition` on `dst`, plus consume from buffer place keyed by `seq` |
//! | `TransferPolicy { buffer }`                 | capacity of the buffer place                    |
//! | Sequential per-worker order                 | "control" places threading each worker's transitions in order |
//!
//! ### Why per-worker control places
//!
//! PRD §8.4: "Statically determined firing order. Order is decided at
//! compile time, not by token availability at run time." The control
//! places enforce per-worker linear order in the *firing-simulation*
//! semantics of the net: a worker's k-th transition can only fire
//! after its (k-1)-th, because it consumes the token the (k-1)-th
//! produced. Cross-worker firings (`Sync`, `Push`+`Wait` pairs) then
//! constrain the relative ordering between workers — exactly the DAG
//! the deadlock/boundedness analyses (TASK-0028/0029) will walk.
//!
//! Each worker has an initial control place with `initial_marking=1`
//! so the first transition is enabled at start.
//!
//! ### Why unroll `Repeat`
//!
//! The task brief and PRD §8.4 both note that static firing order is
//! required. The two viable encodings are:
//!
//! 1. **Unroll** — emit N body-copies of transitions for a range of
//!    length N. Simple. Linear in N in both the net and downstream
//!    analyses. Explodes for large N.
//! 2. **Loop with counter** — one set of body transitions, plus a
//!    "loop counter" place pre-marked with N tokens. Compact. Hides
//!    the per-iteration tile in the firing trace; downstream analyses
//!    that want to reason per-iteration become awkward.
//!
//! v2 M2 picks (1) **for the analysis Net**. It is the smaller code
//! change, lines up cleanly with the per-worker control-place chain,
//! and the boundedness / deadlock / determinism analyses
//! (TASK-0028/0029) consume the unrolled firing order directly.
//!
//! NOTE (TASK-0159): this is the **analysis** path and it still
//! unrolls. The separate **EventList codegen** projection
//! (`petri_to_events`) NO LONGER unrolls — it preserves the loop
//! nest as `Event::Loop` so a backend that consumes only the
//! EventList can re-emit the rolled `for` verbatim. The two walks of
//! the ACFG are intentionally decoupled: the Net stays unrolled
//! (analyses depend on it), the EventList stays rolled (codegen
//! contract). Do not unify them. The earlier claim here that
//! unrolling "matches what the EventList projection needs" was the
//! pre-TASK-0159 trade and is no longer true.
//!
//! Filed as a follow-up to reconsider the Net encoding once
//! boundedness/deadlock analyses are in and we have a feel for how N
//! scales in real examples.
//!
//! ### Buffer place capacity
//!
//! [`TransferPolicy::buffer`] is `u64`; Petri-net place capacity is
//! `Option<NonZeroU32>` ([`crate::petri::Place::capacity`]). We
//! convert by clamping to `u32::MAX` and rejecting `buffer == 0` (an
//! upstream invariant — `TransferPolicy::default().buffer == 1` and
//! the schedule parser does not accept `buffer=0`). The conversion is
//! `expect`-guarded so a future grammar relaxation that lets through
//! a 0 produces a loud failure here rather than a silently-unbounded
//! place.
//!
//! ### Initial markings — what we DO and DO NOT translate
//!
//! PRD §8.2 says "pipeline depth / latency-hiding head-start" maps to
//! initial markings on places. The ACFG today carries `TransferPolicy`
//! (sync/async/buffer/notify) but NOT a `pipeline=D` loop option (no
//! schedule-IR field, no ACFG carrier). So `pipeline=D` and `reuse`
//! loop options are **not yet** translated to initial markings —
//! buffer places start at 0 tokens. This is recorded as a follow-up
//! (TASK-0028/0029 area or a dedicated task) and matches the task's
//! note that analyses come later.
//!
//! For Petri-net analysis purposes: an `async, buffer=N` transfer
//! currently has the *same* net shape as `sync` apart from the buffer
//! place's capacity. Synchrony also affects when the producer is
//! considered free to continue, but per the EventList contract that
//! manifests in the linearisation pass (TASK-0027), not in the net
//! topology emitted here.
//!
//! ## Output shape
//!
//! The returned [`Net`] is intended to be deterministic: identical
//! inputs produce structurally identical nets, with stable
//! [`PlaceId`]/[`TransitionId`] assignment. Determinism comes from:
//!
//! 1. The ACFG itself is deterministic (its construction sorts names
//!    in `BTreeMap` order; see `acfg::build_acfg` docs).
//! 2. Per-worker chains are built in `WorkerId` numeric order (a
//!    `BTreeSet<WorkerId>` iteration is sorted).
//! 3. The walk is a depth-first traversal of the ACFG tree in source
//!    order; no hash-map iteration is involved on the hot path.
//!
//! ## Honest limitations (recorded for follow-up)
//!
//! - **Iteration unrolling**. Repeat bodies are unrolled at lowering
//!    time. A range of length 1_000_000 emits 1_000_000 transitions
//!    per body kernel. Acceptable for the example schedules in
//!    `nuc-nucleus/examples/` (all have small N), but a future
//!    optimisation should fold static-bounded loops into a parametric
//!    encoding.
//!
//! - **No pipeline-depth initial markings**. See "Initial markings"
//!    above. A `pipeline=D` loop directive does not yet flow through
//!    ACFG; once it does, we set the corresponding buffer place's
//!    initial marking accordingly.
//!
//! - **Distributed placements treat the set as one entity**. If a
//!    kernel is placed on `{w0, w1, w2, w3}`, the Operation transition
//!    has arcs from/to each worker's control place. We do not (yet)
//!    replicate the firing per worker. The transfer-injection pass
//!    has the same limitation; both will be revisited once a
//!    partition pass exists (TASK-0016+).
//!
//! - **Sync transition vs barrier place**. The PRD §8.2 mapping
//!    speaks of "barrier place + N input arcs + N output arcs OR a
//!    transition with N tokens needed". We pick the second form: one
//!    transition consuming one control token from each participant
//!    and producing one back into each participant's next control
//!    place. Equivalent for analysis; simpler in the structural
//!    count (no extra place).

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use crate::acfg::{ACFGNode, Operation, SyncPlaceholder, XferPlaceholder, XferRole, ACFG};
use crate::event::{SeqTag, WorkerId};
use crate::petri::{ArcKind, Net, PlaceId, TransitionId};

// --------------------------------------------------------------------
// Public entry point
// --------------------------------------------------------------------

/// Lower an injected `ACFG` to a global [`Net`].
///
/// The input ACFG is expected to have been processed by
/// [`crate::passes::sync_inject::inject_syncs`] and
/// [`crate::passes::transfer_inject::inject_transfers`] (any order;
/// PRD §5's pipeline runs sync first then transfer, and that is the
/// shape this pass was tested against).
///
/// The resulting net contains:
/// - one `Place` per worker, per "slot" along that worker's linearised
///   sequence (the worker's control-flow chain);
/// - one `Place` per cross-worker `(seq)` tag (the buffer place),
///   with capacity = transfer policy's `buffer` field;
/// - one `Transition` per `Operation`, per `Sync`, per `Push`, per
///   `Wait`, replicated for every iteration of every enclosing
///   `Repeat`.
///
/// The function is pure and deterministic. See module docs.
pub fn acfg_to_net(acfg: &ACFG) -> Net {
    let mut builder = NetBuilder::new(acfg);
    builder.walk(&acfg.root);
    builder.finish()
}

// --------------------------------------------------------------------
// Builder
// --------------------------------------------------------------------

/// Mutable state carried during the walk.
///
/// `worker_current` maps each `WorkerId` to its current "head" control
/// place — the place whose token the next transition on that worker
/// must consume. Advancing a worker means inserting a new place and
/// updating this map.
struct NetBuilder<'a> {
    acfg: &'a ACFG,
    net: Net,
    /// For each worker, the [`PlaceId`] holding the token that the
    /// worker's next transition must consume. Initialised on demand
    /// from `acfg.name_workers`.
    worker_current: BTreeMap<WorkerId, PlaceId>,
    /// Buffer places keyed by transfer sequence tag. A Push deposits
    /// into the buffer place; the matching Wait consumes from it.
    /// Created lazily on first encounter (either side may appear
    /// first in source order; in practice Push precedes Wait because
    /// the transfer-injection pass emits them in that order, but we
    /// don't rely on it).
    buffer_places: BTreeMap<SeqTag, PlaceId>,
    /// Monotonic counter for naming control-slot places per worker.
    /// Useful only for human-readable labels; firing semantics depend
    /// solely on arc structure.
    slot_counters: BTreeMap<WorkerId, u32>,
}

impl<'a> NetBuilder<'a> {
    fn new(acfg: &'a ACFG) -> Self {
        let mut net = Net::new();
        let mut worker_current = BTreeMap::new();
        let mut slot_counters = BTreeMap::new();

        // Seed: one control place per worker with initial_marking = 1.
        // We walk `name_workers` in BTreeMap order (sorted by name) so
        // the place ids assigned for worker-0's start, worker-1's
        // start, ... are stable.
        for (name, wid) in &acfg.name_workers {
            let pid = net.add_place(
                format!("ctl_{name}_0"),
                Some(NonZeroU32::new(1).expect("1 != 0")),
                1,
            );
            worker_current.insert(*wid, pid);
            slot_counters.insert(*wid, 0);
        }

        NetBuilder {
            acfg,
            net,
            worker_current,
            buffer_places: BTreeMap::new(),
            slot_counters,
        }
    }

    fn finish(self) -> Net {
        self.net
    }

    // ----- High-level walker -----

    fn walk(&mut self, node: &ACFGNode) {
        match node {
            ACFGNode::Operation(op) => self.emit_operation(op),
            ACFGNode::Sync(s) => self.emit_sync(s),
            ACFGNode::Xfer(x) => self.emit_xfer(x),
            ACFGNode::Sequence(children) => {
                for c in children {
                    self.walk(c);
                }
            }
            ACFGNode::Repeat { range, body, .. } => {
                // Unroll. `range.len()` is the iteration count; for
                // a malformed empty range we simply emit nothing,
                // which is also the firing semantics ("zero firings").
                let count = range.end.saturating_sub(range.start).max(0) as u64;
                for _ in 0..count {
                    self.walk(body);
                }
            }
        }
    }

    // ----- Operation -----

    /// Emit a transition for `op`. Consumes one token from each
    /// worker's current control place; produces one into each
    /// worker's freshly allocated next control place; advances each
    /// participating worker's `worker_current`.
    fn emit_operation(&mut self, op: &Operation) {
        // Canonical naming: include kernel id for readability.
        let label = format!("op_k{}", op.kernel.0);

        // Distributed placement: the transition is owned by one
        // worker in `Transition::worker` (for projection). We pick
        // the lexicographically-smallest WorkerId in the set as the
        // canonical owner, matching what `transfer_inject` does on
        // its `src`/`dst`. The projection pass (TASK-0027) handles
        // multi-worker replication later.
        let canonical_worker = op.workers.iter().next().copied();
        let tid = self.net.add_transition(label, canonical_worker);

        // Connect to every participating worker's control chain.
        for wid in &op.workers {
            self.thread_through_worker(*wid, tid);
        }
    }

    // ----- Sync -----

    fn emit_sync(&mut self, s: &SyncPlaceholder) {
        // The sync-injection pass elides syncs with fewer than 2
        // participants, but we don't trust that here — we still
        // tolerate it as a single-worker tick. The resulting
        // transition has no cross-worker constraint and behaves like
        // an Operation on the lone participant.
        let label = "sync_barrier".to_string();
        let canonical = s.participants.iter().next().copied();
        let tid = self.net.add_transition(label, canonical);

        for wid in &s.participants {
            self.thread_through_worker(*wid, tid);
        }
    }

    // ----- Xfer -----

    fn emit_xfer(&mut self, x: &XferPlaceholder) {
        // Naming: include role and seq so the DOT output is legible.
        let label = match x.role {
            XferRole::Push => format!("push_seq{}", x.seq.0),
            XferRole::Wait => format!("wait_seq{}", x.seq.0),
        };
        // Owning worker for projection:
        //   Push  -> the producer (src)
        //   Wait  -> the consumer (dst)
        let owner = match x.role {
            XferRole::Push => x.src,
            XferRole::Wait => x.dst,
        };
        let tid = self.net.add_transition(label, Some(owner));

        // Thread through the owning worker's control chain (same as
        // Operation): consume current control, produce new control.
        self.thread_through_worker(owner, tid);

        // Buffer place: created on first encounter (Push or Wait),
        // sized by the transfer's policy.
        let bpid = self.buffer_place_for(x);

        match x.role {
            XferRole::Push => {
                // Push deposits one token into the buffer place.
                self.net.add_arc(ArcKind::TtoP, bpid, tid, 1);
            }
            XferRole::Wait => {
                // Wait consumes one token from the buffer place.
                self.net.add_arc(ArcKind::PtoT, bpid, tid, 1);
            }
        }
    }

    /// Allocate (or return the existing) buffer place for transfer
    /// `x`. Capacity comes from `x.policy.buffer`.
    fn buffer_place_for(&mut self, x: &XferPlaceholder) -> PlaceId {
        if let Some(pid) = self.buffer_places.get(&x.seq) {
            return *pid;
        }

        // Cap the u64 schedule field at u32::MAX; the petri-net
        // library uses NonZeroU32 for place capacity. A `buffer=0`
        // upstream would already have been rejected by the schedule
        // grammar/lowering, so 0 here is a programming error.
        let cap_u64 = x.policy.buffer;
        assert!(
            cap_u64 > 0,
            "transfer buffer must be >= 1 (saw 0 on seq {}); upstream invariant violated",
            x.seq.0
        );
        let cap_u32 = u32::try_from(cap_u64).unwrap_or(u32::MAX);
        let cap = NonZeroU32::new(cap_u32).expect("cap > 0 by the assertion above");

        // Look up data symbol name for readability.
        let data_name = self
            .acfg
            .name_data
            .iter()
            .find_map(|(n, id)| {
                if *id == x.data {
                    Some(n.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("?");
        let name = format!("buf_{data_name}_seq{}", x.seq.0);

        let pid = self.net.add_place(name, Some(cap), 0);
        self.buffer_places.insert(x.seq, pid);
        pid
    }

    // ----- Helpers -----

    /// Connect transition `tid` into worker `wid`'s control chain:
    /// consume from the worker's current control place, produce into
    /// a freshly allocated next control place, and advance the
    /// worker's head.
    fn thread_through_worker(&mut self, wid: WorkerId, tid: TransitionId) {
        let prev = *self
            .worker_current
            .get(&wid)
            .expect("worker control chain seeded for every named worker");

        // Allocate the next control place for this worker.
        let counter = self.slot_counters.entry(wid).or_insert(0);
        *counter += 1;
        let worker_name = self
            .acfg
            .name_workers
            .iter()
            .find_map(|(n, id)| if *id == wid { Some(n.as_str()) } else { None })
            .unwrap_or("?");
        let next_pid = self.net.add_place(
            format!("ctl_{worker_name}_{}", counter),
            Some(NonZeroU32::new(1).expect("1 != 0")),
            0,
        );

        // Wire the transition through this worker.
        self.net.add_arc(ArcKind::PtoT, prev, tid, 1);
        self.net.add_arc(ArcKind::TtoP, next_pid, tid, 1);

        // Advance head.
        self.worker_current.insert(wid, next_pid);
    }
}
