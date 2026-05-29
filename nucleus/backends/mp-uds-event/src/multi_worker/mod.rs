//! mp-uds-event multi-worker codegen (TASK-0042.05 / Stage 3 of TASK-0042.02).
//!
//! # Status
//!
//! Lands the multi-worker arm of [`super::emit`]: emit one
//! `src/bin/<worker>.rs` per used worker, each containing the mio
//! reactor wired to per-(seq, peer) bounded outbound queues plus
//! per-seq inbound queues. The shared
//! [`backend_common::multi_worker_walker::render_worker_events`]
//! drives the per-event walk with `rendezvous_prefix = "chan"`, so
//! Push/Wait sites lower to `chan_<rid>.push(name.clone())` /
//! `chan_<rid>.wait()` — the same surface pthreads-async uses for
//! `ring_<id>` (the prefix is THE one knob that differs across
//! backends).
//!
//! # Design notes
//!
//! - **Two sockets per (host, worker) pair**: DATA (mio-managed,
//!   non-blocking) for Push/Wait; CTRL (`std::os::unix::net::UnixStream`,
//!   blocking) for barriers via `wire::barrier_cross`. Mirrors
//!   mp-tcp-event's DATA+CTRL split — both backends need it for the
//!   same reason: producer/consumer barrier-vs-data ordering can
//!   differ on each side of a `(host,worker)` pair, and a single FIFO
//!   would corrupt frame demuxing. mp-uds-event's data-channel
//!   demultiplex happens by `seq` instead of arrival order, but
//!   barriers still need their own ordered channel.
//! - **Path-as-rendezvous handshake** (cycle 197): host binds a
//!   `UnixListener` per non-host worker at the well-known paths
//!   `$NUC_RENDEZVOUS_DIR/<wname>.data.sock` and `<wname>.ctrl.sock`.
//!   Non-host worker `connect_retry`s the same paths directly (no
//!   port file, no port-binding race — the path itself IS the
//!   rendezvous, which is structurally SIMPLER than mp-tcp-event's
//!   TCP-port rendezvous). The shared multi_binary run.sh template's
//!   `$here`-rooted rendezvous dir would bust the UDS sun_path 104
//!   byte cap on the e2e harness's deep scratch hierarchy; mp-uds-event
//!   post-processes run.sh to use a `/tmp`-rooted `mktemp -d -t
//!   nuc-uds-XXXXXXXX` instead (see `render_run_sh` at the bottom of
//!   this file + memory `project-uds-path-cap-rendezvous`).
//! - **Reactor**: see `runtime_src.rs`. One Reactor per worker
//!   process. The DATA socket per peer is set non-blocking and
//!   registered with mio; readable readiness drains frames into
//!   `Reactor::inbound[seq]`; writable readiness drains
//!   `Reactor::outbound[(seq, peer)]`. `Chan<T>::push(v)` enqueues
//!   (blocks on cap); `Chan<T>::wait()` blocks while inbound[seq] is
//!   empty.
//! - **Sizing**: per-pair cap from
//!   `sidecar.transfer_buffer_for_seq[seq]` (TASK-0233). A missing
//!   entry is a [`EmitError::ContractGap`] — same precedent as
//!   pthreads-async's `Plan::build`.
//! - **Barrier identity**: contract-carried
//!   [`nucleus_compiler::event::SyncTag`] (TASK-0172). Same shape as
//!   mp-tcp-bufsync.
//! - **Host-excluding barriers**: same transport limitation as
//!   mp-tcp-bufsync — one CTRL stream per `(host, worker)` pair, so a
//!   barrier whose participants exclude host cannot be lowered
//!   AT THE BACKEND directly. The original TASK-0175 worker-to-worker
//!   combined filing was split into a DATA arm and a CTRL arm
//!   cycles 148/149. The DATA arm was lifted in two phases — top-level
//!   w↔w `Push`/`Wait` via host-relay cycle 149 (TASK-0327), and
//!   in-`Repeat`-body w↔w `Push`/`Wait` via the
//!   `apply_host_data_relay_inject` ACFG pass cycles 163-164b
//!   (TASK-0329.01.02). The CTRL arm — host-mediated barrier
//!   mediation for host-excluding barriers — was lifted cycle 160
//!   (TASK-0329, marked Done) via the `apply_host_mediation_inject`
//!   compiler pass at `nucleus_compiler::passes::host_mediation_inject`,
//!   dispatched from the driver before backend emit. The pass adds
//!   `host` as a participant to every `Sync` whose participant set
//!   excludes it, turning each host-excluding barrier into an N+1-party
//!   star through host that the existing barrier-shim emitter handles
//!   transparently. The backend's `ContractGap` rejection at
//!   `Plan::build` is now defense-in-depth — it should never fire for
//!   ACFGs that came through the driver's pipeline. (Wire-message text
//!   still cites TASK-0175. Defense-in-depth only; not test-pinned at
//!   the integration layer in mp-uds-event (the e2e gate is the
//!   behavioural witness, via the driver's cycle-197-widened
//!   `apply_host_mediation_inject`). The sibling mp-tcp-event has a
//!   `tests/multi_worker_emit.rs::host_excluding_barrier_is_typed_contract_gap`
//!   pin; porting it to mp-uds-event is left as a defensive-coverage
//!   follow-up if the upstream mediation gate is ever loosened.)
//! - **Worker-to-worker `Push`/`Wait`** (TASK-0327, cycle 149):
//!   DATA-side w↔w lifted via HOST-RELAY. Src's `chan_<rid>.push`
//!   uses peer_idx=0 (host); HOST's `main()` runs a synchronous
//!   relay phase (`Plan::render_relay_phase`) emitted just BEFORE
//!   the LAST top-level `Event::Sync` that calls
//!   `Reactor::relay_one(seq, dst_peer, cap)` per hop — drains
//!   `inbound[seq]` from src and re-pushes to
//!   `outbound[(seq, dst_peer_idx_at_host)]` toward dst. Dst's
//!   `chan_<rid>.wait` reads its own `inbound[seq]` (driven by
//!   host's forwarded frames on its `data_host` socket).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::elect_host_from_worker_names;
use backend_common::multi_worker_walker::{self as walker, RendezvousId};
use backend_common::project_skeleton::multi_binary::render_run_sh_multi;

