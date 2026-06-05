//! Multi-worker codegen for the openmp-rs backend
//! (TASK-0044.01.01 cycle 196, twin of `pthreads-sync/src/multi_worker.rs`).
//!
//! Lowers the per-worker [`Event`] lists of a ≥2-worker schedule into
//! a standalone Rust program that wraps the per-non-host-worker spawn
//! loop in a `rayon::scope(|s| { ... })` and uses `s.spawn(move |_| {
//! body })` per non-host worker (instead of `std::thread::spawn` +
//! explicit `handle.join()`). The cross-worker rendezvous primitive
//! (per-pair `Slot<T> = Mutex<Option<T>> + Condvar`) and per-barrier
//! `Arc<Barrier>` carry over verbatim from pthreads-sync's
//! multi_worker.rs — those are `std::sync` primitives and work
//! identically inside a rayon::scope, so the substrate swap is
//! confined to the spawn site.
//!
//! ## Why rayon::scope (Option A) over rayon::join (Option B)
//!
//! `rayon::scope(|s| s.spawn(move |_| body))` accepts an N-way fan-out
//! cleanly and provides implicit join at scope-end (no explicit
//! `handle.join()` loop). `rayon::join` would force a recursive
//! two-way fanout for N>2 workers, which is significantly more
//! codegen refactoring for no semantic gain (rayon::scope's
//! work-stealing scheduler is the same one rayon::join uses
//! underneath). Option A keeps the diff vs pthreads-sync minimal and
//! the emitted code legible — see TASK-0044.01.01 cycle-196
//! implementation plan.
//!
//! ## AlgoIR-/LinkedIR-free (TASK-0124 AC#2)
//!
//! This module reads ONLY the per-worker `EventList`, the
//! [`NameTables`], and the [`NameSidecar`] — same contract as
//! pthreads-sync's multi-worker emitter. Codegen primitives
//! (`Slot<T>` allocation, barrier identity by `SyncTag`,
//! `slot_ids` / `barrier_participants` derivation, the shared
//! `multi_worker_walker::render_worker_events` walk) are consumed
//! from `backend_common` directly so the two backends cannot
//! byte-drift through divergent helper code.
//!
//! ## Bit-identical OUTPUT (output.bin vs reference.bin)
//!
//! For every schedule pthreads-sync handles in multi-worker, openmp-rs
//! must produce bit-identical output.bin — that's the cross-backend
//! differential gate. The runtime substrates (std::thread vs
//! rayon::scope) are structurally equivalent under the schedule's
//! sync contract (every cross-worker Push/Wait pair carries its own
//! Slot; every barrier carries its own `Arc<Barrier>`), so the
//! observable output is independent of the spawn primitive. The
//! generated source files are NOT byte-identical (rayon::scope vs
//! thread::spawn + Cargo.toml `rayon = "1"` dep) — only the runtime
//! output is.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use nucleus_compiler::event::{DataId, Event, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use crate::NameTables;
use backend_common::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static, CountCheckLoop,
};
use backend_common::elect_host_from_worker_names;
use backend_common::multi_worker_walker::{self as walker, RendezvousId, WalkerCtx};
use backend_common::render::{render_array_init_for_combine, rust_scalar_type, EmitError};

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Render the contents of `main.rs` for a multi-worker schedule from
/// the per-worker EventList + name tables + sidecar. Twin of
/// `pthreads_sync::multi_worker::render_main_rs_multi`; the emit
/// differs only in the spawn-site substrate (`rayon::scope` +
/// `s.spawn(move |_| ...)` instead of `thread::spawn` + `handle.join()`).
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
/// Push/Wait pair. Same `(DataId, SeqTag)` keying as pthreads-sync:
/// shared codegen primitives in `backend_common::multi_worker_walker`
/// enforce identical slot indexing across both backends, so a
/// twin schedule emits the same `slot_N` identifiers.
type SlotId = RendezvousId;
/// Stable identifier for one barrier — the contract-carried
/// [`SyncTag`] (TASK-0172). Same value as pthreads-sync.
type BarrierId = SyncTag;

