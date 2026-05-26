//! pthreads-async multi-worker codegen (TASK-0228 Wave B).
//!
//! # Status
//!
//! - **Wave A** (cycle 18, commit 1351c7e): pure-function emit
//!   helpers in `src/ring_buffer.rs` for the file-scope `Ring<T>`
//!   struct + per-instance `Arc<Ring<T>>` declarations. Independently
//!   unit-tested + runtime-validated.
//! - **Wave B-1** (cycle 20): the `Plan` data structure — collect
//!   cross-worker `(DataId, SeqTag)` pairs, assign per-pair
//!   `ring_id`, look up per-pair capacity via
//!   `NameSidecar::transfer_buffer_for_seq` (TASK-0233), build the
//!   per-worker dispatch context. Tests pin the Plan's correctness
//!   against real fixtures (02-split-add/split, 13-cnn-inference/
//!   pipeline_parallel).
//! - **Wave B-2** (cycle 26): `Plan::emit` + `render_main_rs_multi`
//!   wire the data structure into a real Rust program. The emitted
//!   `main.rs` mirrors pthreads-sync's multi-worker shape with two
//!   substitutions: the file-scope `Ring<T>` (bounded VecDeque +
//!   Condvar pair) replaces `Slot<T>` (one-shot Mutex+Condvar
//!   rendezvous), and per-pair `let ring_<id>: Arc<Ring<T>> = Arc::
//!   new(Ring::new(cap));` replaces `let slot_<id>: Arc<Slot<T>> =
//!   Arc::new(Slot::new());`. Everything else (per-worker
//!   thread::spawn, barriers, Fire/Loop/Sync/Wait gather,
//!   check_frame instrumentation) is structurally the same emit-string
//!   shape — the Wait gather reuses the leading-axis slice-paste
//!   logic mirroring pthreads-sync.
//! - **Cycle 31 (TASK-0239 de-dup)**: the shared event walker
//!   (`render_worker_events` + `render_wait_assign` + `leading_axis_
//!   slice` + `collect_pre_init_sets` + `collect_xfer_pairs` +
//!   `collect_barriers_by_tag` + `collect_worker_rendezvous` +
//!   `LeadingAxis`) was lifted out of both backends into
//!   `pthreads_sync::multi_worker_walker` (later moved to the
//!   `backend-common` crate), parameterised by ONE string —
//!   `rendezvous_prefix`. The three prefix-using backends today are
//!   `"slot"` (pthreads-sync), `"ring"` (pthreads-async), `"chan"`
//!   (mp-tcp-event); mp-tcp-bufsync is the fourth tier-1 backend but
//!   bypasses `render_worker_events` and calls `render_wait_assign`
//!   directly. All three prefix-using backends route through the
//!   shared walker; emission is byte-identical to the pre-refactor
//!   state. This module retains only the per-backend `Plan` shape
//!   (ring sizing from `transfer_buffer_for_seq`) and the `Plan::emit`
//!   orchestration (substrate decl + per-pair instance alloc + per-
//!   thread spawn).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static,
};
use backend_common::multi_worker_walker::{self as walker, RendezvousId, WalkerCtx};
use backend_common::render::{render_array_init_for, rust_type_of};

use crate::ring_buffer::{emit_ring_instance_decl, emit_ring_struct_decl};
use crate::{EmitError, NameTables};

/// Render the contents of `main.rs` for a multi-worker pthreads-async
/// schedule. Mirrors `pthreads_sync::multi_worker::render_main_rs_multi`
/// but emits the bounded `Ring<T>` runtime substrate (TASK-0228 Wave A)
/// instead of pthreads-sync's one-shot `Slot<T>`, sized per-pair from
/// `NameSidecar::transfer_buffer_for_seq` (TASK-0233).
pub(crate) fn render_main_rs_multi(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    let plan = Plan::build(per_worker, names, sidecar)?;
    plan.emit()
}

/// Stable identifier for one ring buffer (the `(DataId, SeqTag)`
/// pair's runtime channel). Same shape as pthreads-sync's `SlotId`
/// and mp-tcp-event's `ChanId` — a `usize` keyed by `(DataId,
/// SeqTag)` ordered ascending. As of TASK-0239 this is an alias for
/// the shared `backend_common::multi_worker_walker::RendezvousId`;
/// all three prefix-using backends (pthreads-sync, pthreads-async,
/// mp-tcp-event) use the same `BTreeMap<(DataId, SeqTag),
/// RendezvousId>` map type for their per-pair rendezvous index.
pub(crate) type RingId = RendezvousId;

