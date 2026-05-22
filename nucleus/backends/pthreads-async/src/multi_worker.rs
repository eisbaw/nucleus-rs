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
//!   shape — the Wait gather even reuses the leading-axis slice-paste
//!   logic mirroring pthreads-sync. Code duplication between this
//!   module and `pthreads_sync::multi_worker` is deliberate;
//!   factoring the shared event-walk into a parameterised helper is
//!   tracked as TASK-0239 (de-dup follow-up).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use compiler::algo::ResolvedType;
use compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, ViolationKind, WorkerId};
use compiler::sidecar::NameSidecar;

use pthreads_sync::{
    collect_count_check_frames, emit_count_branch, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static, emit_log_branch, render_const_expr_pub, render_fire_args_pub,
    render_fire_output_assign_pub, render_array_init_for, rust_type_of, sanitize_loop_var,
    RenderCtxPub,
};

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
/// pair's runtime channel). Same shape as pthreads-sync's `SlotId` —
/// a `usize` keyed by `(DataId, SeqTag)` ordered ascending.
pub(crate) type RingId = usize;

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

        // Host: the worker named "host", else the smallest used WorkerId.
        // Same rule as pthreads-sync — keeps host-side semantics
        // (input/output ownership, panic propagation) consistent across
        // the two single-binary backends.
        let host_named = names
            .worker
            .iter()
            .find(|(_, n)| n.as_str() == "host")
            .map(|(w, _)| *w)
            .filter(|w| used_workers.contains(w));
        // Mirror pthreads-sync's `.ok_or_else(...)` shape (cycle-20
        // review-gate E.2): typed error instead of `.expect()` so a
        // future refactor that breaks the `len() >= 2` guard above
        // surfaces as a typed ContractGap rather than a panic. The
        // guard is upstream; this branch is structurally unreachable
        // today, but the alignment with pthreads-sync's precedent
        // keeps error handling consistent across backends.
        let host_worker = host_named
            .or_else(|| used_workers.first().copied())
            .ok_or_else(|| {
                EmitError::ContractGap(
                    "pthreads-async Plan: used_workers reachable to host \
                     election but empty — invariant len() >= 2 violated"
                        .to_string(),
                )
            })?;

        // Collect cross-worker (DataId, SeqTag) pairs from every
        // Push/Wait in every worker's events. The pair-tile is the
        // IterTile carried on either endpoint (both endpoints share
        // it under transfer_inject's invariant).
        //
        // Walks Event::Loop bodies recursively (mirrors pthreads-sync's
        // collect_xfer_pairs in multi_worker.rs:1007).
        let mut pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
        for evs in per_worker.values() {
            collect_xfer_pairs(evs, &mut pair_tiles);
        }

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
                .ok_or_else(|| EmitError::ContractGap(format!(
                    "pthreads-async Plan: (data={data:?}, seq={seq:?}) Push/Wait \
                     pair has no entry in sidecar.transfer_buffer_for_seq. \
                     Either build_sidecar's walker missed an Xfer placeholder \
                     (TASK-0233 regression), or the EventList was projected \
                     without running transfer_inject first."
                )))?;
            ring_caps.insert((*data, *seq), cap);
        }

        // Barrier identity by the contract-carried `SyncTag` (TASK-0172).
        // Same shape as pthreads-sync's `multi_worker::Plan::build` at
        // multi_worker.rs:215-222: walk each used worker's events,
        // record the participant set the first time a SyncTag is seen.
        // Distinct tags are independent barriers, so partial/non-uniform
        // is fine without validation.
        let mut barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>> =
            BTreeMap::new();
        for w in &used_workers {
            collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
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

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            host_worker,
            ring_ids,
            ring_caps,
            pair_tiles,
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
            collect_worker_rings(evs, &self.ring_ids, &mut out);
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
        writeln!(
            out,
            "//! Generated by the pthreads-async backend (TASK-0228, multi-worker)."
        )
        .ok();
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
            let cap = self.ring_caps.get(&(*data_id, *seq)).copied().ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "ring_caps missing entry for (data={data_id:?}, seq={seq:?}); \
                     ring_ids and ring_caps must be 1:1 — see Plan::build invariant"
                ))
            })?;
            emit_ring_instance_decl(
                &mut out,
                &format!("ring_{ring_id}"),
                &rty,
                cap,
            );
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

        let ctx = RenderCtxPub::new(self.names, self.sidecar);
        self.render_worker_events(worker, evs, &mut out, base_indent, prefix, &ctx)?;
        Ok(out)
    }

    /// Recursive event-list walker. Mirrors pthreads-sync's
    /// `render_worker_events` (multi_worker.rs:531) with two
    /// substitutions: `slot_<id>` → `ring_<id>` in Push/Wait, and the
    /// Wait gather routes through this module's `render_wait_assign`
    /// (which uses `self.pair_tiles` / `self.ring_ids` — pthreads-async
    /// state, even though the slice-paste arithmetic is identical).
    fn render_worker_events(
        &self,
        worker: WorkerId,
        events: &[Event],
        out: &mut String,
        indent: usize,
        prefix: &str,
        ctx: &RenderCtxPub<'_>,
    ) -> Result<(), EmitError> {
        let pad = "    ".repeat(indent);
        for e in events {
            match e {
                Event::Fire {
                    kernel, bindings, ..
                } => {
                    let callee = self.names.kernel.get(kernel).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "kernel id {kernel:?} in a Fire has no name in NameTables"
                        ))
                    })?;
                    let args = render_fire_args_pub(*kernel, &bindings.inputs, ctx)?;
                    match &bindings.output {
                        None => {
                            writeln!(out, "{pad}kernels::{callee}({args});").ok();
                        }
                        Some(o) if o.indices.is_empty() => {
                            let name = self.data_name(o.data)?;
                            writeln!(
                                out,
                                "{pad}let mut {name} = kernels::{callee}({args});"
                            )
                            .ok();
                        }
                        Some(o) => {
                            let rhs = format!("kernels::{callee}({args})");
                            let stmt = render_fire_output_assign_pub(o, &rhs, ctx)?;
                            writeln!(out, "{pad}{stmt}").ok();
                        }
                    }
                }
                Event::Loop {
                    iter_var,
                    range,
                    body,
                    block_tag,
                    check_frame,
                } => {
                    let var = self.names.iter_var.get(iter_var).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                        ))
                    })?;
                    // Strip-mined multi-worker loops require per-
                    // occurrence absolute-index rebinding, which only
                    // lives on the shared single-worker path (TASK-0180);
                    // no tier-1 schedule blocks a multi-worker loop, so
                    // refuse to silently emit an un-rebound form. Same
                    // guard as pthreads-sync (TASK-0181).
                    if block_tag.is_some() {
                        return Err(EmitError::ContractGap(format!(
                            "Event::Loop for iter var `{var}` carries a strip-mine \
                             block_tag inside a MULTI-worker pthreads-async schedule; \
                             per-occurrence absolute-index rebinding lives only on \
                             the shared single-worker path (TASK-0180). Refusing to \
                             emit un-rebound (would double-count). Tracked as TASK-0181."
                        )));
                    }
                    // Defense-in-depth invariant (mirrors pthreads-sync
                    // multi_worker.rs:636): `inject_check_frames` is
                    // contracted to populate check_frame only on outer
                    // source loops (block_tag == None). The block_tag
                    // guard above already rejected, so this branch is
                    // structurally unreachable today — but if a future
                    // refactor weakens the outer guard, this catches
                    // the projection-layer regression rather than
                    // silently emitting an un-rebound check_frame.
                    if check_frame.is_some() && block_tag.is_some() {
                        return Err(EmitError::ContractGap(format!(
                            "Event::Loop for iter var `{var}` carries BOTH a \
                             check_frame and a block_tag — `inject_check_frames` is \
                             contracted to populate check_frame only on outer source \
                             loops; this is a projection-layer bug (TASK-0052.05 \
                             multi-worker invariant, pthreads-async mirror)."
                        )));
                    }
                    // Per-worker partition override (TASK-0212): use
                    // the concrete literal range if the partition pass
                    // recorded one for this worker on this iter var.
                    // Otherwise fall through to the source-form
                    // symbolic / literal precedence.
                    let partition_slice = self
                        .sidecar
                        .partition_worker_ranges
                        .get(iter_var)
                        .and_then(|m| m.get(&worker));
                    let (lo, hi) = match partition_slice {
                        Some(r) => (
                            format!("{}_i64", r.start),
                            format!("{}_i64", r.end),
                        ),
                        None => match self.sidecar.loop_bounds.get(iter_var) {
                            Some(b) => (
                                render_const_expr_pub(&b.lo, ctx)?,
                                render_const_expr_pub(&b.hi, ctx)?,
                            ),
                            None => (
                                format!("{}_i64", range.start),
                                format!("{}_i64", range.end),
                            ),
                        },
                    };
                    writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                    let body_indent = indent + 1;
                    let body_pad = "    ".repeat(body_indent);
                    if let Some(frame) = check_frame {
                        debug_assert_eq!(
                            var.as_str(),
                            frame.loop_var.as_str(),
                            "CheckFrame.loop_var diverged from NameTables.iter_var \
                             (projection-layer bug; TASK-0221)"
                        );
                        writeln!(
                            out,
                            "{body_pad}let _check_start = std::time::Instant::now();"
                        )
                        .ok();
                        self.render_worker_events(
                            worker, body, out, body_indent, prefix, ctx,
                        )?;
                        writeln!(
                            out,
                            "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                        )
                        .ok();
                        match frame.on_violation {
                            ViolationKind::Panic => {
                                writeln!(
                                    out,
                                    "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                                    ns = frame.latency_max_ns,
                                    lv = frame.loop_var,
                                )
                                .ok();
                            }
                            ViolationKind::Log => {
                                emit_log_branch(
                                    out,
                                    &body_pad,
                                    &frame.loop_var,
                                    frame.latency_max_ns,
                                );
                            }
                            ViolationKind::Count => {
                                let id = sanitize_loop_var(&frame.loop_var);
                                emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
                            }
                        }
                    } else {
                        self.render_worker_events(worker, body, out, indent + 1, prefix, ctx)?;
                    }
                    writeln!(out, "{pad}}}").ok();
                }
                Event::Sync { sync, .. } => {
                    let bid = sync.0;
                    writeln!(out, "{pad}{prefix}bar_{bid}.wait();").ok();
                }
                Event::Push { data, dst, seq, .. } => {
                    let rid = self.ring_ids.get(&(*data, *seq)).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Push of data {data:?} (seq {seq:?}) has no ring id \
                             (not collected as cross-worker)"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let to = self.worker_name(*dst);
                    writeln!(
                        out,
                        "{pad}{prefix}ring_{rid}.push({name}.clone()); // send `{name}` to {to}",
                    )
                    .ok();
                }
                Event::Wait {
                    data, src, seq, ..
                } => {
                    let rid = self.ring_ids.get(&(*data, *seq)).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Wait of data {data:?} (seq {seq:?}) has no ring id \
                             (not collected as cross-worker)"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let from = self.worker_name(*src);
                    let assign = self.render_wait_assign(
                        &name,
                        *data,
                        *seq,
                        &format!("{prefix}ring_{rid}.wait()"),
                    )?;
                    writeln!(out, "{pad}{assign} // recv `{name}` from {from}",).ok();
                }
                Event::Alloc { .. } | Event::Free { .. } => {
                    // RAII Vec storage; no explicit reservation.
                }
            }
        }
        Ok(())
    }

    /// Per-worker pre-init set: cross-worker inputs Waited on + data
    /// written via an indexed Fire output and never whole-array.
    /// Sorted by name (matches pthreads-sync's `collect_pre_init`).
    fn collect_pre_init(&self, worker: WorkerId) -> Result<Vec<(String, DataId)>, EmitError> {
        let evs = &self.per_worker[&worker];
        let mut waited: BTreeSet<DataId> = BTreeSet::new();
        let mut whole: BTreeSet<DataId> = BTreeSet::new();
        let mut indexed: BTreeSet<DataId> = BTreeSet::new();
        collect_pre_init_sets(evs, &mut waited, &mut whole, &mut indexed);

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

    /// Receiver-side assignment statement for one Wait. Two shapes:
    ///
    /// - **Whole-array assign** (`name = <rhs>;`) — the pre-TASK-0117
    ///   single-pair behaviour; selected when the pair's tile is
    ///   empty or covers the data's full leading-axis range.
    /// - **Slice-paste** (`{ let _tmp = <rhs>; name[lo..hi]
    ///   .copy_from_slice(&_tmp[lo..hi]); }`) — TASK-0117 host-side
    ///   gather; selected when the tile's outer axis is a strict
    ///   sub-range of the data's leading axis.
    ///
    /// Identical to pthreads-sync's `render_wait_assign` in shape +
    /// arithmetic; the only difference is which crate's Plan owns
    /// `pair_tiles` / `ring_ids`. The honest-limit on inner-axis
    /// partitions documented in pthreads-sync (multi_worker.rs:904
    /// onward) applies identically here.
    fn render_wait_assign(
        &self,
        name: &str,
        data: DataId,
        seq: SeqTag,
        rhs: &str,
    ) -> Result<String, EmitError> {
        let slice = match self.pair_tiles.get(&(data, seq)) {
            Some(tile) => self.leading_axis_slice(data, tile)?,
            None => None,
        };
        match slice {
            None => Ok(format!("{name} = {rhs};")),
            Some(LeadingAxis { lo, hi, stride }) => {
                let lo_off = lo.saturating_mul(stride);
                let hi_off = hi.saturating_mul(stride);
                Ok(format!(
                    "{{ let _tmp = {rhs}; \
                     {name}[{lo_off}usize..{hi_off}usize].copy_from_slice(\
                     &_tmp[{lo_off}usize..{hi_off}usize]); }}"
                ))
            }
        }
    }

    /// Tile-driven leading-axis slice computation. Mirrors
    /// pthreads-sync's `leading_axis_slice` (multi_worker.rs:922)
    /// verbatim. The honest-limit (assumes `tile.bounds[0].iter_var`
    /// names the data's leading dim) is inherited; for `partition=
    /// workers` on the leading axis (the in-tree case) this holds.
    fn leading_axis_slice(
        &self,
        data: DataId,
        tile: &IterTile,
    ) -> Result<Option<LeadingAxis>, EmitError> {
        let Some((_iv, range)) = tile.bounds.first() else {
            return Ok(None);
        };
        let ty: &ResolvedType = self.sidecar.data_type(data).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "Wait of data {data:?} has no ResolvedType in NameSidecar"
            ))
        })?;
        if ty.dims.is_empty() {
            return Ok(None);
        }
        let leading_dim = ty.dims[0] as i64;
        if range.start == 0 && range.end == leading_dim {
            return Ok(None);
        }
        if range.start < 0 || range.end > leading_dim || range.start >= range.end {
            return Err(EmitError::ContractGap(format!(
                "Wait of data {data:?}: tile leading-axis range {:?} out of \
                 bounds for data dims {:?} (leading-dim {})",
                range, ty.dims, leading_dim
            )));
        }
        let stride: usize = ty.dims[1..].iter().product();
        Ok(Some(LeadingAxis {
            lo: range.start as usize,
            hi: range.end as usize,
            stride,
        }))
    }
}

