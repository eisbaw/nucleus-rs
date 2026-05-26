//! mp-tcp-event multi-worker codegen (TASK-0042.05 / Stage 3 of TASK-0042.02).
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
//!   non-blocking) for Push/Wait; CTRL (`std::net::TcpStream`,
//!   blocking) for barriers via `wire::barrier_cross`. Mirrors
//!   mp-tcp-bufsync's DATA+CTRL split — both backends need it for the
//!   same reason: producer/consumer barrier-vs-data ordering can
//!   differ on each side of a `(host,worker)` pair, and a single FIFO
//!   would corrupt frame demuxing. mp-tcp-event's data-channel
//!   demultiplex happens by `seq` instead of arrival order, but
//!   barriers still need their own ordered channel.
//! - **Rendezvous-file handshake** (TASK-0176): host binds
//!   `127.0.0.1:0` ITSELF per non-host worker and atomically publishes
//!   the OS-assigned port to `$NUC_RENDEZVOUS_DIR/<wname>.port`
//!   (tmp + rename). Non-host worker polls the file (600 × 10 ms =
//!   6 s) then `connect_retry`s. NEVER use the deleted
//!   `__nuc_pick_port` helper — its close-then-rebind shape opened a
//!   TOCTOU window that TASK-0176 closed.
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
//!   barrier whose participants exclude host cannot be lowered.
//!   Fail-loud with a typed `ContractGap` forward-linking TASK-0175
//!   (filed forward as TASK-0329 for host-mediated barrier
//!   mediation, analogous to cycle-148/149's data lift).
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
use std::fmt::Write as _;

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static,
};
use backend_common::multi_worker_walker::{self as walker, RendezvousId, WalkerCtx};
use backend_common::render::{render_array_init_for, rust_type_of};

use crate::{EmitError, NameTables};

/// Stable identifier for one mp-tcp-event channel — the runtime
/// `chan_<id>` variable that wraps `(DataId, SeqTag)`'s reactor route.
/// `usize` alias for [`RendezvousId`] (the shared walker key type).
pub(crate) type ChanId = RendezvousId;