use crate::{EmitError, NameTables};

mod encode;
mod relay;
mod walkers;
mod worker_program;

use walkers::{collect_push_pairs, detect_wait_before_push_hazard};

/// Stable identifier for one mp-uds-event channel — the runtime
/// `chan_<id>` variable that wraps `(DataId, SeqTag)`'s reactor route.
/// `usize` alias for [`RendezvousId`] (the shared walker key type).
type ChanId = RendezvousId;

/// Per-worker codegen Plan: every fact needed to emit one
/// `src/bin/<wname>.rs`. Field set mirrors `pthreads-async`'s Plan
/// modulo the per-pair PEER index needed for mp-tcp's per-(src,dst)
/// outbound queue and the host-mediated barrier topology check.
pub(crate) struct Plan<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    pub(crate) used_workers: Vec<WorkerId>,
    host_worker: WorkerId,
    /// Cross-worker Push/Wait pairs `(DataId, SeqTag) -> chan index`.
    chan_ids: BTreeMap<(DataId, SeqTag), ChanId>,
    /// Per-pair channel capacity from `transfer DATA : buffer=N`
    /// (TASK-0233).
    chan_caps: BTreeMap<(DataId, SeqTag), u64>,
    /// Per-pair (src, dst) workers. The reactor's outbound queue is
    /// keyed on `(seq, peer_idx)`; the producer side puts (seq, dst)
    /// and the consumer side reads inbound[seq] (independent of src).
    chan_pairs: BTreeMap<(DataId, SeqTag), (WorkerId, WorkerId)>,
    /// Per-pair iteration tile (TASK-0117 host-side gather).
    pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343 cycle 189). Mirrors pthreads-sync's
    /// and pthreads-async's `multi_worker::Plan::accumulate_waits`
    /// field; populated by `walker::collect_accumulate_waits` per
    /// worker and unioned with the WorkerId. Consumed by the shared
    /// event walker via `WalkerCtx::accumulate_waits` to switch
    /// Event::Wait emit from whole-array overwrite assign to
    /// element-wise `wrapping_add` accumulate. Empty for every cell
    /// without an overlapping-write fan-in.
    accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
    /// SyncTag -> participants. Same shape as mp-tcp-bufsync's.
    barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>>,
}

