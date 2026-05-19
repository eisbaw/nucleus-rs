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
//! ## Barrier identity (HONEST LIMITATION — see TASK-0172)
//!
//! [`Event::Push`]/[`Event::Wait`] carry a stable cross-worker
//! `seq` tag; [`Event::Sync`] does **not** carry any cross-worker
//! barrier identity (only `participants` + `kind`). The previous
//! implementation recovered barrier identity from a *global* ACFG
//! tree walk (`walk_assign_sync_ids`); that global structure is
//! deliberately absent from the disjoint per-worker EventLists.
//!
//! We therefore assign a barrier id by each worker's **pre-order
//! Sync index** (the k-th `Sync` a worker encounters, descending
//! into `Event::Loop` bodies, is barrier k). This is byte-identical
//! to the old global-walk ids **iff every participant of every
//! barrier sees the same prefix of barriers in the same order** —
//! true for *uniform* barriers (every Sync has the same participant
//! set), which is the shape `inject_syncs` produces for the tier-1
//! examples (example 02-split: three `{host,w0}` barriers). We
//! **validate** that invariant and fail loud
//! ([`EmitError::ContractGap`]) if a non-uniform / partial-barrier
//! schedule reaches this path — we do NOT silently emit a mismatched
//! barrier graph. A proper stable [`Event::Sync`] identity (the Sync
//! analogue of `seq`/`FireBinding`/`Event::Loop`) is filed as
//! **TASK-0172**; until it lands, partial-barrier multi-worker
//! schedules are a typed codegen error here, not a wrong binary.
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
use compiler::event::{DataId, Event, WorkerId};
use compiler::sidecar::NameSidecar;

use crate::{render_const_expr_pub, rust_scalar_type, EmitError, NameTables, RenderCtxPub};

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

/// Stable identifier for one Slot allocated for a cross-worker data
/// symbol (assigned by sorted `DataId` — the same deterministic
/// order the old `xfers.keys().enumerate()` produced, since
/// `acfg.name_data` assigned DataIds in declaration order).
type SlotId = usize;
/// Stable identifier for one barrier.
type BarrierId = usize;

