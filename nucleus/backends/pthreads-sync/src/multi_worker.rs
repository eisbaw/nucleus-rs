//! Multi-worker codegen for the pthreads-sync backend
//! (TASK-0122, rewired to the EventList contract by TASK-0124).
//!
//! Lowers the per-worker [`Event`] lists of a ≥2-worker schedule
//! into a standalone Rust program that spawns one `std::thread` per
//! non-host worker, synchronises them via `std::sync::Barrier`, and
//! exchanges typed data via shared-memory `Slot<T>` (a
//! `Mutex<Option<T>>` + `Condvar`).
//!
//! ## AlgoIR-/LinkedIR-free (TASK-0124 AC#2)
//!
//! This module reads ONLY the per-worker `EventList`, the
//! [`NameTables`], and the [`NameSidecar`]. It does **not** touch
//! `compiler::algo` / `compiler::link`:
//!
//! - Slot allocation: derived from the [`Event::Push`] /
//!   [`Event::Wait`] events themselves (the set of cross-worker
//!   `DataId`s), not `linked.data_producers/consumers`. The
//!   `Event::Push.dst` / `Event::Wait.src` give the producer/consumer
//!   workers; `seq` pairs them (the projection already finalised the
//!   cross-scope Push/Wait pairing — TASK-0136/0139).
//! - Slot element type: from `sidecar.data_type(did)`, not
//!   `algo.data`.
//! - Pre-init sizing: from the sidecar, exactly as the single-worker
//!   emitter.
//! - Loop bounds: source form from `sidecar.loop_bounds` + consts.
//! - Kernel calls: reconstructed from `FireBinding` + name tables.
//!
//! ## Barrier identity (contract-carried — TASK-0172)
//!
//! [`Event::Push`]/[`Event::Wait`] carry a stable cross-worker
//! `seq` tag; [`Event::Sync`] now carries the analogous stable
//! cross-worker `sync: SyncTag` (TASK-0172). Every participant of one
//! barrier records an `Event::Sync` with the **same** `SyncTag`
//! (assigned once per barrier by the sync-injection pass, where the
//! global barrier structure is visible, and cloned into every
//! participant's list by the projection). This backend keys barriers
//! directly by that `SyncTag`: it collects the participant set per
//! tag and emits one `bar_<tag>` per distinct tag.
//!
//! No global ACFG walk and no per-worker pre-order-index heuristic is
//! needed — the disjoint per-worker EventLists agree on barrier
//! identity by construction. Because identity no longer depends on
//! every participant seeing the same prefix of barriers, a
//! **partial / non-uniform barrier** (Syncs whose participant sets
//! differ between barriers) lowers correctly: each `Sync` resolves to
//! its own `SyncTag`-keyed barrier regardless of what other barriers
//! a given worker does or does not take part in. The previous
//! pre-order-index heuristic, its uniform-barrier validation, and the
//! non-uniform [`EmitError::ContractGap`] are gone (TASK-0172) — they
//! were a workaround for the absence of the now-present contract id.
//! For a uniform-barrier program the `SyncTag`s are `0,1,2,…` in
//! pre-order (deterministic assignment in `inject_syncs`), the same
//! numbering the old heuristic produced, so generated code stays
//! byte-identical (example 02-split: three `{host,w0}` barriers).
//!
//! ## Scope and limitations (unchanged)
//!
//! - **Sync transfers only.** async / `buffer>1` are rejected
//!   upstream (capability check) and not representable here.
//! - **Single producer + single consumer entity per data symbol**
//!   (single-assignment, PRD §6.2.1). Multi-consumer fan-out is not
//!   exercised by the tier-1 set; if a data symbol is Waited by two
//!   distinct workers this path emits one slot per (producer,
//!   consumer) `seq` chain as the events dictate.
//! - **Distributed placements** are rejected before this path (the
//!   EventList for a distributed kernel would carry the same Fire on
//!   multiple workers; the projection + capability check handle
//!   that). This module does not re-validate placement — it consumes
//!   whatever the projection produced.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

// NOTE: `Event::Push`/`Wait` also carry a `SeqTag` (the stable
// cross-worker pairing tag). This backend keys slots by `DataId`
// (one slot per cross-worker data symbol — the pre-TASK-0124
// behaviour), so `SeqTag` is not consulted here; a future
// per-(seq) slot split (multi-buffer transfers) would use it.
use compiler::event::{DataId, Event, SeqTag, SyncTag, ViolationKind, WorkerId};
use compiler::sidecar::NameSidecar;