impl<'a> Plan<'a> {
    /// Build the Plan; returns `EmitError::ContractGap` for any
    /// invariant violation reachable from valid input (cap missing,
    /// host-excluding barrier, malformed projection).
    pub(crate) fn build(
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
        // host-relay (see `Plan::render_relay_phase` +
        // `Reactor::relay_one`), so Branch D now bites only the
        // genuinely-malformed `src == dst == host` projection. This
        // order is load-bearing for the negative-path test fixtures
        // in this file's `#[cfg(test)] mod tests` and in
        // `tests/multi_worker_emit.rs` — each test must NOT trip any
        // EARLIER check to exercise its target branch. A new check
        // inserted between branches may silently invalidate a bypass-
        // fixture; if you reorder/add, update the fixtures together.
        if used_workers.len() < 2 {
            return Err(EmitError::ContractGap(format!(
                "mp-uds-event Plan::build requires used_workers.len() >= 2; \
                 got {n}. Single-worker is handled by emit()'s single-worker arm.",
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
                EmitError::ContractGap(
                    "mp-uds-event Plan: used_workers reachable to host \
                     election but empty — invariant len() >= 2 violated"
                        .to_string(),
                )
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
                    "mp-uds-event Plan: (data={:?}, seq={:?}) has a Wait \
                     but no matching Push — malformed projection",
                    k.0, k.1
                )));
            }
        }

        // Deterministic chan_id assignment: ascending by (DataId, SeqTag).
        let chan_ids: BTreeMap<(DataId, SeqTag), ChanId> = pair_tiles
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();

