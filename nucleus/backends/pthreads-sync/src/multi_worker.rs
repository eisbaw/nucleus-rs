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
use compiler::event::{DataId, Event, SeqTag, SyncTag, WorkerId};
use compiler::sidecar::NameSidecar;

use backend_common::multi_worker_walker::{
    self as walker, RendezvousId, WalkerCtx,
};
use backend_common::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static, CountCheckLoop,
};
use backend_common::render::{rust_scalar_type, EmitError};
use crate::NameTables;

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
/// `SlotId` is the shared rendezvous-id alias from
/// [`multi_worker_walker`] — both backends use `usize` keyed by
/// `(DataId, SeqTag)`. Kept as a local alias so the rest of this
/// module's local nomenclature ("slot") stays unchanged.
type SlotId = RendezvousId;
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
            walker::collect_xfer_pairs(evs, &mut xfer_pairs);
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
            walker::collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
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
        // Backend-agnostic header (TASK-0231): pthreads-sync multi-worker
        // and pthreads-async multi-worker emit byte-identical headers so
        // the provenance reads truthfully regardless of which backend
        // produced it; the substrate (Slot vs Ring) is the load-bearing
        // difference, not the header.
        writeln!(out, "//! Generated by the nucleus pre-compiler.").ok();
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
        writeln!(
            out,
            "        let mut g = self.mu.lock().expect(\"slot mutex poisoned — producer panicked before notify\");"
        )
        .ok();
        writeln!(out, "        *g = Some(v);").ok();
        writeln!(out, "        self.cv.notify_one();").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    fn wait(&self) -> T {{").ok();
        writeln!(
            out,
            "        let mut g = self.mu.lock().expect(\"slot mutex poisoned — producer panicked before notify\");"
        )
        .ok();
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
                // TASK-0222: shared template — see emit_count_static.
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

        // TASK-0052.05: per-Count-loop Drop guard local. Mirrors the
        // single-worker emit in `lib.rs::render_main_rs`. The guard's
        // Drop runs at fn main exit (after every `handle.join()`
        // below has returned, since handles are dropped LIFO in
        // reverse insertion order: the guards declared HERE outlive
        // the handles declared below). Each unique sanitized ident
        // gets ONE guard, matching the single static.
        for cf in &count_frames {
            // TASK-0222: shared template — see emit_count_guard_local.
            emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
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
        walker::collect_worker_rendezvous(&self.per_worker[&w], &self.slot_ids, &mut s);
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

        // Dispatch through the shared walker (TASK-0239) — the
        // rendezvous prefix `"slot"` is the only knob distinguishing
        // pthreads-sync's emit from pthreads-async's.
        let walker_ctx = WalkerCtx {
            names: self.names,
            sidecar: self.sidecar,
            rendezvous_prefix: "slot",
            rendezvous_ids: &self.slot_ids,
            pair_tiles: &self.pair_tiles,
        };
        walker::render_worker_events(&walker_ctx, worker, evs, &mut out, base_indent, prefix)?;
        Ok(out)
    }


    /// Pre-init set for a worker: cross-worker inputs it Waits on +
    /// data it writes via an indexed Fire output and never
    /// whole-array. Sorted by name.
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
// Event-walk helpers extracted to `multi_worker_walker` (TASK-0239).
//
// The shared helpers — `collect_xfer_pairs`, `collect_worker_rendezvous`,
// `collect_barriers_by_tag`, `collect_pre_init_sets`, the event walker
// `render_worker_events`, the Wait gather `render_wait_assign`, the
// `LeadingAxis` slice computation — now live in `multi_worker_walker`
// so pthreads-async can call them directly. This module retains only
// the per-backend `Plan` shape (`Slot<T>` substrate, one-shot
// rendezvous semantics) plus the `Plan::emit` orchestration above.


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