use crate::{
    collect_count_check_frames, emit_count_reporter_struct, render_const_expr_pub,
    rust_scalar_type, sanitize_loop_var, CountCheckLoop, EmitError, NameTables, RenderCtxPub,
};

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Render the contents of `main.rs` for a multi-worker schedule from
/// the per-worker EventList + name tables + sidecar.
pub(crate) fn render_main_rs_multi(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    let plan = Plan::build(per_worker, names, sidecar)?;
    plan.emit()
}

// --------------------------------------------------------------------
// Plan
// --------------------------------------------------------------------

/// Stable identifier for one Slot allocated for a cross-worker
/// Push/Wait pair (TASK-0117). Pre-TASK-0117, slots were keyed by
/// `DataId` alone (one slot per cross-worker data symbol); the
/// transfer-injection canonical-collapse meant a data symbol crossing
/// {host} → {w0,w1,w2,w3} produced exactly one pair, hence one slot.
/// Post-TASK-0117, the same crossing produces N pairs (one per
/// destination worker) and each pair MUST have its own slot — without
/// per-pair slots, four `slot.push(input.clone())` and four
/// `slot.wait()` would race nondeterministically.
///
/// Slots are now keyed by the `(DataId, SeqTag)` pair (= one slot per
/// XferPlaceholder pair). For an example whose data symbols each have
/// one pair (examples 01..07: 1:1 host↔single-worker transfers), the
/// keying degrades to the pre-TASK-0117 1-slot-per-data shape with
/// stable `slot_X` indices, because the BTreeSet iteration order over
/// (DataId, SeqTag) ascends by DataId first.
type SlotId = usize;
/// Stable identifier for one barrier — the contract-carried
/// [`SyncTag`] (TASK-0172). Same value for every participant of the
/// barrier; distinct between distinct barriers.
type BarrierId = SyncTag;

struct Plan<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    /// Workers with a non-empty EventList, in WorkerId order.
    used_workers: Vec<WorkerId>,
    /// The host (worker named "host", else smallest used WorkerId).
    host_worker: WorkerId,
    /// Cross-worker Push/Wait pairs (those that appear in a Push or
    /// Wait event), sorted by `(DataId, SeqTag)` -> SlotId. The
    /// `SeqTag` half disambiguates the per-destination fan-out pairs
    /// for one data symbol so multi-worker codegen (TASK-0117) can
    /// allocate distinct slots without racing.
    slot_ids: BTreeMap<(DataId, SeqTag), SlotId>,
    /// Per-pair tile carried on the originating XferPlaceholder. The
    /// tile names the iteration-axis slice this pair is responsible
    /// for. Used by host-side Wait codegen to slice-paste a
    /// per-worker partial buffer into the host's whole `output` (the
    /// gather half of TASK-0117 fan-out). Empty tile (no iteration
    /// nest) means "whole-array transfer"; the receiver-side
    /// `name = slot.wait();` path is taken.
    pair_tiles: BTreeMap<(DataId, SeqTag), compiler::event::IterTile>,
    /// `SyncTag` -> participants. Keyed directly by the contract
    /// barrier identity (TASK-0172). The projection clones the same
    /// participant set into every participant's `Event::Sync`, so
    /// recording the set the first time a tag is seen is exact; no
    /// uniform-barrier validation is needed (and a partial/non-uniform
    /// barrier is fine — every tag is independent).
    barrier_participants: BTreeMap<BarrierId, BTreeSet<WorkerId>>,
}

impl<'a> Plan<'a> {
    fn build(
        per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
        names: &'a NameTables,
        sidecar: &'a NameSidecar,
    ) -> Result<Self, EmitError> {
        let used_workers: Vec<WorkerId> = per_worker
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(w, _)| *w)
            .collect();

        // Host: the worker literally named "host", else the smallest
        // used WorkerId (matches the old Plan::build choice).
        let host_named = names
            .worker
            .iter()
            .find(|(_, n)| n.as_str() == "host")
            .map(|(w, _)| *w)
            .filter(|w| used_workers.contains(w));
        let host_worker = host_named
            .or_else(|| used_workers.first().copied())
            .ok_or_else(|| {
                EmitError::ContractGap(
                    "multi-worker emit requires at least one used worker".to_string(),
                )
            })?;