struct Plan<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    /// Workers with a non-empty EventList, in WorkerId order.
    used_workers: Vec<WorkerId>,
    /// The host (worker named "host", else smallest used WorkerId).
    host_worker: WorkerId,
    /// Cross-worker data symbols (those that appear in a Push/Wait),
    /// sorted by DataId -> SlotId.
    slot_ids: BTreeMap<DataId, SlotId>,
    /// Number of distinct barriers (max pre-order Sync index + 1
    /// across used workers; validated uniform).
    barrier_count: usize,
    /// BarrierId -> participants. Recovered from the per-worker
    /// `Event::Sync.participants` (every participant records the
    /// barrier; we take the union, which equals the set since the
    /// projection clones the same participant set into each).
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

        // Cross-worker data: every DataId mentioned by a Push/Wait.
        // Sorted by DataId (BTreeSet) -> SlotId, the same order the
        // old `xfers` BTreeMap<DataId> produced.
        let mut xfer_data: BTreeSet<DataId> = BTreeSet::new();
        for evs in per_worker.values() {
            collect_xfer_data(evs, &mut xfer_data);
        }
        let slot_ids: BTreeMap<DataId, SlotId> =
            xfer_data.iter().enumerate().map(|(i, d)| (*d, i)).collect();

        // Barrier identity by per-worker pre-order Sync index. Each
        // worker's k-th Sync is barrier k. Validate that every
        // worker agrees on the participant set per index (uniform
        // barrier invariant — see module docs / TASK-0172).
        let mut barrier_participants: BTreeMap<BarrierId, BTreeSet<WorkerId>> = BTreeMap::new();
        let mut barrier_count = 0usize;
        for w in &used_workers {
            let evs = &per_worker[w];
            let mut idx = 0usize;
            collect_barriers_preorder(evs, &mut idx, &mut |bid, parts| {
                barrier_count = barrier_count.max(bid + 1);
                match barrier_participants.get(&bid) {
                    None => {
                        barrier_participants.insert(bid, parts.clone());
                        Ok(())
                    }
                    Some(existing) if existing == parts => Ok(()),
                    Some(existing) => Err(EmitError::ContractGap(format!(
                        "barrier #{bid} has inconsistent participant sets across \
                         workers ({existing:?} vs {parts:?}); Event::Sync carries \
                         no stable cross-worker identity and the pre-order-index \
                         recovery only holds for uniform barriers. This is a \
                         partial-barrier schedule the EventList-only multi-worker \
                         path cannot byte-identically lower yet — a stable \
                         Event::Sync identity is TASK-0172."
                    ))),
                }
            })?;
        }

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            host_worker,
            slot_ids,
            barrier_count,
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

        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        // ---- NUC_NONDET_TEST per-process nonce (TASK-0145). ----
        //
        // Preserved verbatim from the pre-TASK-0124 implementation:
        // the determinism-check-negative gate relies on this. Two
        // `nucleus build` processes get two nonces -> guaranteed byte
        // difference -> determinism-check fails -> -negative succeeds.
        // Runtime env gate (not cfg!): a nested cargo --features in
        // the harness's own cargo run does not reliably rebuild
        // against the shared target cache. Gated on exact "1"; LOUD
        // stderr banner so a non-reproducible build is never silent.
        // Internal test scaffolding only; relocating it out of
        // production codegen is TASK-0157.
        if std::env::var("NUC_NONDET_TEST").as_deref() == Ok("1") {
            eprintln!(
                "nucleus: WARNING: NUC_NONDET_TEST=1 — injecting a \
                 per-process nonce into generated code ON PURPOSE to test \
                 the determinism check. This build is NOT reproducible. \
                 Never set this in a real build (TASK-0145)."
            );
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            writeln!(
                out,
                "    // NUC_NONDET_TEST nonce: pid={} nanos={nanos}",
                std::process::id()
            )
            .ok();
        }

        // ---- Allocate slots (sorted DataId order). ----
        for (data_id, slot_id) in &self.slot_ids {
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

        // ---- Allocate barriers (id order). ----
        for bid in 0..self.barrier_count {
            let parts = self
                .barrier_participants
                .get(&bid)
                .cloned()
                .unwrap_or_default();
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
            for bid in &used_barriers {
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

    /// BarrierIds a worker participates in (sorted).
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
        self.render_worker_events(evs, &mut out, base_indent, prefix, &ctx, &mut 0)?;
        Ok(out)
    }

    /// Walk a worker's EventList. `sync_idx` is the running pre-order
    /// Sync counter (barrier id source).
    #[allow(clippy::too_many_arguments)]
    fn render_worker_events(
        &self,
        events: &[Event],
        out: &mut String,
        indent: usize,
        prefix: &str,
        ctx: &RenderCtxPub<'_>,
        sync_idx: &mut usize,
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
                            let name = self.data_name(o.data)?;
                            let idx = crate::render_flat_index_pub(o, ctx)?;
                            writeln!(
                                out,
                                "{pad}{name}[{idx}] = kernels::{callee}({args});"
                            )
                            .ok();
                        }
                    }
                }
                Event::Loop {
                    iter_var,
                    range,
                    body,
                    block_tag,
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
                    let (lo, hi) = match self.sidecar.loop_bounds.get(iter_var) {
                        Some(b) => (
                            render_const_expr_pub(&b.lo, ctx)?,
                            render_const_expr_pub(&b.hi, ctx)?,
                        ),
                        None => (
                            format!("{}_i64", range.start),
                            format!("{}_i64", range.end),
                        ),
                    };
                    writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                    self.render_worker_events(body, out, indent + 1, prefix, ctx, sync_idx)?;
                    writeln!(out, "{pad}}}").ok();
                }
                Event::Sync { .. } => {
                    let bid = *sync_idx;
                    *sync_idx += 1;
                    writeln!(out, "{pad}{prefix}bar_{bid}.wait();").ok();
                }
                Event::Push { data, dst, .. } => {
                    let sid = self.slot_ids.get(data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Push of data {data:?} has no slot id (not collected as \
                             cross-worker)"
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
                Event::Wait { data, src, .. } => {
                    let sid = self.slot_ids.get(data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Wait of data {data:?} has no slot id (not collected as \
                             cross-worker)"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let from = self.worker_name(*src);
                    writeln!(
                        out,
                        "{pad}{name} = {prefix}slot_{sid}.wait(); // recv `{name}` from {from}",
                    )
                    .ok();
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
}

// --------------------------------------------------------------------
// Event-walk helpers (recurse into Event::Loop bodies)
// --------------------------------------------------------------------

fn collect_xfer_data(events: &[Event], out: &mut BTreeSet<DataId>) {
    for e in events {
        match e {
            Event::Push { data, .. } | Event::Wait { data, .. } => {
                out.insert(*data);
            }
            Event::Loop { body, .. } => collect_xfer_data(body, out),
            _ => {}
        }
    }
}

fn collect_worker_slots(
    events: &[Event],
    slot_ids: &BTreeMap<DataId, SlotId>,
    out: &mut BTreeSet<SlotId>,
) {
    for e in events {
        match e {
            Event::Push { data, .. } | Event::Wait { data, .. } => {
                if let Some(s) = slot_ids.get(data) {
                    out.insert(*s);
                }
            }
            Event::Loop { body, .. } => collect_worker_slots(body, slot_ids, out),
            _ => {}
        }
    }
}

/// Pre-order Sync visitor: invoke `f(barrier_id, participants)` for
/// each Sync, descending into Loop bodies. The barrier id is the
/// running pre-order index. Stops on the first `Err`.
fn collect_barriers_preorder<F>(
    events: &[Event],
    idx: &mut usize,
    f: &mut F,
) -> Result<(), EmitError>
where
    F: FnMut(usize, &BTreeSet<WorkerId>) -> Result<(), EmitError>,
{
    for e in events {
        match e {
            Event::Sync { participants, .. } => {
                let bid = *idx;
                *idx += 1;
                f(bid, participants)?;
            }
            Event::Loop { body, .. } => collect_barriers_preorder(body, idx, f)?,
            _ => {}
        }
    }
    Ok(())
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