/// Leading-axis slice descriptor for the host-side gather codegen
/// (TASK-0117). Same shape as pthreads-sync's `LeadingAxis`; duplicated
/// here so the slice-paste arithmetic stays a single-crate concern
/// until TASK-0239 lifts it to a shared backend-common location.
struct LeadingAxis {
    lo: usize,
    hi: usize,
    /// Product of the data type's inner dims; per-outer-axis stride
    /// in flat-Vec elements.
    stride: usize,
}

/// Visit every `Event::Wait` / `Event::Fire` output to build the
/// three sets needed for the pre-init computation. Identical to
/// pthreads-sync's `collect_pre_init_sets`.
fn collect_pre_init_sets(
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

/// Collect unique Count-violation `check_frame` instances across every
/// worker's event list. Multiple workers can carry the SAME (loop_var,
/// threshold, ViolationKind::Count) frame (partition=workers projects
/// the same source loop onto N workers); dedup by sanitized ident so
/// the file-scope `AtomicU64` static + Drop guard are emitted exactly
/// once per UNIQUE ident.
///
/// Lifts pthreads-sync's `collect_unique_count_check_frames` shape
/// into pthreads-async; the helper there is `pub(crate)` so cannot be
/// reused directly. TASK-0239 covers de-dup.
fn collect_unique_count_check_frames(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
) -> Vec<pthreads_sync::CountCheckLoop> {
    let mut by_ident: BTreeMap<String, pthreads_sync::CountCheckLoop> = BTreeMap::new();
    for evs in per_worker.values() {
        for cf in collect_count_check_frames(evs) {
            by_ident.entry(cf.ident.clone()).or_insert(cf);
        }
    }
    by_ident.into_values().collect()
}

/// Walk an event list collecting every `(DataId, SeqTag)` pair seen
/// on a `Push` or `Wait`, paired with the `IterTile` carried at
/// either endpoint. Descends into `Event::Loop` bodies so a
/// pipelined transfer rolled inside an outer loop is collected.
///
/// One entry per pair: the second sighting (the matching endpoint)
/// is `.or_insert_with` a no-op since the tile is identical on both
/// endpoints (transfer_inject invariant).
///
/// **Event::Sync is intentionally not handled by this walker** — it's
/// collected by the separate `collect_barriers_by_tag` walker (cycle 21,
/// TASK-0234 closed with option (b)). The two walkers are independent
/// because the data shapes are different (Push/Wait fall through a
/// pair-tile join; Sync is a flat (tag -> participants) map). Calling
/// both in `Plan::build` produces the full Plan in one read.
fn collect_xfer_pairs(
    events: &[Event],
    out: &mut BTreeMap<(DataId, SeqTag), IterTile>,
) {
    for e in events {
        match e {
            Event::Push {
                data, seq, tile, ..
            }
            | Event::Wait {
                data, seq, tile, ..
            } => {
                out.entry((*data, *seq))
                    .or_insert_with(|| tile.clone());
            }
            Event::Loop { body, .. } => collect_xfer_pairs(body, out),
            _ => {}
        }
    }
}

/// Sync visitor (cycle 21, TASK-0234 option b): invoke `f(sync_tag,
/// participants)` for each `Event::Sync`, descending into `Event::Loop`
/// bodies. Barrier identity is the contract-carried [`SyncTag`]
/// (TASK-0172) — no running index, no fallibility (every tag is
/// independent, so partial/non-uniform barriers lower correctly).
///
/// Mirrors `pthreads_sync::multi_worker::collect_barriers_by_tag`
/// (multi_worker.rs:1051). The two backends now share the SAME barrier
/// projection shape — when Wave B-2 emits `std::sync::Barrier`, the
/// per-barrier participant size is read from `barrier_participants[tag].len()`
/// exactly like pthreads-sync (multi_worker.rs ~lines 380-390).
fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants,
                sync,
                ..
            } => {
                f(*sync, participants);
            }
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}