        // Cross-worker pairs: every (DataId, SeqTag) appearing on a
        // Push or Wait event (a Push and its matching Wait carry the
        // same `seq`, so seeing either is enough). Sorted by (DataId,
        // SeqTag) so the slot order is deterministic; ties on DataId
        // ascend by SeqTag. Examples with one pair per data symbol
        // (01..07: every cross-worker transfer is host↔single-worker)
        // map to one slot each at slot indices 0..N-1, preserving the
        // pre-TASK-0117 stable layout.
        let mut xfer_pairs: BTreeMap<(DataId, SeqTag), compiler::event::IterTile> =
            BTreeMap::new();
        for evs in per_worker.values() {
            collect_xfer_pairs(evs, &mut xfer_pairs);
        }
        let slot_ids: BTreeMap<(DataId, SeqTag), SlotId> = xfer_pairs
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();
        let pair_tiles = xfer_pairs;

        // Barrier identity by the contract-carried `SyncTag`
        // (TASK-0172). Each `Event::Sync` names its own barrier; the
        // projection clones the same participant set into every
        // participant's list, so the first sighting of a tag fixes its
        // participants. No pre-order-index heuristic and no
        // uniform-barrier validation: distinct tags are independent
        // barriers, so a partial/non-uniform barrier lowers correctly.
        let mut barrier_participants: BTreeMap<BarrierId, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            host_worker,
            slot_ids,
            pair_tiles,
            barrier_participants,
        })
    }

    fn emit(&self) -> Result<String, EmitError> {
        let mut out = String::new();
        writeln!(
            out,
            "//! Generated by the pthreads-sync backend (TASK-0122, multi-worker)."
        )
        .ok();
        writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
        writeln!(out).ok();
        writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
        writeln!(out, "mod kernels;").ok();
        writeln!(out).ok();
        writeln!(out, "use std::sync::{{Arc, Barrier, Condvar, Mutex}};").ok();
        writeln!(out, "use std::thread;").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "/// One-shot typed rendezvous slot. Producer calls `push(v)`; the"
        )
        .ok();
        writeln!(
            out,
            "/// consumer's `wait()` blocks until a value is available, then takes it."
        )
        .ok();
        writeln!(
            out,
            "/// Reusable across iterations: after `wait()` consumes the value the"
        )
        .ok();
        writeln!(
            out,
            "/// slot is empty again and a subsequent `push` reuses it."
        )
        .ok();
        writeln!(out, "struct Slot<T> {{ mu: Mutex<Option<T>>, cv: Condvar }}").ok();
        writeln!(out, "impl<T> Slot<T> {{").ok();
        writeln!(
            out,
            "    fn new() -> Self {{ Slot {{ mu: Mutex::new(None), cv: Condvar::new() }} }}"
        )
        .ok();
        writeln!(out, "    fn push(&self, v: T) {{").ok();
        writeln!(out, "        let mut g = self.mu.lock().unwrap();").ok();
        writeln!(out, "        *g = Some(v);").ok();
        writeln!(out, "        self.cv.notify_one();").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    fn wait(&self) -> T {{").ok();
        writeln!(out, "        let mut g = self.mu.lock().unwrap();").ok();
        writeln!(
            out,
            "        while g.is_none() {{ g = self.cv.wait(g).unwrap(); }}"
        )
        .ok();
        writeln!(out, "        g.take().unwrap()").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();

        // TASK-0052.05: file-scope items for every Count `check loop`
        // in the program. The codegen helpers `collect_count_check_
        // frames` / `emit_count_reporter_struct` / `sanitize_loop_var`
        // are shared verbatim with the single-worker pthreads-sync
        // path (see `lib.rs`) and mp-tcp-bufsync — same emit-string
        // shape across all three sites, by construction.
        //
        // **Multi-worker Count semantic (shared static across threads).**
        // `partition=workers` projects the SAME source loop onto every
        // participating worker; `inject_check_frames` attaches a frame
        // to each projected `Event::Loop`, so the SAME (loop_var,
        // threshold, ViolationKind::Count) appears in N workers'
        // event lists. All N workers must `fetch_add` into the SAME
        // `AtomicU64` (PRD §6.3.5 "tallied across the program run":
        // ONE summary line, not N), so we emit ONE static per UNIQUE
        // sanitized ident — keyed by `ident`, not by worker. The
        // `AtomicU64` lives at file scope and is reachable from every
        // thread without closure capture; threads coordinate by
        // `fetch_add(_, Relaxed)` on the same global. The Drop guard
        // local (emitted inside `fn main` below) is OWNED BY THE HOST
        // THREAD and prints ONE aggregate stderr summary at fn main
        // exit (after `join()` on every worker handle — see the join
        // site at the end of `fn main`), which is the correct semantic
        // for "tallied across the program run".
        let count_frames = collect_unique_count_check_frames(self.per_worker);
        if !count_frames.is_empty() {
            emit_count_reporter_struct(&mut out);
            for cf in &count_frames {
                writeln!(
                    out,
                    "static NUC_CHECK_COUNT_{ident}: std::sync::atomic::AtomicU64 = \
                     std::sync::atomic::AtomicU64::new(0);",
                    ident = cf.ident,
                )
                .ok();
            }
            writeln!(out).ok();
        }

        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        // TASK-0052.05: per-Count-loop Drop guard local. Mirrors the
        // single-worker emit in `lib.rs::render_main_rs`. The guard's
        // Drop runs at fn main exit (after every `handle.join()`
        // below has returned, since handles are dropped LIFO in
        // reverse insertion order: the guards declared HERE outlive
        // the handles declared below). Each unique sanitized ident
        // gets ONE guard, matching the single static.
        for cf in &count_frames {
            writeln!(
                out,
                "    let _nuc_check_reporter_{ident} = NucCheckCountReporter {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20counter: &NUC_CHECK_COUNT_{ident},\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20loop_var: \"{loop_var}\",\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20threshold_ns: {ns},\n\
                 \x20\x20\x20\x20}};",
                ident = cf.ident,
                loop_var = cf.loop_var,
                ns = cf.latency_max_ns,
            )
            .ok();
        }
        if !count_frames.is_empty() {
            writeln!(out).ok();
        }

        // NOTE: this production codegen path is intentionally free of
        // any test-only nondeterminism branch. The determinism-check
        // -negative gate's perturbation (formerly an inline
        // NUC_NONDET_TEST per-process nonce here, TASK-0145) was
        // relocated harness-side in TASK-0157: the e2e determinism
        // harness post-processes ONE of its two emitted trees when
        // NUC_NONDET_TEST=1. See nucleus/e2e/src/main.rs
        // `maybe_perturb_for_nondet_test`. Do NOT reintroduce an env
        // read or self-corruption branch on the codegen critical path.

        // ---- Allocate slots (sorted (DataId, SeqTag) order). ----
        // One slot per Push/Wait pair, so multi-worker fan-out
        // (TASK-0117) doesn't race; an example with one pair per data
        // symbol degrades to the pre-TASK-0117 one-slot-per-data
        // layout, byte-identical to the old emit.
        for ((data_id, _seq), slot_id) in &self.slot_ids {
            let name = self.data_name(*data_id)?;
            let ty = self.sidecar.data_type(*data_id).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "cross-worker data `{name}` ({data_id:?}) has no ResolvedType \
                     in the NameSidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            writeln!(
                out,
                "    let slot_{slot_id}: Arc<Slot<{rty}>> = Arc::new(Slot::new()); // data `{name}`",
            )
            .ok();
        }

        // ---- Allocate barriers (SyncTag order). ----
        // Iterating the BTreeMap gives ascending SyncTag order, which
        // for a uniform-barrier program is the same 0,1,2,… the old
        // pre-order-index scheme produced (deterministic tag
        // assignment in `inject_syncs`) — generated code byte-identical.
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
            let used_slots = self.slots_used_by(*w);
            let used_barriers = self.barriers_used_by(*w);

            for slot_id in &used_slots {
                writeln!(
                    out,
                    "    let {wname}_slot_{slot_id} = Arc::clone(&slot_{slot_id});"
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

        // ---- Host body (bare slot/bar names, no closure prefix). ----
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

    /// SlotIds a worker touches (it Pushes or Waits the data).
    fn slots_used_by(&self, w: WorkerId) -> Vec<SlotId> {
        let mut s: BTreeSet<SlotId> = BTreeSet::new();
        collect_worker_slots(&self.per_worker[&w], &self.slot_ids, &mut s);
        s.into_iter().collect()
    }

    /// The barrier `SyncTag`s a worker participates in (ascending
    /// tag order — `SyncTag: Ord`).
    fn barriers_used_by(&self, w: WorkerId) -> Vec<BarrierId> {
        let mut out: Vec<BarrierId> = self
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

    /// Render one worker's body: pre-init, then the EventList walk.
    /// `prefix` is `""` for the host (bare top-level slot/bar names)
    /// or `<wname>_` for a spawned worker (closure-captured clones).
    fn render_worker_body(
        &self,
        worker: WorkerId,
        base_indent: usize,
        prefix: &str,
    ) -> Result<String, EmitError> {
        let mut out = String::new();
        let pad = "    ".repeat(base_indent);
        let evs = &self.per_worker[&worker];

        // Pre-init: data this worker WAITs on (cross-worker input,
        // overwritten by slot.wait()) OR writes via an indexed Fire
        // output and never whole-array. Sorted by name. With explicit
        // `: <ty>` annotation (matches the old multi-worker emitter,
        // which differs from the single-worker emitter here).
        let pre_init = self.collect_pre_init(worker)?;
        for (name, did) in &pre_init {
            let ty = self.sidecar.data_type(*did).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "pre-init data `{name}` ({did:?}) has no ResolvedType in sidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            let init = render_array_init(ty);
            writeln!(out, "{pad}let mut {name}: {rty} = {init};").ok();
        }
        if !pre_init.is_empty() {
            writeln!(out).ok();
        }

        let ctx = RenderCtxPub::new(self.names, self.sidecar);
        self.render_worker_events(worker, evs, &mut out, base_indent, prefix, &ctx)?;
        Ok(out)
    }

    /// Walk a worker's EventList. Barrier identity comes from each
    /// `Event::Sync`'s contract-carried `SyncTag` (TASK-0172) — no
    /// running pre-order counter is needed any more.
    ///
    /// `worker` is the [`WorkerId`] of the scope being rendered (the
    /// same one `render_worker_body` was called with — threaded
    /// verbatim through recursion into nested `Event::Loop` bodies).
    /// It is consulted at the `Event::Loop` site to apply the
    /// per-worker partition range from
    /// [`NameSidecar::partition_worker_ranges`] (TASK-0212), so each
    /// participating worker emits `for n in (lo)..(hi)` over its own
    /// exclusive slice rather than the shared source range.
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
                    let args = crate::render_fire_args_pub(*kernel, &bindings.inputs, ctx)?;
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
                            // TASK-0209: shared scalar-vs-sub-array
                            // classifier; the pthreads-sync multi-
                            // worker path and mp-tcp-bufsync both
                            // route through `render_fire_output_
                            // assign_pub` so the three Fire-output
                            // sites cannot drift.
                            let rhs = format!("kernels::{callee}({args})");
                            let stmt = crate::render_fire_output_assign_pub(o, &rhs, ctx)?;
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
                    // A `block_tag` here means a strip-mined inner loop
                    // in a *multi*-worker schedule. That needs the same
                    // per-occurrence absolute-index rebinding the
                    // single-worker path does; this multi-worker
                    // renderer does NOT yet thread it. No tier-1
                    // schedule is a blocked multi-worker schedule, so
                    // this is unreachable today — but fail LOUD with
                    // context (typed error, never silently emit the
                    // un-rebound loop, which would double-count an
                    // accumulator exactly like the TASK-0180 bug).
                    if block_tag.is_some() {
                        return Err(EmitError::ContractGap(format!(
                            "Event::Loop for iter var `{var}` carries a strip-mine \
                             block_tag inside a MULTI-worker schedule; per-occurrence \
                             absolute-index rebinding is implemented only on the \
                             shared single-worker path (TASK-0180). No tier-1 schedule \
                             blocks a multi-worker loop; refusing to emit un-rebound \
                             (would double-count). Tracked as TASK-0181."
                        )));
                    }
                    // TASK-0052.05: real-time `check loop V :
                    // latency_max=T` codegen on the multi-worker path.
                    // The shape MIRRORS the single-worker path in
                    // `lib.rs::render_event` (Event::Loop arm) — same
                    // `Instant::now()` + `_check_elapsed` + dispatch on
                    // `on_violation`. The per-thread semantics:
                    //   * Panic: each worker thread panics
                    //     independently; the `handle.join().expect(..)`
                    //     at fn main propagates the panic, exit code 101.
                    //   * Log: each worker thread's `eprintln!` is
                    //     atomic per call; multiple threads' lines may
                    //     interleave on stderr — that is the intended
                    //     behaviour (the cross-backend differential
                    //     compares stdout only, per PRD §10.1).
                    //   * Count: ALL threads `fetch_add(_, Relaxed)`
                    //     into the SAME file-scope static
                    //     `NUC_CHECK_COUNT_<ident>` (emitted once per
                    //     unique ident in `Plan::emit`, NOT per worker).
                    //     The Drop guard on the host thread aggregates
                    //     all threads' contributions into ONE summary
                    //     line at fn main exit — PRD §6.3.5 "tallied".
                    // Strip-mined invariant: `inject_check_frames`
                    // populates check_frame only on outer source loops
                    // (block_tag == None); the `block_tag.is_some()`
                    // arm above already rejected, so reaching here with
                    // a check_frame implies block_tag.is_none() — the
                    // invariant the single-worker path also defends.
                    if check_frame.is_some() && block_tag.is_some() {
                        return Err(EmitError::ContractGap(format!(
                            "Event::Loop for iter var `{var}` carries BOTH a check_frame \
                             and a block_tag — `inject_check_frames` is contracted to \
                             populate check_frame only on outer source loops; this is a \
                             projection-layer bug (TASK-0052.05 multi-worker invariant)."
                        )));
                    }
                    // Per-worker partition override (TASK-0212): if the
                    // partition pass recorded a slice for THIS worker on
                    // this iter var, render the concrete literal range.
                    // The symbolic `loop_bounds` entry names the SOURCE
                    // range, not the partitioned slice, so it is the
                    // wrong rendering for a partitioned worker. A worker
                    // not listed in the per-iter-var map (e.g. host,
                    // which doesn't participate in partition=workers)
                    // falls through to the source-form symbolic /
                    // literal precedence exactly as before TASK-0212.
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
                        // TASK-0052.05 — mirrors `lib.rs::render_event`
                        // Event::Loop check_frame arm verbatim (same
                        // emit strings, same `_check_start` /
                        // `_check_elapsed` locals, same dispatch).
                        writeln!(
                            out,
                            "{body_pad}let _check_start = std::time::Instant::now();"
                        )
                        .ok();
                        self.render_worker_events(
                            worker,
                            body,
                            out,
                            body_indent,
                            prefix,
                            ctx,
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
                                writeln!(
                                    out,
                                    "{body_pad}if _check_elapsed > {ns}_u128 {{ eprintln!(\"warning: check loop `{lv}` violated latency_max={ns} ns: iteration took {{}} ns\", _check_elapsed); }}",
                                    ns = frame.latency_max_ns,
                                    lv = frame.loop_var,
                                )
                                .ok();
                            }
                            ViolationKind::Count => {
                                // SHARED file-scope static (one per
                                // unique sanitized ident; emitted by
                                // `Plan::emit`). All worker threads
                                // `fetch_add(Relaxed)` into the same
                                // global. Relaxed is sufficient: the
                                // host-thread Drop guard's
                                // `load(Relaxed)` runs AFTER every
                                // `handle.join()` (synchronises-with
                                // its panicking-or-clean termination),
                                // so all preceding fetch_adds happen-
                                // before the load on the host thread.
                                let id = sanitize_loop_var(&frame.loop_var);
                                writeln!(
                                    out,
                                    "{body_pad}if _check_elapsed > {ns}_u128 {{ NUC_CHECK_COUNT_{id}.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }}",
                                    ns = frame.latency_max_ns,
                                    id = id,
                                )
                                .ok();
                            }
                        }
                    } else {
                        self.render_worker_events(worker, body, out, indent + 1, prefix, ctx)?;
                    }
                    writeln!(out, "{pad}}}").ok();
                }
                Event::Sync { sync, .. } => {
                    // Barrier identity is the contract-carried SyncTag
                    // (TASK-0172): every participant of this barrier
                    // carries the same tag, so all participants
                    // .wait() on the same `bar_<tag>` with no
                    // pre-order-index recovery.
                    let bid = sync.0;
                    writeln!(out, "{pad}{prefix}bar_{bid}.wait();").ok();
                }
                Event::Push { data, dst, seq, .. } => {
                    let sid = self.slot_ids.get(&(*data, *seq)).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Push of data {data:?} (seq {seq:?}) has no slot id \
                             (not collected as cross-worker)"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let to = self.worker_name(*dst);
                    writeln!(
                        out,
                        "{pad}{prefix}slot_{sid}.push({name}.clone()); // send `{name}` to {to}",
                    )
                    .ok();
                }
                Event::Wait {
                    data, src, seq, ..
                } => {
                    let sid = self.slot_ids.get(&(*data, *seq)).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Wait of data {data:?} (seq {seq:?}) has no slot id \
                             (not collected as cross-worker)"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let from = self.worker_name(*src);
                    // TASK-0117 host-side gather: when the pair's tile
                    // names a strict sub-slice of the data's leading
                    // axis, the producer worker pushed its whole local
                    // buffer with only its slice populated; the host
                    // must slice-paste that slice into its own whole
                    // `name` buffer (so the union of 4 workers'
                    // contributions covers the source range). When the
                    // tile is empty or covers the source range, fall
                    // back to the pre-TASK-0117 whole-array assign.
                    let assign = self.render_wait_assign(
                        &name,
                        *data,
                        *seq,
                        &format!("{prefix}slot_{sid}.wait()"),
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

    /// Pre-init set for a worker: cross-worker inputs it Waits on +
    /// data it writes via an indexed Fire output and never
    /// whole-array. Sorted by name.
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

    /// Render the receiver-side assignment statement for one Wait
    /// event. Returns one statement (no trailing newline).
    ///
    /// Two shapes:
    /// - **Whole-array assign** (`name = <rhs>;`) — the pre-TASK-0117
    ///   single-pair behaviour. Selected when the pair's tile is
    ///   empty (no enclosing iteration nest, e.g. a top-level
    ///   load_input ⇒ host transfer), OR when the leading axis of
    ///   the tile covers the data's full leading-axis range (i.e. the
    ///   producer sent the whole array on this pair).
    /// - **Slice-paste** (`{ let _tmp = <rhs>; name[lo..hi]
    ///   .copy_from_slice(&_tmp[lo..hi]); }`) — TASK-0117 host-side
    ///   gather. Selected when the tile's outer axis is a strict
    ///   sub-range of the data's leading axis. The producer pushed
    ///   its whole local buffer with only its tile-slice populated;
    ///   the receiver copies that slice into its own whole buffer.
    ///   The byte/element stride per outer-axis element is the
    ///   product of the data's inner dims.
    ///
    /// Why receiver-side slice-paste (not producer-side slice-push):
    /// the producer-worker's local `name` is pre-initialised to the
    /// full-shape Vec (collect_pre_init logic) with only its tile
    /// positions filled; pushing the whole Vec keeps the producer
    /// codegen identical to the single-worker case. Slicing on the
    /// receiver concentrates the new logic at one site and keeps the
    /// 1:1 transfers (examples 01..07) byte-identical to before
    /// because their pair tiles are empty.
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
            None => {
                // Empty tile (or no shape match) — whole-array
                // assign. The pre-TASK-0117 contract; the 1:1 cases
                // (examples 01..07) take this path.
                Ok(format!("{name} = {rhs};"))
            }
            Some(LeadingAxis { lo, hi, stride }) => {
                // Slice-paste: the receiver-side gather half of
                // TASK-0117. Element offsets are `lo*stride..hi*stride`.
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

    /// Compute the leading-axis slice for a Wait's tile, returning
    /// `Some(LeadingAxis { lo, hi, stride })` when the tile's outer
    /// axis is a strict sub-range of the data's leading axis (the
    /// slice-paste path), `None` when the tile is empty or the outer
    /// axis covers the full source range (the whole-array path).
    ///
    /// Returns `Err` on a shape mismatch — e.g. the tile's leading
    /// axis range exceeds the data's leading-dim length; that is a
    /// compiler-pass invariant violation worth failing loud rather
    /// than silently emitting an out-of-bounds slice.
    ///
    /// **HONEST-PARTIAL ASSUMPTION (TASK-0117 cycle-1 review-gate
    /// finding):** this gather code assumes `tile.bounds[0].iter_var`
    /// maps to the DATA's leading dim (axis 0). The `_iv` is
    /// currently not consulted — only the numerical range is
    /// validated. For the in-tree `partition=workers` schedules
    /// (example 13's `loop n : partition=workers` on rank-4 data
    /// `f32[B][C][H][W]` where `n` IS the leading-axis index), this
    /// holds. For a hypothetical inner-axis partition
    /// (`loop k : partition=workers` on `data D : i32[B][K]` where
    /// `k` is axis 1, NOT axis 0), the slice would silently address
    /// the WRONG axis. The numerical bounds-check guards against
    /// out-of-range slices but NOT against wrong-axis selection.
    /// Tracked as a honest-limit in TASK-0117 final notes (§ "Honest
    /// limits / not-tested-this-cycle"); a future cycle that exercises
    /// inner-axis partition must extend this helper to consult
    /// `_iv → axis` via the data-type metadata. Failing loud with a
    /// dedicated EmitError variant would be preferable to silent
    /// wrong-axis slicing; left as a follow-up.
    fn leading_axis_slice(
        &self,
        data: DataId,
        tile: &compiler::event::IterTile,
    ) -> Result<Option<LeadingAxis>, EmitError> {
        // Empty tile ⇒ no per-axis slicing.
        let Some((_iv, range)) = tile.bounds.first() else {
            return Ok(None);
        };
        let ty = self.sidecar.data_type(data).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "Wait of data {data:?} has no ResolvedType in NameSidecar"
            ))
        })?;
        // Scalar data: no slice axes — whole-value transfer.
        if ty.dims.is_empty() {
            return Ok(None);
        }
        let leading_dim = ty.dims[0] as i64;
        // Pre-TASK-0117 single-pair: tile covers the full source
        // range of the leading axis (0..B). No slicing.
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
/// (TASK-0117).
struct LeadingAxis {
    lo: usize,
    hi: usize,
    /// Product of the data type's inner dims; the per-outer-axis
    /// stride in flat-Vec elements.
    stride: usize,
}

// --------------------------------------------------------------------
// Event-walk helpers (recurse into Event::Loop bodies)
// --------------------------------------------------------------------

/// Collect every (DataId, SeqTag) pair appearing on a Push or Wait
/// event in `events` (descending into Loop bodies). The map's value is
/// the pair's tile, copied from the first event sighting; the same
/// `seq` is carried on both endpoints (Push and Wait) by the
/// XferPlaceholder construction (TASK-0018) so first-sighting is
/// well-defined. The tile is retained for later host-side slice-paste
/// codegen (TASK-0117 gather).
fn collect_xfer_pairs(
    events: &[Event],
    out: &mut BTreeMap<(DataId, SeqTag), compiler::event::IterTile>,
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

fn collect_worker_slots(
    events: &[Event],
    slot_ids: &BTreeMap<(DataId, SeqTag), SlotId>,
    out: &mut BTreeSet<SlotId>,
) {
    for e in events {
        match e {
            Event::Push { data, seq, .. } | Event::Wait { data, seq, .. } => {
                if let Some(s) = slot_ids.get(&(*data, *seq)) {
                    out.insert(*s);
                }
            }
            Event::Loop { body, .. } => collect_worker_slots(body, slot_ids, out),
            _ => {}
        }
    }
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier, so there is
/// nothing to validate / reject here any more).
fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
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


// --------------------------------------------------------------------
// Type rendering helpers (sidecar-driven; no AlgoIR)
// --------------------------------------------------------------------

/// Map a `ResolvedType` to the Rust surface type: arrays flatten to
/// `Vec<T>`, scalars use the natural spelling. Mirrors the old
/// `rust_type_of`.
fn rust_type_of(ty: &compiler::algo::ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

fn render_array_init(ty: &compiler::algo::ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
    }
}

fn rust_scalar_zero(t: &compiler::algo::ScalarType) -> &'static str {
    use compiler::algo::ScalarType::*;
    match t {
        F32 | F64 => "0.0",
        Bool => "false",
        _ => "0",
    }
}

// Expression / index / bound rendering is delegated entirely to the
// crate-level shared shims (`render_fire_args_pub`,
// `render_flat_index_pub`, `render_const_expr_pub`) so there is ONE
// implementation shared with the single-worker emitter and the two
// paths cannot byte-drift. This module therefore does not import the
// lower-level `bin_op_str` / `render_int_expr_pub` directly.

// --------------------------------------------------------------------
// TASK-0052.05: Count check_frame collection across workers
// --------------------------------------------------------------------

/// Collect every Count `check loop` frame from every worker's
/// EventList and DEDUPLICATE by sanitized ident. Why dedup is needed:
/// when a `check loop V` directive lands on a loop var whose loop is
/// projected onto N workers (`partition=workers`), the
/// `inject_check_frames` pass attaches the SAME `CheckFrame` to every
/// participating worker's `Event::Loop`. The single-worker helper
/// `collect_count_check_frames` walks one worker's events and never
/// sees duplicates; here we must aggregate across workers and emit
/// ONE static + ONE Drop guard per UNIQUE ident (the static is shared
/// across threads — see `Plan::emit` comment on Count semantics).
///
/// Determinism: the result preserves the first sighting of each
/// ident in worker-iteration order (BTreeMap is sorted by WorkerId);
/// within a worker, events are walked in their EventList order. Two
/// identical CheckFrames (same loop_var + same threshold +
/// `ViolationKind::Count`) sanitize to the same ident, so the dedup
/// is well-defined. Two DIFFERENT directives that sanitize to the
/// same ident — only reachable today via grammar-extension to non-
/// ASCII loop names — would silently collide; the grammar rejects
/// those at parse, see `sanitize_loop_var`'s docstring.
fn collect_unique_count_check_frames(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
) -> Vec<CountCheckLoop> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<CountCheckLoop> = Vec::new();
    for evs in per_worker.values() {
        for cf in collect_count_check_frames(evs) {
            if seen.insert(cf.ident.clone()) {
                out.push(cf);
            }
        }
    }
    out
}