        // Capacity per pair (TASK-0233 sidecar lookup).
        let mut chan_caps: BTreeMap<(DataId, SeqTag), u64> = BTreeMap::new();
        for (data, seq) in pair_tiles.keys() {
            let cap = sidecar
                .transfer_buffer_for_seq
                .get(seq)
                .copied()
                .ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "mp-uds-event Plan: (data={data:?}, seq={seq:?}) Push/Wait \
                         pair has no entry in sidecar.transfer_buffer_for_seq. \
                         Either build_sidecar's walker missed an Xfer placeholder \
                         (TASK-0233 regression), or the EventList was projected \
                         without running transfer_inject first."
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
        // host. Mirrors mp-tcp-bufsync's check (one CTRL stream per
        // (host,worker)). The CTRL-arm host-mediated barrier
        // mediation (TASK-0329 — Done cycle 160 via
        // `apply_host_mediation_inject`) lifts the underlying
        // limitation by adding host to every `Sync`'s participant set
        // before backend emit; the DATA arm was lifted as TASK-0327
        // (cycles 148/149) plus TASK-0329.01.02 (cycles 163-164b for
        // in-`Repeat`-body w↔w). The ContractGap below is now
        // defense-in-depth — it should never fire for ACFGs that came
        // through the driver's pipeline; it still bites loud if an
        // upstream change ever removes the mediation pass.
        //
        // NB: the ContractGap message text below intentionally still
        // says "filed as TASK-0175". Defense-in-depth only on
        // mp-uds-event — the e2e gate is the integration-layer
        // witness for the cycle-197-widened
        // `apply_host_mediation_inject` driver pass that prevents
        // ACFGs from reaching this branch in normal compilation.
        // The forward-link in the prose ABOVE supersedes the message
        // literal; do not propose updating the literal here.
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "mp-uds-event barrier #{bid} participants {parts:?} exclude \
                     the host worker; the one-CTRL-stream-per-(host,worker) \
                     topology requires host as the barrier hub. A \
                     host-excluding barrier needs a worker-to-worker mesh \
                     (filed as TASK-0175)."
                )));
            }
        }

        // Cross-worker push-pair topology: every Push must travel
        // host<->non-host on the (host,worker) star. TASK-0327 (cycle
        // 149) lifts the prior fail-loud rejection of worker-to-worker
        // pairs by routing them via SYNCHRONOUS HOST-RELAY: both src
        // and dst non-host workers use their existing `data_host`
        // reactor socket (peer_idx=0 — see `chan_peer_index`), and
        // HOST runs a relay phase (`Plan::render_relay_phase`) that
        // drains `inbound[seq]` from src and re-pushes to
        // `outbound[(seq, dst_peer_idx_at_host)]` toward dst. Mirrors
        // mp-tcp-bufsync's cycle-148 lift. Filed forward as TASK-0175
        // for the eventual full-mesh path. The defensive
        // src==host==dst projection check still bites — a Push naming
        // host both ways is malformed regardless of topology.
        for ((d, s), (src, dst)) in &chan_pairs {
            if *src == host_worker && *dst == host_worker {
                return Err(EmitError::ContractGap(format!(
                    "mp-uds-event Push (data={d:?}, seq={s:?}) from {src:?} \
                     to {dst:?} names host as both src and dst — malformed \
                     projection"
                )));
            }
        }

        // TASK-0332 (cycle 151 AC#2): defensive ContractGap for the
        // wait-before-push host-relay deadlock. Cycle-149's
        // synchronous host-relay (`Plan::render_relay_phase`) emits
        // a FLAT relay block whose hops `__relay.relay_one(seq, ...)`
        // call `Reactor::wait(seq)`. If any non-host worker's first
        // top-level w2w event is a Wait (rather than a Push), host's
        // wait(seq) blocks for that worker's first Push — which the
        // worker can't reach because it's blocked at its initial
        // Wait. Cycle-150's empirical reproducer (05-stencil/
        // distributed-2d × mp-uds-event) deadlocked at 32s with this
        // exact shape; cycle 151 converts the runtime deadlock to a
        // codegen-time fail-loud ContractGap forward-linking
        // TASK-0332.
        //
        // **Cycle-162 update (TASK-0329.01.01 slice 1, Option D —
        // RESOLVED for the cycle-150 trigger):** the landed
        // architectural fix is the driver-side `apply_safe_push_reorder`
        // pass (`nucleus-compiler/src/passes/safe_push_reorder.rs`),
        // which hoists hoistable w2w Pushes above preceding w2w Waits
        // within each non-host worker's top-level boundaries. The
        // hoist runs BEFORE this detector (driver pipeline order;
        // see `driver/src/main.rs` near `apply_safe_push_reorder`),
        // so the hazard SHAPE this detector rejects is structurally
        // unreachable for hoistable shapes — 05/distributed-2d ×
        // mp-uds-event is now [[required]] bit-identical.
        //
        // This detector STAYS in place as a residual safety net for
        // shapes where slice-1's hoist predicate refuses to move the
        // Push: (a) `inside_loop = true` writes by Fire / Wait /
        // nested Loop that taint the data and make the subsequent
        // w2w Push not-hoistable, or (b) tile-overlap dependencies
        // between a preceding w2w Wait and the candidate Push on a
        // shared axis. Neither residual fires on any in-tree schedule
        // today; the detector is dormant on the post-pass matrix and
        // exists for fail-loud hygiene per
        // [[feedback-panic-not-diagnostic-recurring]].
        //
        // Conservative-but-sound check: rejects every schedule whose
        // first top-level w2w event for ANY non-host worker is a
        // Wait. May false-positive on hypothetical "wait-only"
        // workers (those with w2w Waits but no w2w Pushes) where the
        // deadlock cycle would not actually close.
        detect_wait_before_push_hazard(per_worker, host_worker)?;

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
        })
    }

    pub(crate) fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }

    /// Non-host workers in ascending WorkerId order.
    fn non_host_workers(&self) -> Vec<WorkerId> {
        self.used_workers
            .iter()
            .copied()
            .filter(|w| *w != self.host_worker)
            .collect()
    }

    /// Per-worker subset of `chan_ids` — the channels this worker
    /// touches via Push or Wait. Same shape as pthreads-async's
    /// `worker_rings`.
    fn worker_chans(&self, w: WorkerId) -> BTreeSet<ChanId> {
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
    fn peer_index_for(&self, worker: WorkerId, peer: WorkerId) -> Option<usize> {
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
    /// (see `Plan::render_relay_phase`) that drains `inbound[seq]`
    /// from src and re-pushes to `outbound[(seq, dst_peer_idx_at_host)]`
    /// toward dst. For dst, peer_idx is irrelevant (wait reads
    /// `inbound[seq]`); for src, peer_idx = 0 routes the push to
    /// host's `data_<src>` socket. Mirrors mp-tcp-bufsync cycle 148.
    fn chan_peer_index(
        &self,
        worker: WorkerId,
        chan_key: (DataId, SeqTag),
    ) -> Result<usize, EmitError> {
        let (src, dst) = *self.chan_pairs.get(&chan_key).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "mp-uds-event Plan: missing chan_pairs entry for {chan_key:?}"
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
                "mp-uds-event Plan: worker {worker:?} touches chan {chan_key:?} \
                 but is neither src ({src:?}) nor dst ({dst:?})"
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
                "mp-uds-event Plan: worker {worker:?} has no peer index for \
                 peer {peer:?} (effective_peer {effective_peer:?}) on chan \
                 {chan_key:?}"
            ))
        })
    }

    /// Pre-init set per worker: cross-worker inputs Waited on + data
    /// written via indexed Fire output and never whole-array. Sorted
    /// by name. Same definition as mp-tcp-bufsync / pthreads-async.
    /// Returns (pre_init_vec, let_at_wait_set); the second set is
    /// per-worker DataIds with provably-dead pre-init that get
    /// declare-and-assigned at recv site (TASK-0349 cycle 220).
    #[allow(clippy::type_complexity)]
    fn collect_pre_init(
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
    /// sizing in run.sh. Same calculation as mp-tcp-bufsync.
    fn max_payload_bytes(&self) -> Result<usize, EmitError> {
        let mut max = 0usize;
        for (d, _) in self.chan_ids.keys() {
            let ty = self.sidecar.data_type(*d).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "cross-worker data {d:?} has no ResolvedType in sidecar"
                ))
            })?;
            let elems: usize = if ty.is_scalar() {
                1
            } else {
                ty.dims.iter().copied().product()
            };
            let w = encode::scalar_width(&ty.scalar);
            max = max.max(elems * w);
        }
        Ok(max)
    }

    /// SyncTags this worker participates in, ascending order.
    fn barriers_used_by(&self, w: WorkerId) -> Vec<SyncTag> {
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

/// Per-backend SO_BUF commentary block interpolated by the shared
/// [`backend_common::project_skeleton::multi_binary::render_run_sh_multi`]
/// before the `export NUC_SO_BUF=...` line. mp-uds-event additionally
/// buffers `buffer=N` frames per (seq, peer) in the APPLICATION ring,
/// so the SO_*BUF sizing here is the WIRE frame size, not the ring
/// depth — a different rationale than mp-tcp-bufsync's sync-one-msg
/// sizing, hence the per-backend comment.
const SO_BUF_COMMENT_EVENT: &str = "# Socket buffer requirement from the schedule's per-channel\n\
     # buffer needs (largest single transfer payload). Same shape\n\
     # as mp-tcp-bufsync — mp-uds-event additionally buffers up to\n\
     # `transfer DATA : buffer=N` frames per (seq, peer) IN THE\n\
     # APPLICATION ring, so the SO_*BUF sizing here is the WIRE\n\
     # frame size, not the ring depth.\n";

/// Multi-process run.sh: delegate to the shared
/// [`backend_common::project_skeleton::multi_binary::render_run_sh_multi`]
/// (lifted in TASK-0257 cycle 112), supplying the host-first worker
/// ordering + mp-uds-event-specific SO_BUF commentary, then SWAP the
/// shared template's `$here/.nuc-rendezvous-$$` rendezvous-dir line
/// for a `/tmp`-rooted alternative because UDS sun_path is capped at
/// 104 bytes (musl/macOS) — see cycle-197 step 5 AC#9 finding.
///
/// The shared template uses `$here/.nuc-rendezvous-$$` (mp-tcp-* needs
/// only the rendezvous-FILE PATH, not a per-worker socket path, so
/// the 4096-byte filesystem PATH_MAX is the binding cap there). For
/// UDS the rendezvous DIR + socket-filename together must fit in 104
/// bytes; with realistic worker names + the e2e harness's deep scratch
/// hierarchy (`target/e2e-matrix/run-NNN-...../<example>__<sched>__<backend>/`)
/// the `$here`-rooted dir is ~120-180 bytes and busts the cap. Using
/// `/tmp/nuc-uds-$$-<run-tag>/` keeps the dir below ~25 bytes; with
/// worker names ≤ ~60 bytes the total fits well under 104. The trap
/// still tidies the dir on EXIT.
pub(crate) fn render_run_sh(plan: &Plan<'_>) -> Result<String, EmitError> {
    let bufsz = plan.max_payload_bytes()?.max(65536);
    let host_name = plan.worker_name(plan.host_worker);
    let non_host_names: Vec<String> = plan
        .non_host_workers()
        .iter()
        .map(|w| plan.worker_name(*w))
        .collect();
    let shared = render_run_sh_multi(&host_name, &non_host_names, bufsz, SO_BUF_COMMENT_EVENT);

    // SWAP the rendezvous-dir line for a /tmp-rooted UDS-safe path.
    // The shared template's exact block is 4 lines (see
    // `multi_binary::render_run_sh_multi`). We replace the whole block
    // atomically so a future change to the shared template surfaces
    // as a swap miss + falls back to the original (which would then
    // bust the UDS path cap loud at runtime — fail-loud preserved).
    let needle = "NUC_RENDEZVOUS_DIR=\"$here/.nuc-rendezvous-$$\"\n\
                  mkdir -p \"$NUC_RENDEZVOUS_DIR\"\n\
                  trap 'rm -rf \"$NUC_RENDEZVOUS_DIR\"' EXIT\n\
                  export NUC_RENDEZVOUS_DIR";
    let replacement = "# UDS-specific: rendezvous dir under /tmp because\n\
                       # mp-uds-event's UDS sockets must fit in the\n\
                       # 104-byte sun_path cap (musl/macOS limit); the\n\
                       # e2e harness's deep scratch hierarchy + worker\n\
                       # name + .sock suffix easily busts the cap if\n\
                       # we use $here. mktemp -d picks an unused name\n\
                       # under /tmp, guaranteed-fresh per run.\n\
                       NUC_RENDEZVOUS_DIR=\"$(mktemp -d -t nuc-uds-XXXXXXXX)\"\n\
                       trap 'rm -rf \"$NUC_RENDEZVOUS_DIR\"' EXIT\n\
                       export NUC_RENDEZVOUS_DIR";
    // Multi-line literal shell-script block (not a rendered Rust expression);
    // sibling-substring risk is zero. Post-replace fail-loud sentinel below
    // catches needle-not-found. Lift target: transport-parametric
    // render_run_sh_multi parameter (follow-up TASK-0044.03.02 filed cycle 197b).
    let out = shared.replace(needle, replacement); // ALLOW textual replace: multi-line literal bash block (see comment above; needle is a fixed shared-template substring)
    if !out.contains("nuc-uds-XXXXXXXX") {
        return Err(EmitError::ContractGap(format!(
            "mp-uds-event render_run_sh: shared template's rendezvous-dir \
             block changed shape — the cycle-197 step-5 UDS-path swap \
             missed. Expected the literal needle:\n{needle}\n\nbut the \
             shared template emitted:\n{shared}"
        )));
    }
    Ok(out)
}

// --------------------------------------------------------------------
// TASK-0255 — Branch A (used_workers.len() < 2) unit test.
//
// `Plan::build` is `pub(crate)`. Branch A is unreachable from the
// public `emit()` because the lib.rs:290 dispatch routes
// `used_workers.len() <= 1` to the single-worker arm BEFORE
// Plan::build is ever called. The only way to exercise Branch A is
// to call Plan::build directly from inside this crate — hence this
// in-module test.
//
// Branches B/C/D have e2e coverage on 2+ workers (the integration
// witness for them lives in the e2e harness, not a dedicated unit
// test in mp-uds-event/tests/). Porting the
// `host_excluding_barrier_is_typed_contract_gap` pattern from
// mp-tcp-event/tests/multi_worker_emit.rs is left as a
// defensive-coverage follow-up if the upstream driver mediation
// gates are ever loosened.
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NameTables;
    use nucleus_compiler::event::{Event, SyncKind, SyncTag, WorkerId};
    use nucleus_compiler::sidecar::NameSidecar;
    use std::collections::BTreeMap;

    /// Branch A (multi_worker.rs:114-119) — `Plan::build` must reject
    /// single-worker input (used_workers.len() < 2) with a typed
    /// ContractGap naming the >= 2 invariant. This branch is the
    /// gatekeeper that catches a regression where the lib.rs dispatch
    /// arm accidentally routed single-worker input to `Plan::build`
    /// instead of the single-worker emitter.
    ///
    /// Reachability: only from inside the crate. `emit()` routes
    /// `<=1` worker input to `render_single_worker_main` BEFORE
    /// `Plan::build` is called (lib.rs:290).
    #[test]
    fn single_worker_input_is_typed_contract_gap() {
        let w_host = WorkerId(0);

        // ONE non-empty worker. used_workers will be `[w_host]`,
        // len 1, < 2 — Branch A fires.
        let host_marker = Event::Sync {
            participants: [w_host].into_iter().collect(),
            kind: SyncKind::Barrier,
            sync: SyncTag(0),
        };
        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        per_worker.insert(w_host, vec![host_marker]);

        let mut names = NameTables::default();
        names.worker.insert(w_host, "host".to_string());
        let sidecar = NameSidecar::default();

        let r = Plan::build(&per_worker, &names, &sidecar);
        match r {
            Err(EmitError::ContractGap(msg)) => {
                assert!(
                    msg.contains("used_workers.len() >= 2"),
                    "ContractGap must name the >= 2 invariant: {msg}"
                );
                assert!(
                    msg.contains("Single-worker is handled by emit()"),
                    "ContractGap must point to the single-worker arm as the correct route: {msg}"
                );
            }
            Err(other) => {
                panic!("expected ContractGap on single-worker Plan::build; got Err({other:?})")
            }
            Ok(_) => panic!("expected ContractGap on single-worker Plan::build; got Ok(Plan)"),
        }
    }

    /// Edge: ZERO non-empty workers (every Vec is empty). Still
    /// triggers Branch A — used_workers.len() == 0 < 2. The
    /// message's `n=` placeholder must reflect that.
    #[test]
    fn zero_worker_input_is_typed_contract_gap() {
        let w_host = WorkerId(0);
        let w1 = WorkerId(1);
        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        // Both entries present but EMPTY — used_workers filter drops
        // empties, leaving 0.
        per_worker.insert(w_host, vec![]);
        per_worker.insert(w1, vec![]);

        let names = NameTables::default();
        let sidecar = NameSidecar::default();

        let r = Plan::build(&per_worker, &names, &sidecar);
        match r {
            Err(EmitError::ContractGap(msg)) => {
                assert!(
                    msg.contains("used_workers.len() >= 2"),
                    "ContractGap must name the >= 2 invariant: {msg}"
                );
                assert!(
                    msg.contains("got 0"),
                    "ContractGap must report the actual count (0 here): {msg}"
                );
            }
            Err(other) => {
                panic!("expected ContractGap on zero-worker Plan::build; got Err({other:?})")
            }
            Ok(_) => panic!("expected ContractGap on zero-worker Plan::build; got Ok(Plan)"),
        }
    }
}