/// Data structure capturing every fact a Wave B-2 emit() needs to
/// produce a multi-worker pthreads-async binary, derived purely from
/// the per-worker EventList + NameTables + NameSidecar.
///
/// Mirrors `pthreads_sync::multi_worker::Plan` field-for-field where
/// the underlying semantics match (workers, host election, pair
/// collection, tiles, barriers), and substitutes `ring_*` for `slot_*`
/// where the async-specific ring buffer replaces the sync-specific
/// single-slot rendezvous.
///
/// **Barriers + rings are orthogonal** (cycle-21 TASK-0234 decision —
/// option (b) from the cycle-20 review-gate A.1 finding): async
/// transfers replace the sync single-slot Push/Wait with a bounded
/// ring buffer, but `Event::Sync` barriers — used by `inject_syncs`
/// to fence cross-worker writes — are independent of transfer
/// semantics. A pipelined async schedule with cross-worker writes
/// (e.g. 13-cnn-inference/pipeline_parallel) needs BOTH ring buffers
/// (for the transfers) AND barriers (for the writes), so `Plan`
/// carries `barrier_participants` in lockstep with pthreads-sync. Wave
/// B-2 emits `std::sync::Barrier` from the same `barrier_participants`
/// shape pthreads-sync already proves works.
#[derive(Debug)]
pub(crate) struct Plan<'a> {
    pub(crate) per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    pub(crate) names: &'a NameTables,
    pub(crate) sidecar: &'a NameSidecar,
    /// Workers with a non-empty EventList, in WorkerId order.
    pub(crate) used_workers: Vec<WorkerId>,
    /// The host (worker named "host", else smallest used WorkerId).
    /// Same election rule as pthreads-sync's multi_worker::Plan.
    pub(crate) host_worker: WorkerId,
    /// Cross-worker Push/Wait pairs `(DataId, SeqTag) -> ring index`.
    /// Sorted ascending by `(DataId, SeqTag)` for deterministic IDs.
    pub(crate) ring_ids: BTreeMap<(DataId, SeqTag), RingId>,
    /// Per-pair ring capacity from `transfer DATA : buffer=N`. Joined
    /// from `NameSidecar::transfer_buffer_for_seq` (TASK-0233). One
    /// entry per `ring_ids` key — Wave B-2 will pass this directly to
    /// `ring_buffer::emit_ring_instance_decl(..., cap)`.
    pub(crate) ring_caps: BTreeMap<(DataId, SeqTag), u64>,
    /// Per-pair tile carried on the originating XferPlaceholder. The
    /// tile names the iteration-axis slice this pair is responsible
    /// for. Wave B-2 codegen consumes this for fan-out gather (TASK-0117).
    pub(crate) pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343 cycle 189). Mirrors pthreads-sync's
    /// `multi_worker::Plan::accumulate_waits` field; populated by
    /// `walker::collect_accumulate_waits` per worker and unioned with
    /// the WorkerId. Consumed by the shared event walker via
    /// `WalkerCtx::accumulate_waits` to switch Event::Wait emit from
    /// whole-array overwrite assign to element-wise `wrapping_add`
    /// accumulate. Empty for every cell without an overlapping-write
    /// fan-in.
    pub(crate) accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
    /// `SyncTag` -> participants. Keyed directly by the contract barrier
    /// identity (TASK-0172). The projection clones the same participant
    /// set into every participant's `Event::Sync`, so recording the set
    /// the first time a tag is seen is exact; no uniform-barrier
    /// validation is needed (and a partial/non-uniform barrier is fine
    /// — every tag is independent). Mirrors pthreads-sync's
    /// `multi_worker::Plan::barrier_participants` field-for-field —
    /// Wave B-2 emits `std::sync::Barrier::new(N)` keyed by `SyncTag`
    /// the same way pthreads-sync does (TASK-0172).
    pub(crate) barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>>,
}

impl<'a> Plan<'a> {
    /// Build the Plan from the per-worker EventList + names + sidecar.
    ///
    /// Returns `EmitError::ContractGap` if:
    ///
    /// - `used_workers.len() < 2` (single-worker should have been
    ///   handled by the caller — the single-worker `emit()` arm
    ///   delegates to pthreads-sync's `render_single_worker_main`).
    /// - A `(DataId, SeqTag)` Push/Wait pair has no entry in
    ///   `sidecar.transfer_buffer_for_seq` — that would mean
    ///   `build_sidecar` missed an Xfer placeholder (TASK-0233's
    ///   walker invariant is violated), and the ring cannot be sized.
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

        if used_workers.len() < 2 {
            return Err(EmitError::ContractGap(format!(
                "pthreads-async multi-worker Plan::build requires \
                 used_workers.len() >= 2; got {n}. Single-worker is \
                 handled by emit()'s single-worker arm (delegating to \
                 pthreads_sync::render_single_worker_main).",
                n = used_workers.len(),
            )));
        }

        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule (TASK-0336 cycle 164 lift). Same rule as
        // pthreads-sync — keeps host-side semantics (input/output
        // ownership, panic propagation) consistent across the two
        // single-binary backends.
        //
        // The `ok_or_else(ContractGap)` arm preserves cycle-20
        // review-gate E.2: typed error instead of `.expect()` so a
        // future refactor that breaks the `len() >= 2` guard above
        // surfaces as a typed ContractGap rather than a panic. The
        // guard is upstream; this branch is structurally unreachable
        // today, but the alignment with pthreads-sync's precedent
        // keeps error handling consistent across backends.
        let host_worker = backend_common::elect_host_from_worker_names(&names.worker, &used_workers)
            .ok_or_else(|| {
                EmitError::ContractGap(
                    "pthreads-async Plan: used_workers reachable to host \
                     election but empty — invariant len() >= 2 violated"
                        .to_string(),
                )
            })?;

