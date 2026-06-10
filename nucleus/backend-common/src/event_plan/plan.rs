//! The shared multi-process emit `Plan` for the async event-reactor
//! backends (mp-tcp-event, mp-uds-event).
//!
//! `Plan<'a, T>` holds every cross-worker invariant the per-worker
//! codegen depends on: the used-worker list, host election, the
//! `(DataId, SeqTag) -> ChanId` registry, per-pair capacity, per-pair
//! `(src, dst)` workers, per-pair `IterTile`, the overlapping-write
//! accumulator set, barrier participants, plus typed accessors and the
//! peer-index routing. The `T: EventTransport` type parameter is the
//! SOLE axis of variation between mp-tcp-event (TCP loopback) and
//! mp-uds-event (Unix domain sockets); `Plan` carries no `T` value,
//! only a `PhantomData<fn() -> T>`, and dispatches the transport
//! variation through `T::method(..)` at the emit sites in
//! `worker_program.rs` (sibling file).
//!
//! Lifted from the two backends' verbatim-duplicate
//! `multi_worker/mod.rs` Plan (TASK-0044.03.02).

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::elect_host_from_worker_names;
use crate::event_plan::encode;
use crate::event_plan::walkers::{collect_push_pairs, detect_wait_before_push_hazard};
use crate::event_plan::EventTransport;
use crate::multi_worker_walker::{self as walker, RendezvousId};
use crate::EmitError;

/// Stable identifier for one event-reactor channel — the runtime
/// `chan_<id>` variable that wraps `(DataId, SeqTag)`'s reactor route.
/// `usize` alias for [`RendezvousId`] (the shared walker key type).
///
/// `pub(crate)`: an intra-crate type only — no backend references
/// `event_plan::ChanId`. The earlier `pub use plan::{ChanId, ..}`
/// re-export was a dead external surface (TASK-0412); kept symmetric
/// with the `mpi_plan` sibling.
pub(crate) type ChanId = RendezvousId;

/// Per-worker codegen Plan: every fact needed to emit one
/// `src/bin/<wname>.rs`. Field set mirrors `pthreads-async`'s Plan
/// modulo the per-pair PEER index needed for the per-(src,dst) outbound
/// queue and the host-mediated barrier topology check. Parameterised
/// over the per-backend transport `T`. See the module docstring.
pub struct Plan<'a, T: EventTransport> {
    // Visibility: only `used_workers` is `pub` (the backends' `emit()`
    // loop iterates `for w in &plan.used_workers`). Every other field is
    // consumed only by sibling `event_plan` modules (relay / walkers /
    // worker_program), so it is `pub(crate)` — NOT `pub`. Mirrors the
    // post-TASK-0340.04 visibility-hygiene precedent (over-widened Plan
    // items tightened to the minimum the consumers need).
    pub(crate) per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    pub(crate) names: &'a NameTables,
    pub(crate) sidecar: &'a NameSidecar,
    pub used_workers: Vec<WorkerId>,
    pub(crate) host_worker: WorkerId,
    /// Cross-worker Push/Wait pairs `(DataId, SeqTag) -> chan index`.
    pub(crate) chan_ids: BTreeMap<(DataId, SeqTag), ChanId>,
    /// Per-pair channel capacity from `transfer DATA : buffer=N`
    /// (TASK-0233).
    pub(crate) chan_caps: BTreeMap<(DataId, SeqTag), u64>,
    /// Per-pair (src, dst) workers. The reactor's outbound queue is
    /// keyed on `(seq, peer_idx)`; the producer side puts (seq, dst)
    /// and the consumer side reads inbound[seq] (independent of src).
    pub(crate) chan_pairs: BTreeMap<(DataId, SeqTag), (WorkerId, WorkerId)>,
    /// Per-pair iteration tile (TASK-0117 host-side gather).
    pub(crate) pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343 cycle 189). Mirrors pthreads-sync's
    /// and pthreads-async's `multi_worker::Plan::accumulate_waits`
    /// field; populated by `walker::collect_accumulate_waits` per
    /// worker and unioned with the WorkerId. Consumed by the shared
    /// event walker via `WalkerCtx::accumulate_waits` to switch
    /// Event::Wait emit from whole-array overwrite assign to
    /// element-wise `wrapping_add` accumulate. Empty for every cell
    /// without an overlapping-write fan-in.
    pub(crate) accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
    /// SyncTag -> participants. Same shape as the sync-TCP backends'.
    pub(crate) barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>>,
    /// Zero-sized witness of the per-backend transport. `Plan` has no
    /// `T` value; the variation is dispatched through `T`'s associated
    /// functions/consts at the emit sites.
    pub(crate) _transport: PhantomData<fn() -> T>,
}

