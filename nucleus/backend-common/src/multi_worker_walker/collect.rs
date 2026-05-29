//! Event-walk helpers (recurse into Event::Loop bodies). Lifted from
//! per-backend duplication into single sources of truth (TASK-0239 /
//! TASK-0300). Consumed by every tier-1 backend's `multi_worker::Plan::build`
//! (and by backend-common's unit tests).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use super::ctx::RendezvousId;

/// Collect every `(DataId, SeqTag)` pair appearing on a Push or Wait
/// event in `events` (descending into Loop bodies). The map's value is
/// the pair's tile, copied from the first event sighting; the same
/// `seq` is carried on both endpoints by the XferPlaceholder
/// construction (TASK-0018) so first-sighting is well-defined.
pub fn collect_xfer_pairs(events: &[Event], out: &mut BTreeMap<(DataId, SeqTag), IterTile>) {
    for e in events {
        match e {
            Event::Push {
                data, seq, tile, ..
            }
            | Event::Wait {
                data, seq, tile, ..
            } => {
                out.entry((*data, *seq)).or_insert_with(|| tile.clone());
            }
            Event::Loop { body, .. } => collect_xfer_pairs(body, out),
            _ => {}
        }
    }
}

/// Build a `(DataId, SeqTag) -> IterTile` map by folding
/// [`collect_xfer_pairs`] across every worker's projected events.
///
/// Single source of truth for the construction shape that all four
/// tier-1 backends (pthreads-sync, pthreads-async, mp-tcp-bufsync,
/// mp-tcp-event) had been duplicating inline (TASK-0300, cycle 130
/// hardening from TASK-0296 cycle-116 architect P1.2).
///
/// # Contract
///
/// First-sighting on a given `(DataId, SeqTag)` wins; later sightings
/// are dropped. Under valid input, both endpoints carry the same
/// `IterTile` by the XferPlaceholder construction (TASK-0018), so the
/// dropped sightings agree with the kept one and the choice is
/// observationally a no-op. "First" means *first in the input
/// iterator's order* — the helper has no opinion about what that order
/// is; it inherits it from the caller.
///
/// # Current-caller convention (informational, not part of the contract)
///
/// All four tier-1 backends pass `per_worker.values()` where
/// `per_worker: BTreeMap<WorkerId, Vec<Event>>`. `BTreeMap::values()`
/// iterates in key-ascending order, so for those callers "first
/// sighting" = the lowest-`WorkerId` worker whose event list names
/// that `(DataId, SeqTag)`. This is what the cycle-130 pin test
/// `first_sighting_wins_on_conflicting_tiles` relies on. A different
/// caller (e.g. `Vec<&[Event]>::iter().copied()` from the cycle-131
/// `vec_of_slices_input_compiles_and_collects` test) sees
/// insertion-order, not WorkerId-ascending — both are valid uses; the
/// helper does not assume the BTreeMap shape.
///
/// The output is keyed only on `(DataId, SeqTag)`, so input iteration
/// order cannot leak into the output's KEY ordering — only into which
/// tile wins on a conflict.
pub fn collect_pair_tiles<'a, I, T>(events_per_worker: I) -> BTreeMap<(DataId, SeqTag), IterTile>
where
    I: IntoIterator<Item = &'a T>,
    T: AsRef<[Event]> + 'a + ?Sized,
{
    let mut out: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    for evs in events_per_worker {
        collect_xfer_pairs(evs.as_ref(), &mut out);
    }
    out
}