        // Collect cross-worker (DataId, SeqTag) pairs from every
        // Push/Wait in every worker's events via the shared backend-
        // common helper (TASK-0300 cycle 130 hoist). The pair-tile is
        // the IterTile carried on either endpoint; both endpoints share
        // it under transfer_inject's invariant, so the helper's
        // first-sighting-wins choice is well-defined.
        let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> =
            walker::collect_pair_tiles(per_worker.values());

        // Deterministic ring_id assignment: ascending by (DataId, SeqTag).
        let ring_ids: BTreeMap<(DataId, SeqTag), RingId> = pair_tiles
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();

        // Capacity for each pair from the sidecar. A missing entry
        // is a contract-gap: TASK-0233's walker visits every Xfer
        // placeholder, and transfer_inject creates an Xfer for every
        // Push/Wait pair. So if a pair is in `pair_tiles` (= seen in
        // the EventList) but absent from `transfer_buffer_for_seq`,
        // either the walker missed it (regression) or the EventList
        // was built without running transfer_inject (caller bug).
        // Either way, fail-loud rather than default-size and produce
        // a runtime mismatch.
        let mut ring_caps: BTreeMap<(DataId, SeqTag), u64> = BTreeMap::new();
        for (data, seq) in pair_tiles.keys() {
            let cap = sidecar
                .transfer_buffer_for_seq
                .get(seq)
                .copied()
                .ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "pthreads-async Plan: (data={data:?}, seq={seq:?}) Push/Wait \
                     pair has no entry in sidecar.transfer_buffer_for_seq. \
                     Either build_sidecar's walker missed an Xfer placeholder \
                     (TASK-0233 regression), or the EventList was projected \
                     without running transfer_inject first."
                    ))
                })?;
            ring_caps.insert((*data, *seq), cap);
        }

        // Barrier identity by the contract-carried `SyncTag` (TASK-0172).
        // Same shape as pthreads-sync's `multi_worker::Plan::build` at
        // multi_worker.rs:215-222: walk each used worker's events,
        // record the participant set the first time a SyncTag is seen.
        // Distinct tags are independent barriers, so partial/non-uniform
        // is fine without validation.
        let mut barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            walker::collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        // Defensive site-local assertion (cycle-20 review-gate A.5):
        // ring_ids and ring_caps must be in 1:1 correspondence.
        // A divergence here would indicate that the (seq -> cap) join
        // silently collapsed two pairs onto one entry — only possible
        // if the SeqTag-globally-unique invariant (tightened in cycle
        // 19 at event.rs:155-167) regressed. The tests already pin
        // this, but a debug_assert at the build site catches a
        // production-build regression that the test suite misses.
        debug_assert_eq!(
            ring_ids.len(),
            ring_caps.len(),
            "ring_ids and ring_caps must be 1:1; a divergence here means \
             two distinct (DataId, SeqTag) pairs collapsed onto one \
             SeqTag in transfer_buffer_for_seq — see event.rs SeqTag \
             docstring (load-bearing for TASK-0233)."
        );

        // Per-worker overlapping-write accumulator classification
        // (TASK-0343 cycle 189) — mirrors pthreads-sync's Plan::build
        // accumulate_waits computation field-for-field.
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
            ring_ids,
            ring_caps,
            pair_tiles,
            accumulate_waits,
            barrier_participants,
        })
    }

    /// Per-worker subset of `ring_ids` — the set of ring IDs this
    /// worker touches via its Push or Wait events. Wave B-2 codegen
    /// uses this to clone the right `Arc<Ring<T>>` handles into each
    /// `thread::spawn` closure.
    ///
    /// Mirrors pthreads-sync's `collect_worker_slots` in
    /// multi_worker.rs:1028.
    pub(crate) fn worker_rings(&self, w: WorkerId) -> BTreeSet<RingId> {
        let mut out: BTreeSet<RingId> = BTreeSet::new();
        if let Some(evs) = self.per_worker.get(&w) {
            walker::collect_worker_rendezvous(evs, &self.ring_ids, &mut out);
        }
        out
    }

    /// The barrier `SyncTag`s a worker participates in, ascending tag
    /// order (matches pthreads-sync's `barriers_used_by`).
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

    fn worker_name(&self, w: WorkerId) -> String {
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

    /// Generate the full `main.rs` body. Mirrors
    /// `pthreads_sync::multi_worker::Plan::emit` (multi_worker.rs:237)
    /// with the `Slot<T>` substrate replaced by the bounded `Ring<T>`
    /// substrate and per-pair `Ring::new(cap)` sizing from the sidecar.
    /// The emit-string shape for everything that is not the substrate
    /// declaration / per-pair instance is structurally identical to the
    /// sync path; see TASK-0239 for the planned de-dup that lifts
    /// the shared walker into a parameterised helper.
    fn emit(&self) -> Result<String, EmitError> {
        let mut out = String::new();
        // Backend-agnostic header (TASK-0231): matches the pthreads-sync
        // multi-worker header line-for-line. The substrate (Ring vs
        // Slot) is the documented difference; the header is not.
        writeln!(out, "//! Generated by the nucleus pre-compiler.").ok();
        writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
        writeln!(out).ok();
        writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
        writeln!(out, "mod kernels;").ok();
        writeln!(out).ok();
        writeln!(out, "use std::sync::{{Arc, Barrier}};").ok();
        writeln!(out, "use std::thread;").ok();
        writeln!(out).ok();

        // File-scope `Ring<T>` struct via the Wave A helper. One
        // definition serves every per-pair instance; capacity is
        // baked into the instance via `Ring::new(cap)`.
        emit_ring_struct_decl(&mut out);
        writeln!(out).ok();

        // Multi-worker Count check_frame substrate (TASK-0052.05).
        // SAME emit-string shape as pthreads-sync's multi_worker
        // (multi_worker.rs:320): file-scope `AtomicU64` static per
        // UNIQUE sanitized ident; all worker threads `fetch_add`
        // (Relaxed) into the same global; host-thread Drop guard
        // aggregates ONE summary line at fn main exit. The shared
        // helpers (`emit_count_static`, `emit_count_reporter_struct`,
        // `collect_count_check_frames`) extracted under TASK-0222 are
        // the single source of truth; no per-backend drift.
        let count_frames = collect_unique_count_check_frames(self.per_worker);
        if !count_frames.is_empty() {
            emit_count_reporter_struct(&mut out);
            for cf in &count_frames {
                emit_count_static(&mut out, &cf.ident);
            }
            writeln!(out).ok();
        }

        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        // Per-Count-loop Drop guard local. Same shape as
        // pthreads-sync's emit; the host thread owns the guard so
        // the aggregate summary line prints once at fn main exit
        // (after every `handle.join()`).
        for cf in &count_frames {
            emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
        }
        if !count_frames.is_empty() {
            writeln!(out).ok();
        }

        // ---- Allocate per-pair Ring<T> instances. ----
        // Sized to `transfer_buffer_for_seq[seq]` (TASK-0233).
        // Iterates `ring_ids` in ascending `(DataId, SeqTag)` order
        // so ring indices stay deterministic.
        for ((data_id, seq), ring_id) in &self.ring_ids {
            let name = self.data_name(*data_id)?;
            let ty = self.sidecar.data_type(*data_id).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "cross-worker data `{name}` ({data_id:?}) has no ResolvedType \
                     in the NameSidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            let cap = self
                .ring_caps
                .get(&(*data_id, *seq))
                .copied()
                .ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "ring_caps missing entry for (data={data_id:?}, seq={seq:?}); \
                     ring_ids and ring_caps must be 1:1 — see Plan::build invariant"
                    ))
                })?;
            emit_ring_instance_decl(&mut out, &format!("ring_{ring_id}"), &rty, cap);
        }

        // ---- Allocate barriers (ascending SyncTag order). ----
        for (tag, parts) in &self.barrier_participants {
            let bid = tag.0;
            let cnt = parts.len();
            let part_names: Vec<&str> = parts
                .iter()
                .map(|w| self.names.worker.get(w).map(String::as_str).unwrap_or("?"))
                .collect();
            writeln!(
                out,
                "    let bar_{bid}: Arc<Barrier> = Arc::new(Barrier::new({cnt})); // participants: {{{}}}",
                part_names.join(",")
            )
            .ok();
        }
        writeln!(out).ok();

        // ---- Spawn non-host workers. ----
        let mut handles: Vec<String> = Vec::new();
        for w in &self.used_workers {
            if *w == self.host_worker {
                continue;
            }
            let wname = self.worker_name(*w);
            let used_rings = self.worker_rings(*w);
            let used_barriers = self.barriers_used_by(*w);

            for ring_id in &used_rings {
                writeln!(
                    out,
                    "    let {wname}_ring_{ring_id} = Arc::clone(&ring_{ring_id});"
                )
                .ok();
            }
            for tag in &used_barriers {
                let bid = tag.0;
                writeln!(out, "    let {wname}_bar_{bid} = Arc::clone(&bar_{bid});").ok();
            }
            writeln!(out, "    let {wname}_handle = thread::spawn(move || {{").ok();
            let body = self.render_worker_body(*w, 2, &format!("{wname}_"))?;
            out.push_str(&body);
            writeln!(out, "    }});").ok();
            handles.push(format!("{wname}_handle"));
            writeln!(out).ok();
        }

        // ---- Host body (bare ring/bar names, no closure prefix). ----
        let host_body = self.render_worker_body(self.host_worker, 1, "")?;
        out.push_str(&host_body);

        // ---- Join workers. ----
        writeln!(out).ok();
        for h in &handles {
            writeln!(out, "    {h}.join().expect(\"worker thread panicked\");").ok();
        }

        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// Render one worker's body: pre-init locals, then walk the
    /// EventList. `prefix` is `""` for the host (bare top-level
    /// ring/bar names) or `<wname>_` for a spawned worker (closure-
    /// captured clones).
    fn render_worker_body(
        &self,
        worker: WorkerId,
        base_indent: usize,
        prefix: &str,
    ) -> Result<String, EmitError> {
        let mut out = String::new();
        let pad = "    ".repeat(base_indent);
        let evs = &self.per_worker[&worker];

        // Pre-init: cross-worker inputs the worker Waits on + data it
        // writes via an indexed Fire output and never whole-array.
        // Same set semantics as pthreads-sync; sorted by name.
        let pre_init = self.collect_pre_init(worker)?;
        for (name, did) in &pre_init {
            let ty = self.sidecar.data_type(*did).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "pre-init data `{name}` ({did:?}) has no ResolvedType in sidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            let init = render_array_init_for(ty);
            writeln!(out, "{pad}let mut {name}: {rty} = {init};").ok();
        }
        if !pre_init.is_empty() {
            writeln!(out).ok();
        }

        // Dispatch through the shared walker (TASK-0239) — the
        // rendezvous prefix `"ring"` is the only knob distinguishing
        // pthreads-async's emit from pthreads-sync's. The Plan
        // structs differ (this one carries `ring_caps`), but the
        // per-worker event walk is identical modulo the prefix.
        let walker_ctx = WalkerCtx {
            names: self.names,
            sidecar: self.sidecar,
            rendezvous_prefix: "ring",
            rendezvous_ids: &self.ring_ids,
            pair_tiles: &self.pair_tiles,
            accumulate_waits: &self.accumulate_waits,
        };
        walker::render_worker_events(&walker_ctx, worker, evs, &mut out, base_indent, prefix)?;
        Ok(out)
    }

    /// Per-worker pre-init set: cross-worker inputs Waited on + data
    /// written via an indexed Fire output and never whole-array.
    /// Sorted by name (matches pthreads-sync's `collect_pre_init`).
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
}

