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
//! - **Push/Wait pairing across scope boundaries (HISTORICAL gap, now
//!   closed).** `transfer_inject`'s `splice_pushes_for_waits` only
//!   splices Pushes *within* one sequence; on its own that left a
//!   producer-at-top-level / consumer-inside-`for` shape (e.g. example
//!   02-split-add: `load_input` on host, then
//!   `for i { add(a[i], b[i]) }` on `w0`) with Wait nodes and no
//!   matching Push. That gap was CLOSED by TASK-0136 (Pass A
//!   `hoist_invariant_waits` lifts the loop-invariant input Waits out
//!   of the `for`; Pass B `splice_pushes_global` places the matching
//!   host-side Push across the scope boundary) and the sibling
//!   TASK-0149 / TASK-0151 / TASK-0364 work. As of TASK-0428
//!   (cycle-242) PRD §8.3 invariant (2) — matched Push/Wait pairs —
//!   holds on the projected EventList for the ENTIRE example corpus
//!   (all 55 schedules; see
//!   `tests/petri_to_events.rs::task0428_inv2_holds_for_entire_example_corpus`).
//!   This pass faithfully projects whatever it receives; it no longer
//!   surfaces unmatched Waits for shipping programs. The mp-tcp/uds
//!   `host_mediation_inject` + `host_data_relay_inject` post-passes
//!   (which re-route Push/Wait through host) were subsequently verified
//!   inv(2)-clean too (TASK-0422.01, cycle-243,
//!   `driver/tests/task0422_01_inv2_post_mediation.rs`), and
//!   `validate_event_lists` (the FULL surface incl inv(2)) is now wired
//!   as a hard production gate at the driver's final EventList-
//!   consumption point (TASK-0422, cycle-244 — `driver/src/main.rs`
//!   `cmd_build`, before `dispatch::dispatch_backend`).
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

    // Contract-internal gate (TASK-0107). The strictly per-worker
    // invariants of the EventList contract — no self-push, non-empty
    // Sync participants, Alloc/Free pairing — must hold immediately
    // at the projection boundary; a regression in this pass or in any
    // upstream injector (`sync_inject`, `transfer_inject`,
    // `block_transform`, `partition_workers`) that violates them is a
    // bug.
    //
    // Why ONLY the strictly per-worker invariants and not the full
    // [`validate_event_lists`] (which also checks invariant (2),
    // Push/Wait pair matching):
    //
    // HISTORICALLY this site excluded inv(2) because `transfer_inject`'s
    // cross-scope splicing limitation left legitimate shipping programs
    // (02-split-add) with unmatched Wait events, so asserting (2) here
    // would have crashed debug builds on valid input. That premise is
    // now STALE — TASK-0136 (Pass A hoist + Pass B cross-scope Push
    // splice) and siblings closed the gap, and TASK-0428 (cycle-242)
    // empirically verified inv(2) holds on the projected EventList for
    // the ENTIRE example corpus (see the module docs above and
    // `tests/petri_to_events.rs::task0428_inv2_holds_for_entire_example_corpus`).
    //
    // The assert is deliberately LEFT as the per-worker subset here,
    // for a reason that is NOT the stale one: this projection boundary
    // runs BEFORE the mp-tcp/uds `host_mediation_inject` /
    // `host_data_relay_inject` post-passes that re-route Push/Wait
    // through host, AND it is reached by the driver's PRE-mediation
    // host-election preview projections (`driver/src/main.rs` ~484,
    // ~537) — at those intermediate points inv(2) need not yet hold, so
    // asserting the FULL validator here would fire on valid intermediate
    // state. Invariant (2) is instead enforced by
    // [`crate::event_validate::validate_event_lists`] as a HARD
    // production gate at the FINAL EventList-consumption point in the
    // driver (`cmd_build`, before `dispatch::dispatch_backend` —
    // TASK-0422 cycle-244), which is the only point where the EventList
    // is past mediation and actually handed to a backend. inv(2) over
    // the post-mediation EventList was proven clean for all 4 mp-*
    // backends by TASK-0422.01 (cycle-243), which is what made wiring
    // that gate safe.
    //
    // Release builds: the `debug_assert!` is compiled out entirely.
    // The validator has zero cost in production compilation.
    debug_assert!(
        crate::event_validate::validate_event_lists_strict_per_worker(&out).is_ok(),
        "acfg_to_events produced EventLists that violate the per-worker contract: {:?}",
        crate::event_validate::validate_event_lists_strict_per_worker(&out).err()
    );

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
            // The bounded early-exit halt predicate (epic S4). CONSUMED
            // by TASK-0341.02.01.05.04: it rides each per-worker
            // `Event::Loop.break_cond` below, where the single-worker
            // sequential backend emits the runtime `break`. `None` for
            // every plain `for` loop (byte-identical projection, no
            // codegen change). The field stays bound (not `..`-elided)
            // so a future field addition is compiler-forced here.
            break_cond,
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
                    // Silent-by-design drop (module docs lines 304-308): a
                    // worker that contributes NOTHING to a loop body gets no
                    // `Event::Loop` at all, not an empty-bodied one. This is
                    // INTENTIONAL and correct — NOT the build_dataflow
                    // silent-statement-drop class. The defense-in-depth guard
                    // for the one upstream-population-bug shape lives in
                    // `debug_assert_partition_assigned_nonempty` (TASK-0419).
                    debug_assert_partition_assigned_nonempty(wid, per_worker_override, *iter_var);
                    continue;
                }
                let projected_range = match per_worker_override.and_then(|m| m.get(&wid)) {
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
                    // `check_frame` is populated later by
                    // `passes::inject_check_frames`, which joins the
                    // schedule's `check loop V` directives against the
                    // ACFG's `name_iter_vars` map (TASK-0052.02). The
                    // projection here does not see SchedIR — keeping
                    // `acfg_to_events` signature unchanged keeps every
                    // existing test call site stable.
                    check_frame: None,
                    // The `for..until` early-exit predicate (epic S4,
                    // TASK-0341.02.01.05.04). The SAME predicate rides
                    // every worker's projected Loop: single-worker
                    // sequential codegen (`pthreads-sync`) is the only
                    // consumer this slice, and a `for..until` is rejected
                    // for multi-worker partitioning today, so there is no
                    // per-worker predicate divergence to model. (When
                    // multi-worker break emit lands — S7,
                    // TASK-0341.02.01.08 — revisit whether a partitioned
                    // convergence loop needs a per-worker predicate; for
                    // now the uniform clone is the faithful projection.)
                    // `None` for every plain `for` -> byte-identical to
                    // the pre-S4 projection (no e2e change).
                    break_cond: break_cond.clone(),
                });
            }
        }
    }
}