/// Per-worker visit of Push/Wait events to collect the worker's
/// rendezvous-id touch set. Descends into `Event::Loop` bodies.
///
/// Replaces the per-backend `collect_worker_slots` /
/// `collect_worker_rings` — both walked identically, only the value
/// type alias differed (`SlotId = RingId = usize`).
pub fn collect_worker_rendezvous(
    events: &[Event],
    ids: &BTreeMap<(DataId, SeqTag), RendezvousId>,
    out: &mut BTreeSet<RendezvousId>,
) {
    for e in events {
        match e {
            Event::Push { data, seq, .. } | Event::Wait { data, seq, .. } => {
                if let Some(s) = ids.get(&(*data, *seq)) {
                    out.insert(*s);
                }
            }
            Event::Loop { body, .. } => collect_worker_rendezvous(body, ids, out),
            _ => {}
        }
    }
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier, so there is
/// nothing to validate / reject here any more).
pub fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants, sync, ..
            } => f(*sync, participants),
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}

/// Detect the **overlapping-write accumulator fan-in** pattern at the
/// codegen layer (TASK-0343, cycle 189).
///
/// Returns the set of `(DataId, SeqTag)` pairs for which the
/// receiver-side `Event::Wait` MUST emit element-wise accumulate
/// (sum identity) instead of the default whole-array overwrite assign.
///
/// # The pattern this detects
///
/// 08-histogram/distributed: 4 compute workers each compute a local
/// partial histogram over their input partition and Push the FULL
/// 16-element output array to the host. The host then has 4 Waits on
/// the same data symbol, all carrying whole-array tiles. Pre-cycle-189
/// `render_wait_assign` emitted `histogram = slot_N.wait();` for each
/// (last-write-wins); the cycle-189 fix replaces each with an
/// element-wise `wrapping_add` accumulate into the host's
/// zero-initialised destination.
///
/// Contrast 03-reduction's `partials[w]` shape (the DISJOINT-write
/// accumulator, NOT this pattern): each worker pushes ONE slot of
/// `partials`, the Wait tiles carry per-worker slice ranges (`(w,
/// w..w+1)`), `wait_slice` returns `Some(WaitSlice::Flat)`, and the
/// existing slice-paste arm emits the bit-correct gather. This
/// helper does NOT classify those Waits as accumulate (the predicate
/// requires whole-array tiles).
///
/// # The predicate
///
/// For each `DataId` that has N>=2 Waits in `events` where every Wait's
/// tile (from `pair_tiles`) is whole-array — i.e. either has no bounds
/// at all, OR the data is scalar (zero dims), OR every bound's range
/// covers the corresponding data dim's full source range — all of the
/// `(DataId, SeqTag)` pairs for those Waits enter the output set.
///
/// "Whole-array" here matches the same condition `wait_slice` (sibling
/// `wait.rs`) uses to return `None` (the whole-array assign arm), so
/// pre-cycle-189 emit identity holds for any Wait this helper does NOT
/// classify as accumulate: every Wait that would have gone through the
/// `name = rhs;` arm AND is not part of an N>=2 fan-in group still
/// emits exactly that pre-cycle-189 string.
///
/// # Scope LIMITS (filed as TASK-0343 follow-ups)
///
/// - **Sum identity only**: the consumer (`render_wait_assign`) emits
///   `wrapping_add` for integer scalar types and returns
///   `EmitError::ContractGap` for floats / bool (sum identity is not
///   defined; floats also collide with PRD §10.1 bit-identity).
/// - **No algorithm-level cross-check**: detection is purely structural
///   (N>=2 whole-array Waits). An exotic schedule that emits multiple
///   whole-array Pushes for non-accumulator semantics would mis-combine.
///   For every shipped schedule today (08-histogram/distributed is the
///   load-bearing case), the structural pattern is equivalent to the
///   algorithm-level accumulator pattern (LHS appears in RHS).
/// - **No iterative (Repeat-body) accumulator**: Waits inside a
///   `Event::Loop` body carry per-iter tiles (the iter-var range is one
///   of the consulted dims), so they do NOT match the whole-array
///   predicate and the helper does not classify them. Multi-pass
///   time-step accumulator patterns are a separate follow-up.
///
/// Descends into `Event::Loop` bodies on the recursion side ONLY to
/// gather Waits — the whole-array-tile predicate naturally excludes
/// in-loop Waits (their tiles carry the enclosing iter-var range).
pub fn collect_accumulate_waits(
    events: &[Event],
    sidecar: &NameSidecar,
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
) -> BTreeSet<(DataId, SeqTag)> {
    // Group Waits by data: data -> Vec<seq>.
    let mut waits_per_data: BTreeMap<DataId, Vec<SeqTag>> = BTreeMap::new();
    walk_waits(events, &mut waits_per_data);

    let mut out: BTreeSet<(DataId, SeqTag)> = BTreeSet::new();
    for (data, seqs) in waits_per_data {
        if seqs.len() < 2 {
            continue;
        }
        let all_whole = seqs.iter().all(|seq| {
            pair_tiles
                .get(&(data, *seq))
                // Unified classifier (TASK-0355 cycle 225): route through
                // `is_whole_array_recv` so both this accumulator-detection
                // call site AND the let-at-wait classifier at
                // `collect_let_at_wait_inner` (collect.rs:392) consult the
                // same guard chain in `wait_slice` (rank, OOB, sidecar
                // lookup). `Err` arms swallowed to `false` — matches the
                // sibling site's `.unwrap_or(false)` convention and the
                // pre-cycle-225 `is_whole_array_tile` silent-false on
                // axis-beyond-dims (now an explicit `Err` from wait_slice
                // that is treated as not-whole-array here).
                //
                // Conservative-default rationale (cycle-189 architect P3.2,
                // preserved across the cycle-225 unification): if a Wait's
                // (data, seq) is missing from `pair_tiles`, we have NO
                // evidence the tile is whole-array; do NOT classify as
                // accumulate. The branch is structurally unreachable for
                // shipped schedules — `collect_pair_tiles` (sibling, this
                // module) is contracted to record every (Push|Wait) pair
                // (TASK-0018 XferPlaceholder construction). If a future
                // projection-layer regression breaks that contract,
                // falling back to pre-cycle-189 `name = rhs;` overwrite
                // emit is a forward-compatible loss (the user still sees
                // wrong output, but the cycle-189 fix does not silently
                // change behaviour here on a different missing-tile
                // surface).
                .map(|tile| super::wait::is_whole_array_recv(sidecar, data, tile).unwrap_or(false))
                .unwrap_or(false)
        });
        if all_whole {
            for seq in seqs {
                out.insert((data, seq));
            }
        }
    }
    out
}