/// Per-worker codegen Plan: every fact needed to emit one
/// `src/bin/<wname>.rs`. Field set mirrors `pthreads-async`'s Plan
/// modulo the per-pair PEER index needed for mp-tcp's per-(src,dst)
/// outbound queue and the host-mediated barrier topology check.
pub(crate) struct Plan<'a> {
    pub(crate) per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    pub(crate) names: &'a NameTables,
    pub(crate) sidecar: &'a NameSidecar,
    pub(crate) used_workers: Vec<WorkerId>,
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
    /// SyncTag -> participants. Same shape as mp-tcp-bufsync's.
    pub(crate) barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>>,
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
                "mp-tcp-event Plan::build requires used_workers.len() >= 2; \
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
        let host_worker = backend_common::elect_host_from_worker_names(&names.worker, &used_workers)
            .ok_or_else(|| {
                EmitError::ContractGap(
                    "mp-tcp-event Plan: used_workers reachable to host \
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
                    "mp-tcp-event Plan: (data={:?}, seq={:?}) has a Wait \
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
                        "mp-tcp-event Plan: (data={data:?}, seq={seq:?}) Push/Wait \
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
        // (host,worker)). A host-excluding barrier needs host-mediated
        // barrier mediation (TASK-0329 — CTRL arm of the cycle-148/149
        // split of the original TASK-0175 combined filing; the DATA
        // arm was lifted as TASK-0327 in cycles 148/149). Fail-loud
        // rather than mis-route.
        //
        // NB: the ContractGap message text below intentionally still
        // says "filed as TASK-0175" — test-pinned by
        // `tests/multi_worker_emit.rs::host_excluding_barrier_is_typed_contract_gap`
        // and `tests/host_relay_emit.rs`. The forward-link in the
        // prose ABOVE supersedes; do not propose updating the literal
        // message string here.
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "mp-tcp-event barrier #{bid} participants {parts:?} exclude \
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
                    "mp-tcp-event Push (data={d:?}, seq={s:?}) from {src:?} \
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
        // distributed-2d × mp-tcp-event) deadlocked at 32s with this
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
        // mp-tcp-event is now [[required]] bit-identical.
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
                "mp-tcp-event Plan: missing chan_pairs entry for {chan_key:?}"
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
                "mp-tcp-event Plan: worker {worker:?} touches chan {chan_key:?} \
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
                "mp-tcp-event Plan: worker {worker:?} has no peer index for \
                 peer {peer:?} (effective_peer {effective_peer:?}) on chan \
                 {chan_key:?}"
            ))
        })
    }

    /// Pre-init set per worker: cross-worker inputs Waited on + data
    /// written via indexed Fire output and never whole-array. Sorted
    /// by name. Same definition as mp-tcp-bufsync / pthreads-async.
    fn collect_pre_init(&self, worker: WorkerId) -> Result<Vec<(String, DataId)>, EmitError> {
        let evs = &self.per_worker[&worker];
        let mut waited: BTreeSet<DataId> = BTreeSet::new();
        let mut whole: BTreeSet<DataId> = BTreeSet::new();
        let mut indexed: BTreeSet<DataId> = BTreeSet::new();
        walker::collect_pre_init_sets(evs, &mut waited, &mut whole, &mut indexed);
        let mut ids: BTreeSet<DataId> = BTreeSet::new();
        for d in &waited {
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
        Ok(out)
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
            let w = scalar_width(&ty.scalar);
            max = max.max(elems * w);
        }
        Ok(max)
    }

    /// Render one worker's full `src/bin/<wname>.rs`.
    pub(crate) fn render_worker_program(&self, worker: WorkerId) -> Result<String, EmitError> {
        let mut out = String::new();
        let wname = self.worker_name(worker);
        let is_host = worker == self.host_worker;

        // ---- File header + modules. ----
        writeln!(
            out,
            "//! Generated by the mp-tcp-event backend (TASK-0042.05, \
             multi-process). Worker `{wname}`{}.",
            if is_host {
                " [host/server]"
            } else {
                " [client]"
            }
        )
        .ok();
        writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
        writeln!(out).ok();
        writeln!(out, "#[path = \"../kernels.rs\"]").ok();
        writeln!(out, "#[allow(dead_code)]").ok();
        writeln!(out, "mod kernels;").ok();
        writeln!(out, "#[path = \"../wire.rs\"]").ok();
        writeln!(out, "#[allow(dead_code)]").ok();
        writeln!(out, "mod wire;").ok();
        writeln!(out, "#[path = \"../runtime.rs\"]").ok();
        writeln!(out, "#[allow(dead_code)]").ok();
        writeln!(out, "mod runtime;").ok();
        writeln!(out).ok();

        // ---- Imports (role-specific so warning-clean). ----
        writeln!(out, "use std::fs;").ok();
        writeln!(out, "use std::path::PathBuf;").ok();
        if is_host {
            writeln!(out, "use std::net::TcpListener;").ok();
            writeln!(out, "use std::io::Write as _;").ok();
        } else {
            writeln!(out, "use std::net::TcpStream as StdTcpStream;").ok();
            writeln!(out, "use std::time::Duration;").ok();
        }
        // Host-or-not — both need these in main() for the reactor +
        // chan setup.
        writeln!(out, "use std::cell::RefCell;").ok();
        writeln!(out, "use std::rc::Rc;").ok();
        writeln!(out).ok();

        // ---- Count check_frame substrate (file scope). ----
        let count_frames = collect_count_check_frames(&self.per_worker[&worker]);
        if !count_frames.is_empty() {
            emit_count_reporter_struct(&mut out);
            for cf in &count_frames {
                emit_count_static(&mut out, &cf.ident);
            }
            writeln!(out).ok();
        }

        // ---- main(). ----
        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, \
             unused_assignments, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        // Per-Count-loop Drop guard local.
        for cf in &count_frames {
            emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
        }
        if !count_frames.is_empty() {
            writeln!(out).ok();
        }

        // ---- Rendezvous-file handshake + DATA/CTRL socket setup. ----
        self.emit_handshake(&mut out, worker, is_host)?;
        writeln!(out).ok();

        // ---- Build the Reactor (mio-managed DATA sockets) + chan
        //      instances. ----
        self.emit_reactor_and_chans(&mut out, worker)?;
        writeln!(out).ok();

        // ---- Pre-init locals (Wait targets + indexed Fire writes). ----
        let pre_init = self.collect_pre_init(worker)?;
        for (name, did) in &pre_init {
            let ty = self.sidecar.data_type(*did).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "pre-init data `{name}` ({did:?}) has no ResolvedType in sidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            let init = render_array_init_for(ty);
            writeln!(out, "    let mut {name}: {rty} = {init};").ok();
        }
        if !pre_init.is_empty() {
            writeln!(out).ok();
        }

        // ---- Drive the shared walker. ----
        let walker_ctx = WalkerCtx {
            names: self.names,
            sidecar: self.sidecar,
            rendezvous_prefix: "chan",
            rendezvous_ids: &self.chan_ids,
            pair_tiles: &self.pair_tiles,
        };
        // The walker emits chan_<rid>.push(...) / chan_<rid>.wait() /
        // bar_<bid>.wait() — but barriers in mp-tcp-event don't lower
        // to `bar_<bid>.wait()`. They go through CTRL-channel
        // wire::barrier_cross instead, and the walker's prefix knob
        // doesn't reach Event::Sync. So we cannot use the shared
        // walker directly for the Sync arm — we route around it by
        // emitting our own walker for Event::Sync.
        //
        // The cleanest split: emit a per-event recursive walker that
        // delegates Fire/Loop/Push/Wait to the shared walker but
        // special-cases Sync. But the shared walker is a single
        // function on a borrowed context — splitting per-event would
        // duplicate the Fire/Loop logic.
        //
        // Pragmatic solution: extend the shared walker (already
        // emitting `{prefix}bar_<tag>.wait()`) by ALSO declaring a
        // `bar_<tag>` local on this worker that's a small CTRL-channel
        // shim. The shim's `.wait()` calls wire::barrier_cross on the
        // CTRL socket. That way the walker is reused VERBATIM, and the
        // mp-tcp-event-specific work all lives in the emitted prelude.
        //
        // See emit_barrier_shims below.
        self.emit_barrier_shims(&mut out, worker, is_host)?;
        if !self.barriers_used_by(worker).is_empty() {
            writeln!(out).ok();
        }

        // TASK-0327 (cycle 149): for HOST, when the schedule has w2w
        // pushes, splice the synchronous relay phase right BEFORE the
        // LAST top-level `Event::Sync` event (= between pass-1 barrier
        // and pass-2 barrier on the 06/distributed2 reproducer). The
        // relay phase drains all worker-to-worker (seq, dst) hops
        // through host's existing reactor: `wait(seq)` blocks driving
        // the reactor (pulling frames from `data_<src>` mio sockets
        // into `inbound[seq]`); `push(seq, dst_peer, payload, cap)`
        // enqueues to `outbound[(seq, dst_peer)]` (drained on next
        // reactor turn into `data_<dst>` mio socket). Non-host workers
        // render unchanged — their `chan_<rid>` uses peer_idx=0 (host)
        // for both push and wait; host does the forwarding.
        //
        // Mirror of mp-tcp-bufsync's cycle-148 splice in
        // `nucleus/backends/mp-tcp-bufsync/src/lib.rs` `render_worker_program`.
        let host_events = &self.per_worker[&worker];
        let host_relay = if is_host {
            self.render_relay_phase(1)?
        } else {
            String::new()
        };
        if is_host && !host_relay.is_empty() {
            let split_at =
                relay_phase_insertion_point(host_events, self.per_worker, self.host_worker);
            let mut pre = String::new();
            walker::render_worker_events(
                &walker_ctx,
                worker,
                &host_events[..split_at],
                &mut pre,
                1,
                "",
            )?;
            out.push_str(&pre);
            out.push_str(&host_relay);
            let mut post = String::new();
            walker::render_worker_events(
                &walker_ctx,
                worker,
                &host_events[split_at..],
                &mut post,
                1,
                "",
            )?;
            out.push_str(&post);
        } else {
            let body = {
                let mut buf = String::new();
                walker::render_worker_events(
                    &walker_ctx,
                    worker,
                    host_events,
                    &mut buf,
                    1,
                    "",
                )?;
                buf
            };
            out.push_str(&body);
        }

        // ---- Flush outbound + close. ----
        writeln!(out).ok();
        writeln!(out, "    reactor.borrow_mut().flush_outbound();").ok();

        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// TASK-0327 (cycle 149): per non-host src worker, the ordered
    /// list of (seq, dst, data, cap) for every w2w Push event in src's
    /// event list (src != host && dst != host). Event-list order
    /// equals the order in which the host's relay should drain
    /// `inbound[seq]` for that src — though since `wait(seq)` is
    /// per-seq-demuxed, ordering across hops only affects latency,
    /// not correctness.
    ///
    /// Empty for any src with no w2w pushes. Empty overall if the
    /// schedule has no w2w transfers (the common host↔worker-only
    /// case), and then `render_relay_phase` is a no-op.
    fn relay_schedule(&self) -> Result<BTreeMap<WorkerId, Vec<RelayHop>>, EmitError> {
        let mut out: BTreeMap<WorkerId, Vec<RelayHop>> = BTreeMap::new();
        for (src, events) in self.per_worker.iter() {
            if *src == self.host_worker {
                continue;
            }
            let mut hops: Vec<RelayHop> = Vec::new();
            collect_w2w_pushes(events, self.host_worker, &self.chan_caps, &mut hops)?;
            if !hops.is_empty() {
                out.insert(*src, hops);
            }
        }
        Ok(out)
    }

    /// TASK-0327 (cycle 149): emit host's synchronous relay phase as a
    /// String — for each src in BTreeMap (sorted WorkerId) order, for
    /// each hop in src's event-list order, call
    /// `reactor.borrow_mut().relay_one(seq, dst_peer_idx_at_host, cap)`.
    /// `relay_one` (defined in `runtime.rs`) does `wait(seq)` then
    /// `push(seq, dst_peer, payload, cap)` — bytes-verbatim forwarding,
    /// no re-encode. The whole batch runs inside a single
    /// `reactor.borrow_mut()` scope so no other reactor borrow can
    /// interleave (single-threaded RefCell on host).
    ///
    /// Returns `EmitError::ContractGap` if any hop's `DataId` lacks a
    /// name in `NameTables` — same fail-loud contract as the Push/Wait
    /// emit path. Cycle-148 architect P2.2 lesson applies: bubble
    /// data_name errors rather than silently inlining a `{DataId:?}`
    /// fallback in the comment.
    fn render_relay_phase(&self, indent: usize) -> Result<String, EmitError> {
        let pad = "    ".repeat(indent);
        let schedule = self.relay_schedule()?;
        if schedule.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        writeln!(
            out,
            "{pad}// TASK-0327 host-relay phase: forward worker-to-worker Push/Wait\n\
             {pad}// pairs through host's existing per-(host,worker) star-topology\n\
             {pad}// reactor. SYNCHRONOUS: read inbound[seq] (from data_<src>),\n\
             {pad}// then re-push to outbound[(seq, dst_peer)] (toward data_<dst>),\n\
             {pad}// one (seq, dst) hop at a time, srcs iterated in sorted-WorkerId order."
        )
        .ok();
        writeln!(out, "{pad}{{").ok();
        writeln!(out, "{pad}    let mut __relay = reactor.borrow_mut();").ok();
        for (src, hops) in &schedule {
            let src_name = self.worker_name(*src);
            for hop in hops {
                let dst_name = self.worker_name(hop.dst);
                let data_name = self.data_name(hop.data)?;
                let dst_peer = self.peer_index_for(self.host_worker, hop.dst).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "mp-tcp-event relay: host has no peer index for dst {:?} \
                         on hop seq={:?} data={:?}",
                        hop.dst, hop.seq, hop.data
                    ))
                })?;
                writeln!(
                    out,
                    "{pad}    __relay.relay_one({seq}u64, {dst_peer}usize, {cap}usize); \
                     // relay `{data_name}` from {src_name} to {dst_name}",
                    seq = hop.seq.0,
                    dst_peer = dst_peer,
                    cap = hop.cap,
                )
                .ok();
            }
        }
        writeln!(out, "{pad}}}").ok();
        Ok(out)
    }

    /// Emit the rendezvous-file handshake + DATA/CTRL socket setup.
    /// Host = server, non-host = client. Identical pattern to
    /// mp-tcp-bufsync (TASK-0176); rendezvous-file mechanism is the
    /// SINGLE-ALLOCATOR pattern that closes the close-then-rebind
    /// TOCTOU window. DO NOT reintroduce the deleted
    /// `__nuc_pick_port` helper.
    fn emit_handshake(
        &self,
        out: &mut String,
        worker: WorkerId,
        is_host: bool,
    ) -> Result<(), EmitError> {
        let wname = self.worker_name(worker);
        if is_host {
            writeln!(
                out,
                "    let rendezvous_dir: PathBuf = std::env::var_os(\"NUC_RENDEZVOUS_DIR\")\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.map(PathBuf::from)\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|| panic!(\"host: NUC_RENDEZVOUS_DIR not set (run.sh must export it)\"));"
            )
            .ok();
            writeln!(out, "    let _ = &rendezvous_dir;").ok();
            // Per non-host worker: bind, publish port, accept DATA + CTRL.
            for nw in self.non_host_workers() {
                let nwn = self.worker_name(nw);
                writeln!(
                    out,
                    "    let listener_{nwn} = TcpListener::bind(\"127.0.0.1:0\")\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: bind 127.0.0.1:0 for {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(
                    out,
                    "    let port_{nwn}: u16 = listener_{nwn}.local_addr()\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: local_addr for {nwn} listener failed: {{e}}\"))\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.port();"
                )
                .ok();
                writeln!(
                    out,
                    "    {{\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20let rdv_final = rendezvous_dir.join(\"{nwn}.port\");\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20let rdv_tmp = rendezvous_dir.join(\"{nwn}.port.tmp\");\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20let mut f = fs::File::create(&rdv_tmp)\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: create rendezvous tmp `{{}}` for {nwn} failed: {{e}}\", rdv_tmp.display()));\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20write!(f, \"{{}}\", port_{nwn})\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: write port {{}} to rendezvous tmp `{{}}` for {nwn} failed: {{e}}\", port_{nwn}, rdv_tmp.display()));\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20drop(f);\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20fs::rename(&rdv_tmp, &rdv_final)\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: rename rendezvous `{{}}` -> `{{}}` for {nwn} failed: {{e}}\", rdv_tmp.display(), rdv_final.display()));\n\
                     \x20\x20\x20\x20}}"
                )
                .ok();
                writeln!(
                    out,
                    "    let (data_{nwn}_std, _) = listener_{nwn}.accept()\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: accept DATA from {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(
                    out,
                    "    let (ctrl_{nwn}_raw, _) = listener_{nwn}.accept()\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: accept CTRL from {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(out, "    data_{nwn}_std.set_nodelay(true).ok();").ok();
                writeln!(out, "    ctrl_{nwn}_raw.set_nodelay(true).ok();").ok();
                writeln!(out, "    wire::apply_sock_buf(&data_{nwn}_std);").ok();
                writeln!(out, "    wire::apply_sock_buf(&ctrl_{nwn}_raw);").ok();
                writeln!(
                    out,
                    "    let ctrl_{nwn}: Rc<RefCell<std::net::TcpStream>> = Rc::new(RefCell::new(ctrl_{nwn}_raw));"
                )
                .ok();
                // Convert std TcpStream -> mio TcpStream + set non-blocking.
                writeln!(
                    out,
                    "    data_{nwn}_std.set_nonblocking(true)\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: set_nonblocking on DATA to {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(
                    out,
                    "    let data_{nwn} = mio::net::TcpStream::from_std(data_{nwn}_std);"
                )
                .ok();
            }
        } else {
            // Non-host: read the rendezvous file then connect_retry
            // for DATA + CTRL.
            let wn = wname.clone();
            writeln!(
                out,
                "    let rendezvous_dir: PathBuf = std::env::var_os(\"NUC_RENDEZVOUS_DIR\")\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.map(PathBuf::from)\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|| panic!(\"{wn}: NUC_RENDEZVOUS_DIR not set (run.sh must export it)\"));"
            )
            .ok();
            writeln!(
                out,
                "    let rdv_path = rendezvous_dir.join(\"{wn}.port\");"
            )
            .ok();
            writeln!(
                out,
                "    fn read_rendezvous_port(path: &std::path::Path, who: &str) -> u16 {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20let mut attempt = 0u32;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20loop {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match fs::read_to_string(path) {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(s) => {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let trimmed = s.trim();\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return trimmed.parse::<u16>().unwrap_or_else(|e| panic!(\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\"{{}}: rendezvous file `{{}}` contained `{{}}` which is not a u16: {{}}\",\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20who, path.display(), trimmed, e\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20));\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(_e) => {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20attempt += 1;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if attempt > 600 {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20panic!(\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\"{{}}: rendezvous file `{{}}` did not appear within 6s ({{}} attempts x 10ms) — host worker did not start or failed to bind\",\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20who, path.display(), attempt\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20);\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20std::thread::sleep(Duration::from_millis(10));\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20}}"
            )
            .ok();
            writeln!(
                out,
                "    let port: u16 = read_rendezvous_port(&rdv_path, \"{wn}\");"
            )
            .ok();
            writeln!(
                out,
                "    fn connect_retry(port: u16, role: &str) -> StdTcpStream {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20let mut attempt = 0u32;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20loop {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match StdTcpStream::connect((\"127.0.0.1\", port)) {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(s) => return s,\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(e) => {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20attempt += 1;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if attempt > 600 {{ panic!(\"{wn}: cannot connect {{role}} to host 127.0.0.1:{{port}} after {{attempt}} tries: {{e}}\"); }}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20std::thread::sleep(Duration::from_millis(10));\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20}}"
            )
            .ok();
            writeln!(
                out,
                "    let data_host_std = connect_retry(port, \"DATA\");"
            )
            .ok();
            writeln!(
                out,
                "    let ctrl_host_raw = connect_retry(port, \"CTRL\");"
            )
            .ok();
            writeln!(out, "    data_host_std.set_nodelay(true).ok();").ok();
            writeln!(out, "    ctrl_host_raw.set_nodelay(true).ok();").ok();
            writeln!(out, "    wire::apply_sock_buf(&data_host_std);").ok();
            writeln!(out, "    wire::apply_sock_buf(&ctrl_host_raw);").ok();
            writeln!(
                out,
                "    data_host_std.set_nonblocking(true)\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"{wn}: set_nonblocking on DATA to host failed: {{e}}\"));"
            )
            .ok();
            writeln!(
                out,
                "    let data_host = mio::net::TcpStream::from_std(data_host_std);"
            )
            .ok();
            writeln!(
                out,
                "    let ctrl_host: Rc<RefCell<std::net::TcpStream>> = Rc::new(RefCell::new(ctrl_host_raw));"
            )
            .ok();
        }
        Ok(())
    }

    /// Build the Reactor with peer DATA sockets + the per-chan
    /// instances sized from the sidecar.
    fn emit_reactor_and_chans(&self, out: &mut String, worker: WorkerId) -> Result<(), EmitError> {
        // Build the peers Vec for Reactor::new in deterministic order.
        let peers: Vec<WorkerId> = if worker == self.host_worker {
            self.non_host_workers()
        } else {
            vec![self.host_worker]
        };
        writeln!(out, "    let reactor = {{").ok();
        writeln!(
            out,
            "        let peers: Vec<(mio::net::TcpStream, String)> = vec!["
        )
        .ok();
        for p in &peers {
            let pn = self.worker_name(*p);
            // The DATA socket variable name depends on the role:
            // host -> data_<peer_name>, non-host -> data_host.
            let var = if worker == self.host_worker {
                format!("data_{pn}")
            } else {
                "data_host".to_string()
            };
            writeln!(out, "            ({var}, \"{pn}\".to_string()),").ok();
        }
        writeln!(out, "        ];").ok();
        writeln!(
            out,
            "        Rc::new(RefCell::new(runtime::Reactor::new(peers)))"
        )
        .ok();
        writeln!(out, "    }};").ok();
        writeln!(out).ok();

        // Per-chan instances: chan_<rid> = Chan::new(reactor, seq,
        // peer_idx, cap, encode, decode).
        let touched = self.worker_chans(worker);
        for (key, rid) in &self.chan_ids {
            if !touched.contains(rid) {
                continue;
            }
            let (data, seq) = *key;
            let cap =
                self.chan_caps.get(key).copied().ok_or_else(|| {
                    EmitError::ContractGap(format!("missing chan_cap for {key:?}"))
                })?;
            let ty = self.sidecar.data_type(data).ok_or_else(|| {
                EmitError::ContractGap(format!("cross-worker data {data:?} has no ResolvedType"))
            })?;
            let rty = rust_type_of(ty);
            let peer_idx = self.chan_peer_index(worker, *key)?;
            let (enc, dec) = encode_decode_paths(ty);
            writeln!(
                out,
                "    let chan_{rid}: runtime::Chan<{rty}> = runtime::Chan::new(\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20Rc::clone(&reactor), {seq}u64, {peer_idx}usize, \
                 {cap}usize, {enc}, {dec},\n\
                 \x20\x20\x20\x20);",
                seq = seq.0,
                peer_idx = peer_idx,
                cap = cap,
            )
            .ok();
        }
        Ok(())
    }

    /// Emit the per-barrier shim locals. The shared walker emits
    /// `bar_<bid>.wait()`; mp-tcp-event maps that to a struct whose
    /// `.wait()` method calls `wire::barrier_cross` on the CTRL
    /// socket(s) for this barrier's participants. Host crosses with
    /// every non-host participant in WorkerId order; non-host crosses
    /// with host only.
    fn emit_barrier_shims(
        &self,
        out: &mut String,
        worker: WorkerId,
        is_host: bool,
    ) -> Result<(), EmitError> {
        let tags = self.barriers_used_by(worker);
        if tags.is_empty() {
            return Ok(());
        }
        // Inline the shim type ONCE (zero-cost; per-barrier instances
        // are tiny structs that borrow the CTRL streams).
        writeln!(
            out,
            "    // Barrier shim — per-barrier .wait() invokes wire::barrier_cross"
        )
        .ok();
        writeln!(
            out,
            "    // on the CTRL stream(s) for this barrier's participants."
        )
        .ok();
        // The shim type is the same shape across all barriers within
        // one worker: a struct holding Rc<RefCell<TcpStream>> clones
        // for every CTRL peer this barrier crosses. We DECLARE ONE
        // shim type per barrier so the field set matches that
        // barrier's peer set exactly (different barriers can name
        // different participant subsets — see TASK-0172 +
        // mp-tcp-bufsync's partial/non-uniform barrier proof).
        for tag in &tags {
            let bid = tag.0;
            let parts = self
                .barrier_participants
                .get(tag)
                .expect("tag in tags came from barriers_used_by");
            if is_host {
                let mut peers: Vec<WorkerId> = parts
                    .iter()
                    .copied()
                    .filter(|w| *w != self.host_worker)
                    .collect();
                peers.sort_unstable();
                writeln!(out, "    struct Bar{bid} {{").ok();
                for p in &peers {
                    let pn = self.worker_name(*p);
                    writeln!(out, "        ctrl_{pn}: Rc<RefCell<std::net::TcpStream>>,").ok();
                }
                writeln!(out, "    }}").ok();
                writeln!(out, "    impl Bar{bid} {{").ok();
                writeln!(out, "        fn wait(&self) {{").ok();
                for p in &peers {
                    let pn = self.worker_name(*p);
                    writeln!(
                        out,
                        "            wire::barrier_cross(&mut *self.ctrl_{pn}.borrow_mut(), {bid});"
                    )
                    .ok();
                }
                writeln!(out, "        }}").ok();
                writeln!(out, "    }}").ok();
                writeln!(out, "    let bar_{bid} = Bar{bid} {{").ok();
                for p in &peers {
                    let pn = self.worker_name(*p);
                    writeln!(out, "        ctrl_{pn}: Rc::clone(&ctrl_{pn}),").ok();
                }
                writeln!(out, "    }};").ok();
            } else {
                writeln!(out, "    struct Bar{bid} {{").ok();
                writeln!(out, "        ctrl_host: Rc<RefCell<std::net::TcpStream>>,").ok();
                writeln!(out, "    }}").ok();
                writeln!(out, "    impl Bar{bid} {{").ok();
                writeln!(out, "        fn wait(&self) {{").ok();
                writeln!(
                    out,
                    "            wire::barrier_cross(&mut *self.ctrl_host.borrow_mut(), {bid});"
                )
                .ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let bar_{bid} = Bar{bid} {{ ctrl_host: Rc::clone(&ctrl_host) }};"
                )
                .ok();
            }
        }
        Ok(())
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

/// TASK-0332 (cycle 151 AC#2): detect the wait-before-push hazard at
/// codegen time so the synchronous host-relay's circular-seq-dependency
/// deadlock surfaces as a typed `EmitError::ContractGap` instead of a
/// runtime timeout. See the call site in `Plan::build` for the full
/// design narrative; this is the conservative-but-sound implementation.
///
/// Sibling: the same function exists in
/// `nucleus/backends/mp-tcp-bufsync/src/lib.rs` with the same shape +
/// a backend-specific message prefix. Per the cycle-148/149 paired-lift
/// discipline ([[feedback-silent-sibling-defect]] 10th firing), the
/// two implementations were added in the same cycle.
fn detect_wait_before_push_hazard(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> Result<(), EmitError> {
    for (&w, events) in per_worker {
        if w == host {
            continue;
        }
        // Precondition: this worker must have at least one w2w Push
        // for the deadlock cycle to involve it. A "pure consumer"
        // worker (only w2w Waits, no w2w Pushes) is NOT a src in
        // `Plan::relay_schedule`, so host's relay does not wait FOR
        // it, so this worker's wait-before-anything pattern cannot
        // close a deadlock cycle from its own side. Pure-consumer
        // workers are SAFE under host-relay.
        //
        // Cycle-151 architect P2 note (RESOLVED by TASK-0330): this
        // precondition scans only TOP-LEVEL events for w2w Pushes; the
        // `collect_w2w_pushes` helper recurses into Loop bodies. A
        // worker with Wait at top level + Push inside a Loop body
        // would be a false-negative for THIS detector — but TASK-0330
        // now fires a fail-loud ContractGap in `collect_w2w_pushes` for
        // any w2w Push found INSIDE a Loop body, so the Loop-body
        // hazard is rejected later in the pipeline (in
        // `render_relay_phase` rather than here in `Plan::build`). The
        // composition is sound: a Loop-body w2w Push CANNOT silently
        // reach codegen on either backend.
        let has_w2w_push = events
            .iter()
            .any(|e| matches!(e, Event::Push { dst, .. } if *dst != host));
        if !has_w2w_push {
            continue;
        }
        for e in events {
            match e {
                // First top-level w2w event is a Push — safe shape;
                // host's relay can drain this worker's outbound first.
                Event::Push { dst, .. } if *dst != host => break,
                // First top-level w2w event is a Wait — hazard shape;
                // host's relay would deadlock waiting for THIS worker's
                // Push (which the worker can't reach because it's
                // blocked at this Wait).
                Event::Wait { src, .. } if *src != host => {
                    return Err(EmitError::ContractGap(format!(
                        "mp-tcp-event: worker {w:?} has a worker-to-worker \
                         Wait (from src {src:?}) at top level before any \
                         worker-to-worker Push. Cycle-149's synchronous \
                         host-relay would deadlock on the circular seq \
                         dependency: host's relay blocks at wait(seq) for \
                         this worker's first Push; this worker blocks at \
                         this Wait for host's relay of the seq from \
                         {src:?}. TASK-0332 cycle 151 filed this defensive \
                         guard; TASK-0329.01.01 cycle 162 (Option D) \
                         landed `apply_safe_push_reorder` which hoists \
                         hoistable Pushes ahead of preceding Waits before \
                         this check runs. If you are seeing this guard \
                         fire, the candidate Push was NOT hoistable — \
                         likely because a preceding Fire / Wait / Loop \
                         body writes data the Push depends on, or a \
                         preceding w2w Wait covers an overlapping tile \
                         on a shared axis. See \
                         `nucleus-compiler/src/passes/safe_push_reorder.rs` \
                         for the hoistability predicate."
                    )));
                }
                // Non-w2w events (Push/Wait with host as the other
                // endpoint, Fire, Sync, Loop, Alloc, Free) don't
                // affect the hazard. Loop bodies are intentionally
                // NOT walked — the hazard is about top-level event
                // order, not nested.
                _ => continue,
            }
        }
    }
    Ok(())
}

/// Per-pair Push collector: records the (src, dst) of every cross-
/// worker Push. Mirrors `collect_xfer_pairs` but the dst comes from
/// the Push event itself (not from the worker doing the visit).
///
/// **Loop-body interaction with TASK-0330**: this walker recurses into
/// `Event::Loop` bodies and uses `or_insert`, so a w2w Push appearing
/// inside a Loop body would be recorded once (first-visit-wins). The
/// TASK-0330 guard in `collect_w2w_pushes` rejects that shape upstream
/// before host emit reaches `render_relay_phase`, so this walker's
/// duplicate-tolerance is incidentally robust but not load-bearing for
/// the Loop-body case.
fn collect_push_pairs(
    events: &[Event],
    src: WorkerId,
    out: &mut BTreeMap<(DataId, SeqTag), (WorkerId, WorkerId)>,
) {
    for e in events {
        match e {
            Event::Push { data, dst, seq, .. } => {
                out.entry((*data, *seq)).or_insert((src, *dst));
            }
            Event::Loop { body, .. } => collect_push_pairs(body, src, out),
            _ => {}
        }
    }
}

/// TASK-0327 (cycle 149): one host-relay hop for a worker-to-worker
/// `Push`/`Wait` pair on the mp-tcp-event star topology — "drain
/// `inbound[seq]` from src worker, re-push to `outbound[(seq, dst_peer
/// at host)]` toward dst worker". `data` is for codegen-comment
/// disambiguation only; the wire pass-through is bytes-verbatim.
/// Cap is the chan's per-pair `outbound` bound (`chan_caps`).
#[derive(Debug, Clone, Copy)]
struct RelayHop {
    seq: SeqTag,
    dst: WorkerId,
    data: DataId,
    cap: u64,
}

/// TASK-0327 (cycle 149) + TASK-0329.01.01 (slice 1): pick the position
/// in HOST's top-level event list at which the host-relay phase should
/// splice in.
///
/// **Cycle 149 design** (scatter-compute-gather): heuristic returned the
/// LAST top-level `Sync` index — splicing relay between pass-1 barrier
/// and pass-2 barrier on schedules like `06-separable-filter/distributed`.
///
/// **Cycle 161/162 update (TASK-0329.01.01 slice 1)**: schedules like
/// `05-stencil/distributed-2d` exchange halo data BEFORE any barrier —
/// after the TASK-0329.01.01 `apply_safe_push_reorder` pass hoists each
/// worker's halo Pushes above its halo Waits, every w2w event happens
/// strictly before the first top-level Sync. The cycle-149 LAST-Sync
/// heuristic placed relay AFTER that Sync, leaving workers blocked at
/// their halo Waits while host blocked at the barrier — observed as a
/// 32s timeout deadlock (cycle 150 empirical finding, TASK-0332).
///
/// The new algorithm walks host's Syncs in event-list order and returns
/// the FIRST Sync (by host's index) such that, for every non-host
/// worker, every w2w event in that worker's event list occurs strictly
/// BEFORE the matching Sync (by `SyncTag` identity — TASK-0172). For
/// schedules where workers' w2w activity is between two barriers (e.g.
/// 06/distributed: w2w activity between bar_0 and bar_1), this still
/// returns bar_1's index — byte-identical with the cycle-149 heuristic.
/// For schedules where workers' w2w activity precedes the first barrier
/// (e.g. 05/distributed-2d after `apply_safe_push_reorder`), this
/// returns bar_0's index, enabling relay to run BEFORE the barrier.
///
/// Constraint 3 of the cycle-149 design (per-seq demux removes the
/// stream-race hazard that bufsync's analogous splice has — see memory
/// `project-mp-tcp-event-vs-bufsync-safety-profile`) is still INERT on
/// mp-tcp-event: moving the relay earlier in host's events is safe
/// because each `relay_one(seq, ...)` drains a distinct `inbound[seq]`
/// queue. **mp-tcp-bufsync's `relay_phase_insertion_point` is NOT
/// updated** — see AC#3b of TASK-0329.01.01 + the comment at the
/// bufsync splice site.
///
/// Fallbacks (preserving cycle-149 behaviour for schedules where the
/// scan finds nothing): LAST Sync (cycle-149 primary), then FIRST
/// top-level Wait, then end-of-events.
fn relay_phase_insertion_point(
    host_events: &[Event],
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> usize {
    'outer: for (host_idx, ev) in host_events.iter().enumerate() {
        let Event::Sync { sync: host_tag, .. } = ev else {
            continue;
        };
        for (&w, w_events) in per_worker {
            if w == host {
                continue;
            }
            // Find this worker's matching Sync by SyncTag (TASK-0172).
            let Some(w_sync_idx) = w_events
                .iter()
                .position(|e| matches!(e, Event::Sync { sync, .. } if sync == host_tag))
            else {
                // Worker doesn't participate in this Sync. Not a hard
                // disqualifier — host's barrier shim handles partial
                // barriers via the `barrier_participants` set. Skip
                // this worker for the all-complete check.
                continue;
            };
            // Are there any w2w events at or after this worker's
            // matching Sync? If yes, this host Sync is too early.
            let has_post_w2w = w_events[w_sync_idx..].iter().any(|e| match e {
                Event::Push { dst, .. } if *dst != host => true,
                Event::Wait { src, .. } if *src != host => true,
                _ => false,
            });
            if has_post_w2w {
                continue 'outer;
            }
        }
        // Every non-host worker has finished w2w by this Sync; relay
        // splices at this host Sync index (= relay runs BEFORE this
        // Sync in host's event stream).
        return host_idx;
    }
    // Fallbacks (cycle-149 originals, kept for schedules that don't
    // match the scan — e.g. relay-needed shapes without any top-level
    // Sync would never reach `render_relay_phase` because the schedule
    // would have rejected at `detect_wait_before_push_hazard` first
    // for any non-trivial w2w shape).
    if let Some(idx) = host_events
        .iter()
        .rposition(|e| matches!(e, Event::Sync { .. }))
    {
        return idx;
    }
    if let Some(idx) = host_events.iter().position(|e| matches!(e, Event::Wait { .. })) {
        return idx;
    }
    host_events.len()
}

/// TASK-0327 (cycle 149): collect every Push event where the dst is a
/// non-host worker (= worker-to-worker push), in event-list order.
/// `cap` is looked up against `chan_caps` so the emitted relay code
/// can pass the right back-pressure bound.
///
/// Recurses into Loop bodies in event-list order; the relay block is
/// emitted FLAT outside any loop.
///
/// **TASK-0330 active guard** (defensive ContractGap):
/// when the recursion is INSIDE an `Event::Loop` body and encounters a
/// w2w `Push`, returns [`EmitError::ContractGap`] forward-linking
/// TASK-0330. The flat relay block would either over-count (host calls
/// `relay_one` once per (seq, dst) but the worker pushes N times around
/// the loop) or mis-order (the flat replay order would not align with
/// the loop's nested iteration order). Fail-loud at codegen > silent
/// miscompile, per [[feedback-panic-not-diagnostic-recurring]].
///
/// **TASK-0329.01.02 cycle 163 update + cycle-166 reframe (slice 2
/// lift + residual-class enumeration):** the compiler-level
/// `apply_host_data_relay_inject` ACFG pass
/// (`nucleus-compiler/src/passes/host_data_relay_inject.rs`) routes
/// every PAIRED non-host Push/Wait inside a Repeat body through host
/// BEFORE projection. After the pass, the in-Loop-body w2w-Push shape
/// this guard fires on is structurally impossible for shapes the pass
/// handled. The guard STAYS in place as a fail-loud safety net for
/// two precisely-enumerated residual classes (cycle-163b architect
/// P2.4 → cycle 166 audit confirmed both classes still reachable in
/// principle):
///
/// - **(R-bare)** A bare `Xfer` outside any parent `Sequence`. The
///   pass requires a sibling slot to land the 4 routed nodes into;
///   `rewrite_at`'s non-`Sequence` arm early-returns. In practice
///   every `Xfer` from `transfer_inject` is produced inside a
///   `Sequence` — this class is reachable only if `transfer_inject`'s
///   contract weakens.
/// - **(R-singleton)** A `Push` (or `Wait`) without its matching
///   sibling endpoint in the SAME `Sequence`. The pass's pair-match
///   in `rewrite_sequence_children` requires both endpoints to live
///   in the same `Sequence`; if `transfer_inject::hoist_invariant_waits`
///   has hoisted one endpoint out, the unmatched endpoint is left
///   alone and would reach this guard.
///
/// In-tree schedules today: ZERO residual hits on the post-pass
/// matrix (verified across the 3 trigger cells in cycles 162/163/165
/// and re-verified bit-identical in cycle 166's audit). Test pins in
/// `nucleus/backends/mp-tcp-event/tests/loop_body_w2w_push.rs`
/// continue to assert the guard's contract on synthetic
/// pass-bypassing fixtures (those fixtures construct the Loop-body
/// shape directly, never via the pass).
fn collect_w2w_pushes(
    events: &[Event],
    host: WorkerId,
    chan_caps: &BTreeMap<(DataId, SeqTag), u64>,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    collect_w2w_pushes_inner(events, host, false, chan_caps, out)
}

fn collect_w2w_pushes_inner(
    events: &[Event],
    host: WorkerId,
    inside_loop: bool,
    chan_caps: &BTreeMap<(DataId, SeqTag), u64>,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Push { dst, data, seq, .. } if *dst != host => {
                if inside_loop {
                    return Err(EmitError::ContractGap(format!(
                        "mp-tcp-event: TASK-0330 defensive guard — \
                         worker-to-worker Push (data={data:?}, dst={dst:?}, \
                         seq={seq:?}) found INSIDE an Event::Loop body. The \
                         cycle-149 host-relay (TASK-0327) emits the relay \
                         block FLAT outside any loop, so a nested w2w Push \
                         would either over-count (host calls relay_one once \
                         per (seq, dst) but the worker pushes N times around \
                         the loop) or mis-order (the flat replay order would \
                         not align with the loop's nested iteration order). \
                         TASK-0329.01.02 cycle 163 (slice 2): the compiler \
                         pass `apply_host_data_relay_inject` routes every \
                         non-host-pair Push/Wait through host at the ACFG \
                         layer; if you are seeing this guard fire on an \
                         in-tree schedule, the pass either didn't run \
                         (driver wiring is mp-tcp-event-only — check \
                         driver/src/main.rs near `apply_host_data_relay_inject`) \
                         or the pair predicate didn't fire (Push/Wait not in \
                         the same Sequence — see \
                         `host_data_relay_inject::rewrite_sequence_children` \
                         singleton-left-alone comment for the residual class)."
                    )));
                }
                let cap = chan_caps.get(&(*data, *seq)).copied().ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "mp-tcp-event relay schedule: missing chan_caps for \
                         (data={data:?}, seq={seq:?}) — Push collected but \
                         Plan::build did not populate the cap"
                    ))
                })?;
                out.push(RelayHop {
                    seq: *seq,
                    dst: *dst,
                    data: *data,
                    cap,
                });
            }
            Event::Loop { body, .. } => {
                collect_w2w_pushes_inner(body, host, true, chan_caps, out)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Encoder/decoder fn-path for a ResolvedType. Returns `(encode_path,
/// decode_path)` as Rust expressions usable in `Chan::new(...)`.
///
/// The encoder takes `&T` (where `T = rust_type_of(ty)`) and returns
/// `Vec<u8>`. The decoder takes `&[u8]` and returns `T`.
fn encode_decode_paths(ty: &ResolvedType) -> (String, String) {
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        // Encoder: |v: &T| wire::enc_<s>(*v); Decoder: wire::dec_<s>.
        (
            format!("|v: &_| wire::enc_{s}(*v)"),
            format!("|b: &[u8]| wire::dec_{s}(b)"),
        )
    } else if ty.scalar == ScalarType::Bool {
        (
            "|v: &Vec<bool>| wire::enc_vec_bool(v)".to_string(),
            "|b: &[u8]| wire::dec_vec_bool(b)".to_string(),
        )
    } else {
        let rs = match &ty.scalar {
            ScalarType::I8 => "i8",
            ScalarType::I16 => "i16",
            ScalarType::I32 => "i32",
            ScalarType::I64 => "i64",
            ScalarType::U8 => "u8",
            ScalarType::U16 => "u16",
            ScalarType::U32 => "u32",
            ScalarType::U64 => "u64",
            ScalarType::F32 => "f32",
            ScalarType::F64 => "f64",
            ScalarType::Bool => unreachable!("handled above"),
            ScalarType::Usize => "u64", // wire-coerced
            ScalarType::Isize => "i64",
        };
        (
            format!("|v: &Vec<{rs}>| wire::enc_vec(v, {rs}::to_le_bytes)"),
            format!("|b: &[u8]| wire::dec_vec(b, {rs}::from_le_bytes)"),
        )
    }
}