// --------------------------------------------------------------------
// Walker helpers extracted to `backend_common::multi_worker_walker`
// (TASK-0239, originally landed in `pthreads_sync::multi_worker_walker`
// and later moved to backend-common). The shared walker —
// `render_worker_events`, `render_wait_assign`, the `WaitSlice`
// shape-dispatch (TASK-0294 / TASK-0117 `leading_axis_slice` +
// `LeadingAxis`), `collect_pre_init_sets`, `collect_xfer_pairs`,
// `collect_barriers_by_tag`, and the per-worker rendezvous-id
// collector — is the single source of truth across all three
// prefix-using backends: pthreads-sync (`rendezvous_prefix = "slot"`),
// pthreads-async (`rendezvous_prefix = "ring"`), and mp-tcp-event
// (`rendezvous_prefix = "chan"`). mp-tcp-bufsync is the fourth
// tier-1 backend but bypasses this walker. This module retains only
// the per-backend `Plan` shape (bounded `Ring<T>` substrate, per-pair
// capacity from `transfer_buffer_for_seq`) plus the `Plan::emit`
// orchestration above.

/// Collect unique Count-violation `check_frame` instances across every
/// worker's event list. Multiple workers can carry the SAME (loop_var,
/// threshold, ViolationKind::Count) frame (partition=workers projects
/// the same source loop onto N workers); dedup by sanitized ident so
/// the file-scope `AtomicU64` static + Drop guard are emitted exactly
/// once per UNIQUE ident.
fn collect_unique_count_check_frames(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
) -> Vec<backend_common::check_frame::CountCheckLoop> {
    let mut by_ident: BTreeMap<String, backend_common::check_frame::CountCheckLoop> =
        BTreeMap::new();
    for evs in per_worker.values() {
        for cf in collect_count_check_frames(evs) {
            by_ident.entry(cf.ident.clone()).or_insert(cf);
        }
    }
    by_ident.into_values().collect()
}