/// Defense-in-depth guard (TASK-0419) for the empty-body `continue` in
/// the `Repeat` arm of [`walk`]. It is the one site a future regression
/// of the silent-statement-drop class could hide behind: a worker that
/// projects an empty body gets no `Event::Loop` at all (intentional;
/// module docs lines 304-308). That is correct for a worker that
/// legitimately does nothing in the loop — but NOT for a worker that
/// `partition=workers` / `partition=blocks2d` assigned an *exclusive*
/// iteration slice (present in `per_worker_override`, i.e.
/// `partition_worker_ranges[iter_var]`). Such a worker must contribute
/// at least the body op(s) for its slice; an empty body means an
/// upstream pass (`partition_workers` / `partition_blocks2d` /
/// `sync_inject` / `transfer_inject` / `halo_inference` /
/// `reuse_inference`) dropped its work silently. We assert loudly in
/// dev/test rather than swallow it.
///
/// ## Why this cannot false-fire on valid input (AC#1 precondition)
///
/// Empirically DISCHARGED (TASK-0419): an unconditional `NUC_TRACE`
/// probe at the call site fired ZERO times across (a) the full e2e
/// matrix (385 cells), (b) a direct-driver sweep of all 18
/// `partition=workers` / `partition=blocks2d` (example, schedule) pairs
/// × 7 tier-1 backends, and (c) the full 1243-test workspace suite.
/// `body_events.is_empty()` is in fact never true on any shipping
/// input: a worker is a `scratch` key ONLY if an `emit_*` inserted ≥1
/// event for it (every `out.entry(..).or_default()` in `emit_operation`
/// / `emit_sync` / `emit_xfer` is immediately followed by `.push(..)`),
/// and the nested `Repeat` arm itself `continue`s rather than inserting
/// an empty vec. So no legitimate partition-assigned empty case exists,
/// and the assert cannot panic-on-valid-input. It catches only a
/// genuine upstream-population regression that newly inserts an
/// empty-bodied scratch entry for a partition-assigned worker.
///
/// ## Release cost
///
/// `debug_assert!` is compiled out entirely in release builds — no
/// e2e / determinism / byte-output change. Mirrors the
/// `validate_event_lists_strict_per_worker` precedent in
/// [`acfg_to_events`].
fn debug_assert_partition_assigned_nonempty(
    wid: WorkerId,
    per_worker_override: Option<&BTreeMap<WorkerId, Range<i64>>>,
    iter_var: IterVar,
) {
    debug_assert!(
        per_worker_override.and_then(|m| m.get(&wid)).is_none(),
        "petri_to_events: worker {wid:?} was assigned an exclusive \
         partition=workers slice for iter_var {iter_var:?} but projected an \
         EMPTY loop body — an upstream pass dropped this worker's body work \
         (silent-drop regression, TASK-0419)"
    );
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

// --------------------------------------------------------------------
// Unit tests — defense-in-depth guard (TASK-0419)
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `per_worker_override` map assigning `wid` an exclusive
    /// slice for one iter_var, the way `partition_workers` /
    /// `partition_blocks2d` populate `partition_worker_ranges[iter_var]`.
    fn override_with(wid: WorkerId, range: Range<i64>) -> BTreeMap<WorkerId, Range<i64>> {
        let mut m = BTreeMap::new();
        m.insert(wid, range);
        m
    }

    /// BITE TEST (AC#3). A worker that WAS assigned an exclusive
    /// partition=workers slice but projects an EMPTY body must trip the
    /// guard. This is the upstream-population-bug shape the guard exists
    /// to catch loudly (silent-drop class, TASK-0419).
    ///
    /// PROVE-THE-CHECK-BITES: deleting the `debug_assert!` inside
    /// `debug_assert_partition_assigned_nonempty` makes this test FAIL
    /// (the call returns normally instead of panicking) — verified
    /// manually during TASK-0419 by removing the assert and observing
    /// `note: test did not panic as expected`.
    ///
    /// TASK-0291 dev-vs-release trap: `debug_assert!` is compiled OUT
    /// under `--release`, so this `#[should_panic]` would FAIL in the
    /// release profile (`just test-release`). Gate it with
    /// `#[cfg(debug_assertions)]` so it only compiles/runs in dev; the
    /// release run sees no such test and stays green.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "projected an EMPTY loop body")]
    fn partition_assigned_worker_with_empty_body_trips_guard() {
        let wid = WorkerId(2);
        let iter_var = IterVar(7);
        // wid is present in the per-worker override (partition assigned
        // it the exclusive slice 4..8) yet its body projected nothing.
        let ov = override_with(wid, 4..8);
        debug_assert_partition_assigned_nonempty(wid, Some(&ov), iter_var);
    }

    /// NEGATIVE control: a worker that is NOT in the per-worker override
    /// (e.g. host, or any worker with no partition=workers slice for
    /// this iter_var) with an empty body is the LEGITIMATE silent-by-
    /// design drop — the guard must NOT fire. Proves the guard is
    /// narrow (no false-positive on the intentional do-nothing case).
    #[test]
    fn non_partition_assigned_worker_with_empty_body_does_not_trip() {
        let iter_var = IterVar(7);
        // Override exists for a DIFFERENT worker (1); worker 2 is not in it.
        let ov = override_with(WorkerId(1), 0..4);
        debug_assert_partition_assigned_nonempty(WorkerId(2), Some(&ov), iter_var);
        // And the no-override-at-all case (no partition= on this loop).
        debug_assert_partition_assigned_nonempty(WorkerId(2), None, iter_var);
    }
}