/// Per-data Wait-seq accumulator. Descends into Loop bodies (in-loop
/// Waits are still gathered; the whole-array predicate naturally
/// filters them out at classification time).
fn walk_waits(events: &[Event], out: &mut BTreeMap<DataId, Vec<SeqTag>>) {
    for e in events {
        match e {
            Event::Wait { data, seq, .. } => {
                out.entry(*data).or_default().push(*seq);
            }
            Event::Loop { body, .. } => walk_waits(body, out),
            _ => {}
        }
    }
}

/// Visit every `Event::Wait` / `Event::Fire` output to build the
/// three sets needed for the pre-init computation:
///
/// - `waited`: cross-worker inputs the worker WAITs on (these will
///   be overwritten by the .wait() and need to exist as locals).
/// - `whole`: data the worker writes via a whole-array Fire output
///   (let-bound at the Fire site; no pre-init needed).
/// - `indexed`: data the worker writes via an indexed Fire output
///   (must be pre-initialised so the indexed assign has something to
///   write into).
///
/// A worker's pre-init set is `waited UNION (indexed - whole)`.
pub fn collect_pre_init_sets(
    events: &[Event],
    waited: &mut BTreeSet<DataId>,
    whole: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                waited.insert(*data);
            }
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => collect_pre_init_sets(body, waited, whole, indexed),
            _ => {}
        }
    }
}