// --------------------------------------------------------------------
// Wave B-1 unit tests (cycle 20, TASK-0228)
// --------------------------------------------------------------------
//
// These tests pin the Plan::build invariants against real fixtures:
// 02-split-add/split (two workers, sync transfers, default buffer=1)
// and 13-cnn-inference/pipeline_parallel (four workers, mix of async
// buffer=3 + sync buffer=1). They are unit tests at the crate-private
// level because the Plan struct is `pub(crate)` — Wave B-2 will keep
// it that way and expose only render_main_rs_multi.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as BTM;
    use std::fs;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        here.parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .expect("three ancestors above pthreads-async crate")
            .to_path_buf()
    }

    /// Run the full pipeline + build the contract inputs for an
    /// example/schedule pair. Mirrors the pattern in
    /// `nucleus/backends/pthreads-async/tests/skeleton.rs`'s
    /// `lower_example_01_naive`, generalised to any example.
    fn lower(
        ex_rel: &str,
        sched_rel: &str,
    ) -> (BTreeMap<WorkerId, Vec<Event>>, NameTables, NameSidecar) {
        use nucleus_compiler::{
            acfg_to_events,
            algo::{lower_algo, parse_algo},
            apply_block_transforms, build_acfg, build_sidecar, inject_syncs, inject_transfers,
            link,
            sched::{lower_sched, parse_sched},
        };

        let root = repo_root();
        let ex = root.join("nuc-nucleus/examples").join(ex_rel);
        let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).expect("algo");
        let sched_src = fs::read_to_string(ex.join(sched_rel)).expect("sched");
        let algo_ir = lower_algo(&parse_algo(&algo_src).expect("parse_algo")).expect("lower_algo");
        let sched_ir =
            lower_sched(&parse_sched(&sched_src).expect("parse_sched")).expect("lower_sched");
        let linked = link(algo_ir, sched_ir).expect("link");
        let acfg = build_acfg(&linked).expect("build_acfg");
        let acfg = apply_block_transforms(&linked, acfg).expect("block_transforms");
        let acfg = inject_syncs(acfg);
        let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

        let per_worker = acfg_to_events(&acfg);
        let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
        // TASK-0238 (cycle 25): 5-field NameTables literal collapsed
        // to the centralized constructor.
        let names = NameTables::from_acfg(&acfg);
        (per_worker, names, sidecar)
    }

    #[test]
    fn build_rejects_single_worker_with_contract_gap() {
        // Single-worker EventList — Plan::build is for the multi-
        // worker arm only. The single-worker emit() arm in lib.rs is
        // the caller's responsibility (delegates to pthreads-sync).
        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTM::new();
        per_worker.insert(WorkerId(0), vec![]); // empty -> not used
        let names = NameTables::default();
        let sidecar = NameSidecar::default();
        let r = Plan::build(&per_worker, &names, &sidecar);
        match r {
            Err(EmitError::ContractGap(msg)) => {
                assert!(
                    msg.contains("used_workers.len() >= 2"),
                    "rejection message must name the violated invariant: {msg}"
                );
                assert!(
                    msg.contains("Single-worker") || msg.contains("single-worker"),
                    "rejection message should point at the single-worker arm: {msg}"
                );
            }
            other => panic!("expected ContractGap for single-worker; got {other:?}"),
        }
    }

    #[test]
    fn build_succeeds_for_02_split_with_default_buffer_1() {
        // 02-split-add/split: two workers (host + w_b), sync transfers
        // with default buffer=1. The Plan must:
        //   - Pick host_worker = "host" by name.
        //   - Have at least one ring (the cross-worker transfer).
        //   - Every ring_cap == 1 (default sync buffer).
        let (per_worker, names, sidecar) = lower("02-split-add", "schedules/split.sched.nuc");
        let plan = Plan::build(&per_worker, &names, &sidecar).expect("Plan::build");

        // Host election: the worker named "host" wins.
        let host_name = plan.names.worker.get(&plan.host_worker).map(String::as_str);
        assert_eq!(
            host_name,
            Some("host"),
            "host election must pick the worker named 'host', got {host_name:?}"
        );

        assert!(plan.used_workers.len() >= 2);
        assert!(
            !plan.ring_ids.is_empty(),
            "02-split-add/split must produce at least one cross-worker ring"
        );
        assert_eq!(
            plan.ring_ids.len(),
            plan.ring_caps.len(),
            "ring_ids and ring_caps must have a 1:1 entry correspondence"
        );

        // Every cap is 1 (no async, no buffer override).
        for (&key, &cap) in &plan.ring_caps {
            assert_eq!(
                cap, 1,
                "02-split-add/split has only default-buffer transfers; \
                 ring at {key:?} has cap={cap}, expected 1"
            );
        }
    }

    #[test]
    fn build_succeeds_for_13_pipeline_parallel_with_mixed_buffers() {
        // 13-cnn-inference/pipeline_parallel: 4 workers, 3 async
        // (buffer=3) + 1 sync (output, default buffer=1) transfers.
        // The Plan must:
        //   - Pick the worker named "host" as host_worker.
        //   - Have ring_caps containing exactly 3 entries of value 3
        //     (the async edges) plus some entries of value 1 (sync
        //     output hops).
        //   - ring_ids ascending 0..N-1 by (DataId, SeqTag) order.
        let (per_worker, names, sidecar) =
            lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
        let plan = Plan::build(&per_worker, &names, &sidecar).expect("Plan::build");

        let host_name = plan.names.worker.get(&plan.host_worker).map(String::as_str);
        assert_eq!(host_name, Some("host"));

        assert_eq!(plan.used_workers.len(), 4, "host + 3 stages");

        let count_3 = plan.ring_caps.values().filter(|&&v| v == 3).count();
        assert_eq!(
            count_3, 3,
            "pipeline_parallel has exactly 3 async transfers (buffer=3); \
             got {count_3} ring_caps with value 3 out of {:?}",
            plan.ring_caps
        );

        // ring_ids assigned ascending 0..N-1 in deterministic order.
        for (expected_id, &id) in plan.ring_ids.values().enumerate() {
            assert_eq!(
                id, expected_id,
                "ring_ids must be assigned in ascending order 0..N-1; \
                 found id={id} at position {expected_id}"
            );
        }
    }

    #[test]
    fn worker_rings_returns_subset_per_worker() {
        // Each worker only touches the rings whose (data, seq) it
        // produces or consumes. The union across all used_workers
        // covers the full ring_ids map.
        let (per_worker, names, sidecar) =
            lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
        let plan = Plan::build(&per_worker, &names, &sidecar).expect("Plan::build");

        let mut union: BTreeSet<RingId> = BTreeSet::new();
        for &w in &plan.used_workers {
            let touched = plan.worker_rings(w);
            assert!(
                touched.is_subset(&plan.ring_ids.values().copied().collect()),
                "worker {w:?} touches rings outside the Plan's ring_ids: {touched:?}"
            );
            union.extend(&touched);
        }
        // The union of per-worker touches must be at least every ring
        // (every Push/Wait pair has SOME producer + consumer worker).
        // It can exceed only in degenerate cases (impossible here).
        let all_rings: BTreeSet<RingId> = plan.ring_ids.values().copied().collect();
        assert_eq!(
            union, all_rings,
            "union of per-worker ring touches must equal the full ring set"
        );
    }

    #[test]
    fn build_populates_barrier_participants_for_multi_worker_sync_schedule() {
        // Cycle 21 / TASK-0234 option (b): the Plan now carries a
        // barrier_participants map populated by walking Event::Sync.
        // 02-split-add/split is a 2-worker schedule whose
        // inject_syncs pass produces cross-worker barriers; the map
        // must be non-empty after Plan::build.
        let (per_worker, names, sidecar) = lower("02-split-add", "schedules/split.sched.nuc");
        let plan = Plan::build(&per_worker, &names, &sidecar).expect("Plan::build");

        assert!(
            !plan.barrier_participants.is_empty(),
            "02-split-add/split has cross-worker writes; inject_syncs \
             produces Event::Sync barriers; Plan must record them. \
             Got: {:?}",
            plan.barrier_participants
        );

        // Every barrier's participant set is a subset of used_workers
        // (a barrier never names a non-participating worker).
        let used: BTreeSet<WorkerId> = plan.used_workers.iter().copied().collect();
        for (tag, parts) in &plan.barrier_participants {
            assert!(
                parts.is_subset(&used),
                "barrier {tag:?} names a worker not in used_workers; \
                 parts={parts:?}, used={used:?}"
            );
            assert!(
                !parts.is_empty(),
                "barrier {tag:?} has empty participant set; \
                 inject_syncs invariant violated"
            );
        }
    }

    #[test]
    fn build_records_one_entry_per_unique_sync_tag() {
        // The walker uses `.or_insert_with` on first sighting, so even
        // if multiple workers each carry an Event::Sync with the same
        // SyncTag (= they're ALL participants of one barrier), the
        // barrier_participants map has exactly ONE entry per unique
        // tag. Verify this against 13-cnn-inference/pipeline_parallel,
        // which has multi-stage barriers shared across workers.
        let (per_worker, names, sidecar) =
            lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
        let plan = Plan::build(&per_worker, &names, &sidecar).expect("Plan::build");

        // Independent count: walk every Event::Sync directly + collect
        // unique sync tags. Compare with barrier_participants.len().
        let mut all_tags: BTreeSet<SyncTag> = BTreeSet::new();
        for evs in per_worker.values() {
            walker::collect_barriers_by_tag(evs, &mut |tag, _| {
                all_tags.insert(tag);
            });
        }
        assert_eq!(
            plan.barrier_participants.len(),
            all_tags.len(),
            "barrier_participants should have ONE entry per unique \
             SyncTag (not N entries per N participants). Got map={}, \
             unique tags={}",
            plan.barrier_participants.len(),
            all_tags.len()
        );
    }

    #[test]
    fn build_fails_on_missing_sidecar_buffer_entry() {
        // Synthesise the pair-tile state of 02-split-add but with an
        // EMPTY sidecar.transfer_buffer_for_seq. Plan::build MUST
        // fail-loud — never default-size and produce a runtime
        // mismatch (the TASK-0233 contract-gap path).
        let (per_worker, names, _real_sidecar) = lower("02-split-add", "schedules/split.sched.nuc");
        let degenerate_sidecar = NameSidecar::default(); // empty maps
        let r = Plan::build(&per_worker, &names, &degenerate_sidecar);
        match r {
            Err(EmitError::ContractGap(msg)) => {
                assert!(
                    msg.contains("transfer_buffer_for_seq"),
                    "missing-cap message must name the sidecar field: {msg}"
                );
                assert!(
                    msg.contains("TASK-0233"),
                    "missing-cap message must forward-link the TASK-0233 contract: {msg}"
                );
            }
            other => panic!(
                "expected ContractGap when sidecar.transfer_buffer_for_seq is empty; \
                 got {other:?}"
            ),
        }
    }

    // ----------------------------------------------------------------
    // TASK-0235: edge-case coverage for `barrier_participants` (cycle-21
    // review-gate C.2 / F.3 follow-up). The existing TASK-0234 tests pin
    // the COMMON case (non-empty + uniform participants per tag); these
    // two synthetic fixtures cover the two edges that no in-tree e2e
    // example exercises.
    // ----------------------------------------------------------------

    #[test]
    fn build_zero_barrier_multi_worker_has_empty_barrier_participants() {
        // Edge (a): a multi-worker schedule with ZERO Event::Sync.
        // No real e2e fixture produces this (every schedule that
        // crosses worker boundaries also gets `inject_syncs` barriers),
        // so the only way to exercise it is to construct per_worker
        // directly. A future regression in the walker that synthesised
        // a phantom tag for non-Sync events (e.g. confusing a Push with
        // a barrier) would pass every existing test — this one pins
        // that the map stays empty when the input has no Sync.
        use nucleus_compiler::event::{DataId, SeqTag};
        let host = WorkerId(0);
        let w_b = WorkerId(1);
        let data = DataId(0);
        let seq = SeqTag(0);

        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTM::new();
        per_worker.insert(
            host,
            vec![Event::Push {
                dst: w_b,
                data,
                tile: IterTile::empty(),
                seq,
            }],
        );
        per_worker.insert(
            w_b,
            vec![Event::Wait {
                src: host,
                data,
                tile: IterTile::empty(),
                seq,
            }],
        );

        // Plan::build needs a transfer_buffer_for_seq entry for every
        // cross-worker (DataId, SeqTag) pair; no Sync needed.
        let names = NameTables::default();
        let mut sidecar = NameSidecar::default();
        sidecar.transfer_buffer_for_seq.insert(seq, 1);

        let plan = Plan::build(&per_worker, &names, &sidecar)
            .expect("Plan::build should succeed for 2 workers + 1 ring + 0 barriers");

        assert_eq!(
            plan.used_workers.len(),
            2,
            "fixture precondition: both workers carry events"
        );
        assert_eq!(
            plan.ring_ids.len(),
            1,
            "one cross-worker Push/Wait pair => one ring"
        );
        assert!(
            plan.barrier_participants.is_empty(),
            "no Event::Sync in any worker's events => barrier_participants \
             must stay empty; got {:?}",
            plan.barrier_participants
        );
    }

    #[test]
    fn build_asymmetric_participant_barriers_lower_correctly() {
        // Edge (b): two distinct SyncTags with DIFFERENT participant set
        // sizes within the same Plan. Mirrors pthreads-sync's
        // `partial_nonuniform_barrier_multi_worker_lowers_correctly`
        // (tests/multi_worker.rs:334), proving the docstring claim
        // "partial / non-uniform barriers lower correctly" for
        // pthreads-async's Plan too. A regression that synthesised a
        // UNION participant set across tags (or that pinned one
        // canonical size) would fail here.
        //
        // No Push/Wait: barriers alone are sufficient to exercise the
        // `collect_barriers_by_tag` walker + the `or_insert_with`
        // recording rule in Plan::build. Without Push/Wait the sidecar
        // can stay empty (no transfer_buffer_for_seq lookup happens).
        use nucleus_compiler::event::SyncKind;
        let host = WorkerId(0);
        let w0 = WorkerId(1);
        let w1 = WorkerId(2);
        let tag_a = SyncTag(0);
        let tag_b = SyncTag(1);

        // Two-participant barrier (tag A): {host, w0}.
        let parts_a: BTreeSet<WorkerId> = [host, w0].into_iter().collect();
        // Three-participant barrier (tag B): {host, w0, w1}.
        let parts_b: BTreeSet<WorkerId> = [host, w0, w1].into_iter().collect();

        // Per TASK-0172's contract: every participant carries the
        // Event::Sync for the barriers it participates in, with the
        // SAME participant set. host + w0 carry both; w1 carries only B.
        let sync_a = Event::Sync {
            participants: parts_a.clone(),
            kind: SyncKind::Barrier,
            sync: tag_a,
        };
        let sync_b = Event::Sync {
            participants: parts_b.clone(),
            kind: SyncKind::Barrier,
            sync: tag_b,
        };

        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTM::new();
        per_worker.insert(host, vec![sync_a.clone(), sync_b.clone()]);
        per_worker.insert(w0, vec![sync_a.clone(), sync_b.clone()]);
        per_worker.insert(w1, vec![sync_b.clone()]); // only tag B

        let names = NameTables::default();
        let sidecar = NameSidecar::default(); // no transfers, empty is fine

        let plan = Plan::build(&per_worker, &names, &sidecar)
            .expect("Plan::build should succeed for 3 workers + 0 rings + 2 barriers");

        assert_eq!(
            plan.used_workers.len(),
            3,
            "fixture precondition: all three workers carry at least one Sync"
        );
        assert!(
            plan.ring_ids.is_empty(),
            "no Push/Wait in any worker's events => no rings; got {:?}",
            plan.ring_ids
        );
        assert_eq!(
            plan.barrier_participants.len(),
            2,
            "two distinct SyncTags => two entries; got {:?}",
            plan.barrier_participants
        );

        let a_parts = plan
            .barrier_participants
            .get(&tag_a)
            .expect("tag A must be recorded");
        let b_parts = plan
            .barrier_participants
            .get(&tag_b)
            .expect("tag B must be recorded");

        assert_eq!(
            a_parts.len(),
            2,
            "tag A participants: {{host, w0}} => size 2; got {a_parts:?}"
        );
        assert_eq!(
            b_parts.len(),
            3,
            "tag B participants: {{host, w0, w1}} => size 3; got {b_parts:?}"
        );
        assert_ne!(
            a_parts.len(),
            b_parts.len(),
            "asymmetric barriers must record DIFFERENT participant-set \
             sizes; A={a_parts:?} B={b_parts:?}"
        );

        // Exact set identity (not just size): the walker must record
        // the participant set verbatim from the first Sync sighting,
        // not a union or canonicalised form.
        assert_eq!(a_parts, &parts_a, "tag A set identity must match input");
        assert_eq!(b_parts, &parts_b, "tag B set identity must match input");
    }
}