/// Per-worker visit of Push/Wait events to collect the worker's
/// ring_id touch set. Descends into `Event::Loop` bodies.
fn collect_worker_rings(
    events: &[Event],
    ring_ids: &BTreeMap<(DataId, SeqTag), RingId>,
    out: &mut BTreeSet<RingId>,
) {
    for e in events {
        match e {
            Event::Push { data, seq, .. } | Event::Wait { data, seq, .. } => {
                if let Some(r) = ring_ids.get(&(*data, *seq)) {
                    out.insert(*r);
                }
            }
            Event::Loop { body, .. } => collect_worker_rings(body, ring_ids, out),
            _ => {}
        }
    }
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
        use compiler::{
            acfg_to_events, apply_block_transforms, build_acfg, build_sidecar,
            algo::{lower_algo, parse_algo},
            inject_syncs, inject_transfers, link,
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
        let acfg = inject_transfers(&linked, acfg);

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
        let host_name = plan
            .names
            .worker
            .get(&plan.host_worker)
            .map(String::as_str);
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

        let host_name = plan
            .names
            .worker
            .get(&plan.host_worker)
            .map(String::as_str);
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
            collect_barriers_by_tag(evs, &mut |tag, _| {
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
        let (per_worker, names, _real_sidecar) =
            lower("02-split-add", "schedules/split.sched.nuc");
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
}
