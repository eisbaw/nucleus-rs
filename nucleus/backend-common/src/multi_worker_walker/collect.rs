//! Event-walk helpers (recurse into Event::Loop bodies). Lifted from
//! per-backend duplication into single sources of truth (TASK-0239 /
//! TASK-0300). Consumed by every tier-1 backend's `multi_worker::Plan::build`
//! (and by backend-common's unit tests).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};

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