/// TASK-0349 cycle 220: classify which DataIds have a provably-dead
/// pre-init `let mut <name>: Vec<..> = vec![0; N];` because every
/// `Event::Wait` on them is a whole-array recv (and they are not in
/// the accumulate-fan-in set nor the indexed-Fire-write set, both of
/// which need the zero-init to be live).
///
/// For DataIds in the returned set, the per-backend pre-init pass
/// omits the `let mut` line and the walker's `render_wait_assign`
/// emits `let <name> = <rhs>;` at the recv site (declare-and-assign in
/// one statement). The result is a Vec<T> coming into scope at the
/// first .wait() call, no dead zero-init, no `unused_assignments`
/// warning on cargo build of the emitted project.
///
/// Conservative on shape-errors: if `wait_slice` returns Err for any
/// of the data's Waits, the data is kept OUT of the returned set (the
/// pre-init stays — emit-time render_wait_assign will surface the
/// same error to the caller).
///
/// Inputs:
/// - `events`: the worker's projected event list (descends into Loop
///   bodies).
/// - `pair_tiles`: `(DataId, SeqTag) -> IterTile` map; missing entry
///   means no tile (whole-array transfer).
/// - `sidecar`: needed by `is_whole_array_recv` to read data dims.
/// - `accumulate_data`: DataIds classified as accumulate-fan-in
///   anywhere in this worker (passed as a flat-by-data slice of the
///   per-(worker, data, seq) accumulate set; the caller filters on
///   `WorkerId` upstream).
/// - `indexed`: DataIds the worker writes via indexed Fire output
///   (computed by `collect_pre_init_sets`).
pub fn collect_let_at_wait_data(
    events: &[Event],
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    sidecar: &NameSidecar,
    accumulate_data: &BTreeSet<DataId>,
    indexed: &BTreeSet<DataId>,
) -> BTreeSet<DataId> {
    let mut waited: BTreeSet<DataId> = BTreeSet::new();
    let mut not_all_whole: BTreeSet<DataId> = BTreeSet::new();
    collect_let_at_wait_inner(events, pair_tiles, sidecar, &mut waited, &mut not_all_whole);
    let mut out: BTreeSet<DataId> = BTreeSet::new();
    for d in &waited {
        if not_all_whole.contains(d) {
            continue;
        }
        if accumulate_data.contains(d) {
            continue;
        }
        if indexed.contains(d) {
            continue;
        }
        out.insert(*d);
    }
    out
}

fn collect_let_at_wait_inner(
    events: &[Event],
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    sidecar: &NameSidecar,
    waited: &mut BTreeSet<DataId>,
    not_all_whole: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, seq, .. } => {
                waited.insert(*data);
                let is_whole = match pair_tiles.get(&(*data, *seq)) {
                    None => true,
                    Some(tile) => {
                        super::wait::is_whole_array_recv(sidecar, *data, tile).unwrap_or(false)
                    }
                };
                if !is_whole {
                    not_all_whole.insert(*data);
                }
            }
            Event::Loop { body, .. } => {
                // Descend into the loop body: a whole-array Wait
                // buried inside a loop body IS classified let-at-wait.
                // SCOPE HAZARD (TASK-0356 cycle 222, characterized;
                // fix filed as TASK-0364): classifying an in-loop Wait
                // as let-at-wait means the sibling `wait::
                // render_wait_assign` emits its `let {name} = {rhs};`
                // INSIDE the emitted `for { }` block. A consumer of
                // that data at the ENCLOSING outer scope would read it
                // out of scope. NOT producible today —
                // `transfer_inject` co-locates each Wait with its
                // consumer (see the TASK-0356 note at the `wait.rs`
                // emit site and `tests/wait_let_at_wait_loop_scope.rs`).
                // A scope-aware fix would exclude such a Wait here when
                // its data is consumed at an outer scope.
                collect_let_at_wait_inner(body, pair_tiles, sidecar, waited, not_all_whole);
            }
            _ => {}
        }
    }
}