struct Plan<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    used_workers: Vec<WorkerId>,
    host_worker: WorkerId,
    slot_ids: BTreeMap<(DataId, SeqTag), SlotId>,
    pair_tiles: BTreeMap<(DataId, SeqTag), nucleus_compiler::event::IterTile>,
    accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
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

        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule (TASK-0336 cycle 164 lift). openmp-rs uses
        // the EXACT SAME rule as pthreads-sync so the host-election
        // mirror invariant (driver vs backend) holds trivially —
        // neither side overrides the helper.
        let host_worker =
            elect_host_from_worker_names(&names.worker, &used_workers).ok_or_else(|| {
                EmitError::ContractGap(
                    "multi-worker emit requires at least one used worker".to_string(),
                )
            })?;

        // Cross-worker pairs: every (DataId, SeqTag) appearing on a
        // Push or Wait event. Sorted by (DataId, SeqTag) so slot
        // indexing matches pthreads-sync's bit-for-bit.
        let pair_tiles: BTreeMap<(DataId, SeqTag), nucleus_compiler::event::IterTile> =
            walker::collect_pair_tiles(per_worker.values());
        let slot_ids: BTreeMap<(DataId, SeqTag), SlotId> = pair_tiles
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();

        // Barrier identity by the contract-carried `SyncTag`
        // (TASK-0172). Same derivation as pthreads-sync.
        let mut barrier_participants: BTreeMap<BarrierId, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            walker::collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        // Per-worker overlapping-write accumulator classification
        // (TASK-0343 cycle 189). Same derivation as pthreads-sync.
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
            slot_ids,
            pair_tiles,
            barrier_participants,
            accumulate_waits,
        })
    }

    fn emit(&self) -> Result<String, EmitError> {
        let mut out = String::new();
        // Backend-agnostic header — same neutral form pthreads-sync /
        // pthreads-async emit. The substrate-difference (rayon::scope
        // vs thread::spawn) is the load-bearing distinction in the
        // body, not the header.
        writeln!(out, "//! Generated by the nucleus pre-compiler.").ok();
        writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
        writeln!(out).ok();
        writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
        writeln!(out, "mod kernels;").ok();
        writeln!(out).ok();
        // `Arc<Barrier>` + `Mutex<Option<T>>` + `Condvar` are std::sync
        // primitives and compose with rayon::scope unchanged. `thread`
        // is intentionally NOT imported — the spawn site uses
        // `rayon::scope` + `s.spawn(...)` (the substrate swap from
        // pthreads-sync). The emitted Cargo.toml declares
        // `rayon = "1"` (see `openmp_rs::emit`).
        writeln!(out, "use std::sync::{{Arc, Barrier, Condvar, Mutex}};").ok();
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
        writeln!(
            out,
            "struct Slot<T> {{ mu: Mutex<Option<T>>, cv: Condvar }}"
        )
        .ok();
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
        // in the program — same shape as pthreads-sync (shared
        // emitter via `backend_common::check_frame`). Count semantics
        // identical: one AtomicU64 per unique sanitized ident,
        // fetch_add'd across all workers, ONE Drop guard owned by the
        // host thread prints the aggregate summary at fn main exit.
        // Under rayon::scope the worker closures complete before the
        // scope-end implicit join releases control to the host body's
        // tail, which is the same lifecycle pthreads-sync's
        // handle.join() loop establishes.
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

        // TASK-0052.05: per-Count-loop Drop guard local. Mirrors the
        // pthreads-sync emit; declared BEFORE the rayon::scope so the
        // guard outlives the scope (Drop runs at fn main exit, after
        // scope-end implicit join).
        for cf in &count_frames {
            emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
        }
        if !count_frames.is_empty() {
            writeln!(out).ok();
        }

        // NOTE: this codegen path is intentionally free of any
        // test-only nondeterminism branch. Same protocol as
        // pthreads-sync (TASK-0157): the determinism-check-negative
        // gate perturbation lives harness-side in
        // nucleus/e2e/src/main.rs `maybe_perturb_for_nondet_test`.

        // ---- Allocate slots (sorted (DataId, SeqTag) order). ----
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

        // ---- Spawn non-host workers inside a rayon::scope. ----
        //
        // The substrate swap vs pthreads-sync:
        //   pthreads-sync:  let h = thread::spawn(move || { body }); handles.push(h);  ... handle.join();
        //   openmp-rs:      rayon::scope(|s| { s.spawn(move |_| { body }); ... });
        //
        // rayon::scope provides implicit join at scope-end — control
        // returns to the host-thread body only after every spawned
        // closure has returned. The host body runs INSIDE the scope
        // so it sees the same Arc-clones the spawned closures do.
        writeln!(out, "    rayon::scope(|s| {{").ok();

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
                    "        let {wname}_slot_{slot_id} = Arc::clone(&slot_{slot_id});"
                )
                .ok();
            }
            for tag in &used_barriers {
                let bid = tag.0;
                writeln!(
                    out,
                    "        let {wname}_bar_{bid} = Arc::clone(&bar_{bid});"
                )
                .ok();
            }
            // `move |_|` — rayon::Scope::spawn passes a &Scope to the
            // closure; we don't need it (no nested spawns), so bind
            // to `_`. Mirrors the canonical rayon::scope idiom.
            writeln!(out, "        s.spawn(move |_| {{").ok();
            // base_indent=3: rayon::scope wrap (+1) + s.spawn closure
            // (+1) on top of pthreads-sync's `fn main` (+1) + spawn
            // closure (+1) = 4. Wait, pthreads-sync uses 2 for the
            // worker body. We need 3 here because there's an extra
            // nesting level (the rayon::scope wrapper).
            let body = self.render_worker_body(*w, 3, &format!("{wname}_"))?;
            out.push_str(&body);
            writeln!(out, "        }});").ok();
            writeln!(out).ok();
        }

        // ---- Host body INSIDE rayon::scope. ----
        //
        // Host runs as the calling thread of rayon::scope (it is NOT
        // a spawned task; rayon::scope runs the calling thread's
        // body once and joins at scope-end). The bare slot/bar names
        // (no closure-clone prefix) work because we're inside the
        // closure passed to rayon::scope, where the outer-scope
        // `slot_*` / `bar_*` bindings are visible (they were declared
        // before `rayon::scope`). Indent is 2 (one for `fn main`, one
        // for `rayon::scope(|s| {`) — same shape as pthreads-sync's
        // host body indent.
        let host_body = self.render_worker_body(self.host_worker, 2, "")?;
        out.push_str(&host_body);

        // Implicit join: closing `})` of rayon::scope. Every spawned
        // worker has returned by this point; the host's Drop guards
        // (declared above the scope) fire as fn main exits in the
        // expected order.
        writeln!(out, "    }});").ok();

        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// SlotIds a worker touches (it Pushes or Waits the data). Same
    /// derivation as pthreads-sync.
    fn slots_used_by(&self, w: WorkerId) -> Vec<SlotId> {
        let mut s: BTreeSet<SlotId> = BTreeSet::new();
        walker::collect_worker_rendezvous(&self.per_worker[&w], &self.slot_ids, &mut s);
        s.into_iter().collect()
    }

    /// The barrier `SyncTag`s a worker participates in (ascending
    /// tag order — `SyncTag: Ord`). Same derivation as pthreads-sync.
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
    /// Same shape as pthreads-sync's; the body emits the same code
    /// because both backends use the SAME shared walker with the
    /// SAME `WalkerCtx` (rendezvous_prefix = "slot").
    fn render_worker_body(
        &self,
        worker: WorkerId,
        base_indent: usize,
        prefix: &str,
    ) -> Result<String, EmitError> {
        let mut out = String::new();
        let pad = "    ".repeat(base_indent);
        let evs = &self.per_worker[&worker];

        // TASK-0349 cycle 220: whole-array-recv-only data EXCLUDED
        // from pre-init and emitted as `let <name> = <rhs>;` at recv
        // site (see `collect_pre_init` doc).
        let (pre_init, let_at_wait) = self.collect_pre_init(worker)?;
        for (name, did) in &pre_init {
            let ty = self.sidecar.data_type(*did).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "pre-init data `{name}` ({did:?}) has no ResolvedType in sidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            // Identity-aware: an accumulator-fan-in datum pre-inits to
            // its combine identity (TASK-0343.01.02); every other datum
            // sees `None` → zero, unchanged.
            let init =
                render_array_init_for_combine(ty, self.sidecar.combine_for_data.get(did).copied());
            writeln!(out, "{pad}let mut {name}: {rty} = {init};").ok();
        }
        if !pre_init.is_empty() {
            writeln!(out).ok();
        }

        let walker_ctx = WalkerCtx {
            names: self.names,
            sidecar: self.sidecar,
            rendezvous_prefix: "slot",
            rendezvous_ids: &self.slot_ids,
            pair_tiles: &self.pair_tiles,
            accumulate_waits: &self.accumulate_waits,
            let_at_wait_data: &let_at_wait,
        };
        walker::render_worker_events(&walker_ctx, worker, evs, &mut out, base_indent, prefix)?;
        Ok(out)
    }

    /// Pre-init set — same derivation as pthreads-sync. Returns the
    /// pre-init Vec and the per-worker let-at-wait DataId set
    /// (TASK-0349 cycle 220).
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
}

// --------------------------------------------------------------------
// Type rendering helpers (sidecar-driven; no AlgoIR)
// --------------------------------------------------------------------

fn rust_type_of(ty: &nucleus_compiler::algo::ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

// Pre-init array/scalar literal rendering is delegated to the shared
// `backend_common::render::render_array_init_for_combine` (single
// source of truth for the combine-identity init across all backends,
// TASK-0343.01.02) — there is no longer a local `render_array_init` /
// `rust_scalar_zero` copy that could drift.

// --------------------------------------------------------------------
// TASK-0052.05: Count check_frame collection across workers
// --------------------------------------------------------------------

/// Collect every Count `check loop` frame from every worker's
/// EventList and DEDUPLICATE by sanitized ident. Same shape as
/// pthreads-sync's `collect_unique_count_check_frames`.
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