fn scalar_fn_suffix(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::I8 => "i8",
        ScalarType::I16 => "i16",
        ScalarType::I32 => "i32",
        ScalarType::I64 => "i64",
        ScalarType::U8 => "u8",
        ScalarType::U16 => "u16",
        ScalarType::U32 => "u32",
        ScalarType::U64 => "u64",
        ScalarType::F32 => "f32",
        ScalarType::F64 => "f64",
        ScalarType::Bool => "bool",
        ScalarType::Usize => "u64",
        ScalarType::Isize => "i64",
    }
}

fn scalar_width(t: &ScalarType) -> usize {
    match t {
        ScalarType::I8 | ScalarType::U8 | ScalarType::Bool => 1,
        ScalarType::I16 | ScalarType::U16 => 2,
        ScalarType::I32 | ScalarType::U32 | ScalarType::F32 => 4,
        ScalarType::I64 | ScalarType::U64 | ScalarType::F64 => 8,
        ScalarType::Usize | ScalarType::Isize => 8,
    }
}

/// Per-backend SO_BUF commentary block interpolated by the shared
/// [`backend_common::project_skeleton::multi_binary::render_run_sh_multi`]
/// before the `export NUC_SO_BUF=...` line. mp-tcp-event additionally
/// buffers `buffer=N` frames per (seq, peer) in the APPLICATION ring,
/// so the SO_*BUF sizing here is the WIRE frame size, not the ring
/// depth — a different rationale than mp-tcp-bufsync's sync-one-msg
/// sizing, hence the per-backend comment.
const SO_BUF_COMMENT_EVENT: &str = "# Socket buffer requirement from the schedule's per-channel\n\
     # buffer needs (largest single transfer payload). Same shape\n\
     # as mp-tcp-bufsync — mp-tcp-event additionally buffers up to\n\
     # `transfer DATA : buffer=N` frames per (seq, peer) IN THE\n\
     # APPLICATION ring, so the SO_*BUF sizing here is the WIRE\n\
     # frame size, not the ring depth.\n";

/// Multi-process run.sh: delegate to the shared
/// [`backend_common::project_skeleton::multi_binary::render_run_sh_multi`]
/// (lifted in TASK-0257 cycle 112), supplying the host-first worker
/// ordering + mp-tcp-event-specific SO_BUF commentary.
pub(crate) fn render_run_sh(plan: &Plan<'_>) -> Result<String, EmitError> {
    let bufsz = plan.max_payload_bytes()?.max(65536);
    let host_name = plan.worker_name(plan.host_worker);
    let non_host_names: Vec<String> = plan
        .non_host_workers()
        .iter()
        .map(|w| plan.worker_name(*w))
        .collect();
    Ok(
        backend_common::project_skeleton::multi_binary::render_run_sh_multi(
            &host_name,
            &non_host_names,
            bufsz,
            SO_BUF_COMMENT_EVENT,
        ),
    )
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
// Branches B/C/D have integration tests in
// `tests/multi_worker_emit.rs` (they ARE reachable from `emit()` on
// 2+ workers, so the integration-test path is the right surface for
// them and matches the existing `host_excluding_barrier_is_typed_contract_gap`
// pattern).
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