impl<'a, T: EventTransport> Plan<'a, T> {
    /// Build the Plan; returns `EmitError::ContractGap` for any
    /// invariant violation reachable from valid input (cap missing,
    /// host-excluding barrier, malformed projection).
    pub fn build(
        per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
        names: &'a NameTables,
        sidecar: &'a NameSidecar,
    ) -> Result<Self, EmitError> {
        let used_workers: Vec<WorkerId> = per_worker
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(w, _)| *w)
            .collect();

        // CHECK-ORDER NOTE (TASK-0255 cycle 90, refreshed TASK-0327
        // cycle 149): the ContractGap checks below are A
        // (used_workers<2) -> B (Wait/no-Push) -> C (missing sidecar
        // buffer) -> host-excluding-barrier -> D (defensive
        // src==host==dst projection malformedness). Branch D used to
        // reject any src/dst that were both non-host (the cycle-79
        // worker-to-worker gap); cycle 149 lifted that via
        // host-relay (see `render_relay_phase` + `Reactor::relay_one`),
        // so Branch D now bites only the genuinely-malformed
        // `src == dst == host` projection. This order is load-bearing
        // for the negative-path test fixtures in the backends'
        // `tests/multi_worker_emit.rs` + the in-crate Branch-A test —
        // each test must NOT trip any EARLIER check to exercise its
        // target branch. A new check inserted between branches may
        // silently invalidate a bypass-fixture; if you reorder/add,
        // update the fixtures together.
        if used_workers.len() < 2 {
            return Err(EmitError::ContractGap(format!(
                "{backend} Plan::build requires used_workers.len() >= 2; \
                 got {n}. Single-worker is handled by emit()'s single-worker arm.",
                backend = T::BACKEND_NAME,
                n = used_workers.len(),
            )));
        }

        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule (worker literally named "host" in
        // used_workers, else smallest used WorkerId, else None ->
        // ContractGap). Lifted in TASK-0336 cycle 164 — every
        // tier-1 backend's `multi_worker::Plan::build` AND the 3
        // compiler-level driver wirings (cycles 160 / 162 / 163)
        // consume this one helper, retiring the
        // feedback-driver-must-mirror-backend-election-exactly
        // recurrence surface on the canonical path.
        let host_worker =
            elect_host_from_worker_names(&names.worker, &used_workers).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "{backend} Plan: used_workers reachable to host \
                     election but empty — invariant len() >= 2 violated",
                    backend = T::BACKEND_NAME,
                ))
            })?;

        // Collect cross-worker (DataId, SeqTag) pairs + tiles via the
        // shared backend-common helper (TASK-0300 cycle 130 hoist).
        let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> =
            walker::collect_pair_tiles(per_worker.values());

        // Collect (src, dst) per pair from Push events.
        let mut chan_pairs: BTreeMap<(DataId, SeqTag), (WorkerId, WorkerId)> = BTreeMap::new();
        for (&w, evs) in per_worker {
            collect_push_pairs(evs, w, &mut chan_pairs);
        }
        // Defensive: every (data, seq) in pair_tiles must have an
        // entry in chan_pairs (every cross-worker pair has a Push).
        for k in pair_tiles.keys() {
            if !chan_pairs.contains_key(k) {
                return Err(EmitError::ContractGap(format!(
                    "{backend} Plan: (data={:?}, seq={:?}) has a Wait \
                     but no matching Push — malformed projection",
                    k.0,
                    k.1,
                    backend = T::BACKEND_NAME,
                )));
            }
        }

        // Deterministic chan_id assignment: ascending by (DataId, SeqTag).
        let chan_ids: BTreeMap<(DataId, SeqTag), ChanId> = pair_tiles
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();

        // Capacity per pair (TASK-0233 sidecar lookup, unified into
        // XferFacts by TASK-0455.08 — read via the `xfer_buffer` accessor).
        let mut chan_caps: BTreeMap<(DataId, SeqTag), u64> = BTreeMap::new();
        for (data, seq) in pair_tiles.keys() {
            let cap = sidecar.xfer_buffer(*seq).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "{backend} Plan: (data={data:?}, seq={seq:?}) Push/Wait \
                     pair has no entry in sidecar.xfer_facts. \
                     Either build_sidecar's walker missed an Xfer placeholder \
                     (TASK-0233/TASK-0455.08 regression), or the EventList was \
                     projected without running transfer_inject first.",
                    backend = T::BACKEND_NAME,
                ))
            })?;
            chan_caps.insert((*data, *seq), cap);
        }

        // Barrier identity by SyncTag (TASK-0172).
        let mut barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            walker::collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        // Host-mediated barrier topology — every barrier must include
        // host. One CTRL stream per (host,worker). The CTRL-arm
        // host-mediated barrier mediation (TASK-0329 — Done cycle 160
        // via `apply_host_mediation_inject`) lifts the underlying
        // limitation by adding host to every `Sync`'s participant set
        // before backend emit; the DATA arm was lifted as TASK-0327
        // (cycles 148/149) plus TASK-0329.01.02 (cycles 163-164b for
        // in-`Repeat`-body w↔w). The ContractGap below is now
        // defense-in-depth — it should never fire for ACFGs that came
        // through the driver's pipeline; it still bites loud if an
        // upstream change ever removes the mediation pass.
        //
        // NB: the ContractGap message text below intentionally still
        // says "filed as TASK-0175" — test-pinned by mp-tcp-event's
        // `tests/multi_worker_emit.rs::host_excluding_barrier_is_typed_contract_gap`
        // + `tests/host_relay_emit.rs`. The forward-link in the prose
        // ABOVE supersedes; do not propose updating the literal message
        // string here. The backend name prefix routes through
        // `T::BACKEND_NAME` and is independently pinned per backend.
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "{backend} barrier #{bid} participants {parts:?} exclude \
                     the host worker; the one-CTRL-stream-per-(host,worker) \
                     topology requires host as the barrier hub. A \
                     host-excluding barrier needs a worker-to-worker mesh \
                     (filed as TASK-0175).",
                    backend = T::BACKEND_NAME,
                )));
            }
        }

        // Cross-worker push-pair topology: every Push must travel
        // host<->non-host on the (host,worker) star. TASK-0327 (cycle
        // 149) lifts the prior fail-loud rejection of worker-to-worker
        // pairs by routing them via SYNCHRONOUS HOST-RELAY: both src
        // and dst non-host workers use their existing `data_host`
        // reactor socket (peer_idx=0 — see `chan_peer_index`), and
        // HOST runs a relay phase (`render_relay_phase`) that drains
        // `inbound[seq]` from src and re-pushes to
        // `outbound[(seq, dst_peer_idx_at_host)]` toward dst. Filed
        // forward as TASK-0175 for the eventual full-mesh path. The
        // defensive src==host==dst projection check still bites — a
        // Push naming host both ways is malformed regardless of
        // topology.
        for ((d, s), (src, dst)) in &chan_pairs {
            if *src == host_worker && *dst == host_worker {
                return Err(EmitError::ContractGap(format!(
                    "{backend} Push (data={d:?}, seq={s:?}) from {src:?} \
                     to {dst:?} names host as both src and dst — malformed \
                     projection",
                    backend = T::BACKEND_NAME,
                )));
            }
        }

        // TASK-0332 (cycle 151 AC#2): defensive ContractGap for the
        // wait-before-push host-relay deadlock. Cycle-149's
        // synchronous host-relay (`render_relay_phase`) emits a FLAT
        // relay block whose hops `__relay.relay_one(seq, ...)` call
        // `Reactor::wait(seq)`. If any non-host worker's first
        // top-level w2w event is a Wait (rather than a Push), host's
        // wait(seq) blocks for that worker's first Push — which the
        // worker can't reach because it's blocked at its initial
        // Wait. Cycle-150's empirical reproducer (05-stencil/
        // distributed-2d) deadlocked at 32s with this exact shape;
        // cycle 151 converts the runtime deadlock to a codegen-time
        // fail-loud ContractGap forward-linking TASK-0332.
        //
        // **Cycle-162 update (TASK-0329.01.01 slice 1, Option D —
        // RESOLVED for the cycle-150 trigger):** the landed
        // architectural fix is the driver-side `apply_safe_push_reorder`
        // pass (`nucleus-compiler/src/passes/safe_push_reorder.rs`),
        // which hoists hoistable w2w Pushes above preceding w2w Waits
        // within each non-host worker's top-level boundaries. The
        // hoist runs BEFORE this detector (driver pipeline order), so
        // the hazard SHAPE this detector rejects is structurally
        // unreachable for hoistable shapes — 05/distributed-2d is now
        // [[required]] bit-identical on both event backends.
        //
        // This detector STAYS in place as a residual safety net for
        // shapes where slice-1's hoist predicate refuses to move the
        // Push. The detector is dormant on the post-pass matrix and
        // exists for fail-loud hygiene per
        // [[feedback-panic-not-diagnostic-recurring]].
        detect_wait_before_push_hazard::<T>(per_worker, host_worker)?;

        debug_assert_eq!(chan_ids.len(), chan_caps.len());

        // Per-worker overlapping-write accumulator classification
        // (TASK-0343 cycle 189) — mirrors pthreads-sync's and
        // pthreads-async's Plan::build accumulate_waits computation
        // field-for-field.
        let mut accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)> = BTreeSet::new();
        for w in &used_workers {
            let per_worker_set =
                walker::collect_accumulate_waits(&per_worker[w], sidecar, &pair_tiles);
            for (d, s) in per_worker_set {
                accumulate_waits.insert((*w, d, s));
            }
        }

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            host_worker,
            chan_ids,
            chan_caps,
            chan_pairs,
            pair_tiles,
            accumulate_waits,
            barrier_participants,
            _transport: PhantomData,
        })
    }

    pub fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    pub(crate) fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }

    /// Non-host workers in ascending WorkerId order.
    pub(crate) fn non_host_workers(&self) -> Vec<WorkerId> {
        self.used_workers
            .iter()
            .copied()
            .filter(|w| *w != self.host_worker)
            .collect()
    }

    /// Per-worker subset of `chan_ids` — the channels this worker
    /// touches via Push or Wait. Same shape as pthreads-async's
    /// `worker_rings`.
    pub(crate) fn worker_chans(&self, w: WorkerId) -> BTreeSet<ChanId> {
        let mut out: BTreeSet<ChanId> = BTreeSet::new();
        if let Some(evs) = self.per_worker.get(&w) {
            walker::collect_worker_rendezvous(evs, &self.chan_ids, &mut out);
        }
        out
    }

    /// Per-worker peer index map: which DATA-channel peer index in
    /// the worker's Reactor corresponds to each peer WorkerId. Host
    /// has one peer per non-host worker (assigned ascending). A
    /// non-host worker has exactly one peer: the host (index 0).
    pub(crate) fn peer_index_for(&self, worker: WorkerId, peer: WorkerId) -> Option<usize> {
        if worker == self.host_worker {
            self.non_host_workers().iter().position(|w| *w == peer)
        } else if peer == self.host_worker {
            Some(0)
        } else {
            None
        }
    }

    /// Compute the peer index this worker should use when Pushing on
    /// the given chan, or reading its inbound queue.
    ///
    /// For push: this worker is the SRC, the peer is the DST.
    /// For wait: this worker is the DST; the peer is the SRC (we
    /// still need to know it for the demultiplex; though inbound is
    /// keyed by seq alone, the peer dim of outbound is what each
    /// Chan<T> carries — for a wait-only chan we still construct it
    /// with the peer = SRC for symmetry; push() on a wait-only chan
    /// would be a contract bug).
    ///
    /// TASK-0327 (cycle 149): for worker-to-worker pairs (both
    /// non-host), the logical peer (the other worker) is NOT a peer
    /// in this worker's reactor — its reactor has only HOST as a
    /// peer (peer_idx = 0). Route both src's `push` and dst's `wait`
    /// through HOST: replace the logical peer with `host_worker` for
    /// the peer-index lookup. HOST runs a synchronous relay phase
    /// (see `render_relay_phase`) that drains `inbound[seq]` from src
    /// and re-pushes to `outbound[(seq, dst_peer_idx_at_host)]`
    /// toward dst. For dst, peer_idx is irrelevant (wait reads
    /// `inbound[seq]`); for src, peer_idx = 0 routes the push to
    /// host's `data_<src>` socket.
    pub(crate) fn chan_peer_index(
        &self,
        worker: WorkerId,
        chan_key: (DataId, SeqTag),
    ) -> Result<usize, EmitError> {
        let (src, dst) = *self.chan_pairs.get(&chan_key).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "{backend} Plan: missing chan_pairs entry for {chan_key:?}",
                backend = T::BACKEND_NAME,
            ))
        })?;
        let peer = if worker == src {
            dst
        } else if worker == dst {
            src
        } else {
            // This worker touches the chan via neither Push nor Wait —
            // it shouldn't appear in worker_chans then; defensive.
            return Err(EmitError::ContractGap(format!(
                "{backend} Plan: worker {worker:?} touches chan {chan_key:?} \
                 but is neither src ({src:?}) nor dst ({dst:?})",
                backend = T::BACKEND_NAME,
            )));
        };
        // TASK-0327: route worker-to-worker pairs via host.
        let effective_peer = if peer != self.host_worker && worker != self.host_worker {
            self.host_worker
        } else {
            peer
        };
        self.peer_index_for(worker, effective_peer).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "{backend} Plan: worker {worker:?} has no peer index for \
                 peer {peer:?} (effective_peer {effective_peer:?}) on chan \
                 {chan_key:?}",
                backend = T::BACKEND_NAME,
            ))
        })
    }

    /// Pre-init set per worker: cross-worker inputs Waited on + data
    /// written via indexed Fire output and never whole-array. Sorted
    /// by name. Same definition as the sync-TCP backends /
    /// pthreads-async. Returns (pre_init_vec, let_at_wait_set); the
    /// second set is per-worker DataIds with provably-dead pre-init
    /// that get declare-and-assigned at recv site (TASK-0349 cycle
    /// 220).
    #[allow(clippy::type_complexity)]
    pub(crate) fn collect_pre_init(
        &self,
        worker: WorkerId,
    ) -> Result<(Vec<(String, DataId)>, BTreeSet<DataId>), EmitError> {
        let evs = &self.per_worker[&worker];
        let mut waited: BTreeSet<DataId> = BTreeSet::new();
        let mut whole: BTreeSet<DataId> = BTreeSet::new();
        let mut indexed: BTreeSet<DataId> = BTreeSet::new();
        walker::collect_pre_init_sets(evs, &mut waited, &mut whole, &mut indexed);

        // TASK-0349 cycle 220
        let accumulate_data: BTreeSet<DataId> = self
            .accumulate_waits
            .iter()
            .filter_map(|(w, d, _)| if *w == worker { Some(*d) } else { None })
            .collect();
        let let_at_wait = walker::collect_let_at_wait_data(
            evs,
            &self.pair_tiles,
            self.sidecar,
            &accumulate_data,
            &indexed,
        );

        let mut ids: BTreeSet<DataId> = BTreeSet::new();
        for d in &waited {
            if let_at_wait.contains(d) {
                continue;
            }
            ids.insert(*d);
        }
        for d in &indexed {
            if !whole.contains(d) {
                ids.insert(*d);
            }
        }
        let mut out: Vec<(String, DataId)> = Vec::new();
        for d in &ids {
            out.push((self.data_name(*d)?, *d));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok((out, let_at_wait))
    }

    /// Largest single cross-worker payload in bytes — drives SO_*BUF
    /// sizing in run.sh. Same calculation as the sync-TCP backends.
    ///
    /// TASK-0453.22: routed through the ONE `WireShape` extent per
    /// `(data, seq)` edge so a narrowed edge (a contiguous recv-basis
    /// band) sizes the buffer for the BAND it actually transmits, not the
    /// whole array. The channel keys are `(DataId, SeqTag)`, so each edge
    /// derives its own `WireShape` and the max is the true largest
    /// on-wire payload. A datum with no `ResolvedType` is skipped
    /// (contributes 0) — matching the pre-flip `ok_or_else` guard's
    /// intent (it cannot size a typeless symbol; the paired Wait render
    /// fails loud on the same gap if such an edge ever ships).
    pub(crate) fn max_payload_bytes(&self) -> Result<usize, EmitError> {
        let mut max = 0usize;
        for (d, s) in self.chan_ids.keys() {
            let Some(ty) = self.sidecar.data_type(*d) else {
                continue;
            };
            let wire = walker::WireShape::derive(self.sidecar, &self.pair_tiles, *d, *s)?;
            let bytes = wire.extent_bytes(encode::scalar_width(&ty.scalar));
            max = max.max(bytes);
        }
        Ok(max)
    }

    /// SyncTags this worker participates in, ascending order.
    pub(crate) fn barriers_used_by(&self, w: WorkerId) -> Vec<SyncTag> {
        let mut out: Vec<SyncTag> = self
            .barrier_participants
            .iter()
            .filter(|(_, parts)| parts.contains(&w))
            .map(|(id, _)| *id)
            .collect();
        out.sort_unstable();
        out
    }
}
