//! Multi-worker codegen for the pthreads-sync backend (TASK-0122).
//!
//! Lowers a post-injection ACFG with two or more workers into a
//! standalone Rust program that spawns one `std::thread` per
//! non-host worker, synchronises them via `std::sync::Barrier`, and
//! exchanges typed data via shared-memory `Slot<T>` (a
//! `Mutex<Option<T>>` + `Condvar`).
//!
//! ## Scope and limitations
//!
//! - **Sync transfers only.** The `transfer D : sync` directive is
//!   the only mode supported here; `buffer=N` for N>1 and async
//!   transfers are rejected with `EmitError::UnsupportedFeature`
//!   (the backend's `capabilities.toml` already declares
//!   `supports_async = false`, but we check again at codegen so the
//!   error has a precise message).
//! - **Whole-symbol transfer granularity.** The ACFG's transfer
//!   injection currently records per-iteration Wait placeholders
//!   inside enclosing loops *without* matching Push placeholders
//!   (see the transfer_inject pass notes — splice_pushes_for_waits
//!   only handles producers in the same Sequence as the consumer,
//!   so a load_input at top-level + consumer inside a `for` ends
//!   up with no Push). This codegen sidesteps that gap by
//!   synthesising one whole-array Slot per cross-worker data
//!   symbol, derived from `linked.data_producers` and
//!   `linked.data_consumers`, and ignoring the ACFG's Xfer nodes
//!   entirely. The Sync nodes from `inject_syncs` are still
//!   honoured.
//!
//!   Implication: the AC #5 bit-identical e2e on example 02 works,
//!   but the codegen does NOT exercise per-tile transfers. When
//!   tile-coalescing lands (TASK-0116) and Push insertion across
//!   scopes is fixed, this module switches to ACFG-driven xfer
//!   emission. Filed as a follow-up.
//! - **Single producer per data symbol.** Single-assignment is a
//!   PRD §6.2.1 invariant the algorithm pass already enforces.
//! - **One consumer entity per data symbol.** A data symbol consumed
//!   by two distinct worker entities would need fan-out (one slot
//!   per consumer). Not needed for example 02 (each cross-worker
//!   data has exactly one consumer entity) but flagged in the code
//!   path with an `UnsupportedFeature` error so the rejection is
//!   loud rather than silent.
//! - **Distributed placements (`place k on {w0,w1,w2}`)** are NOT
//!   supported by this pass: a kernel placed on a set of workers
//!   needs iteration-space partitioning, which is a separate
//!   compiler pass (TASK-0117). This codegen rejects such
//!   placements with `UnsupportedFeature`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use compiler::algo::{AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, ResolvedType, ScalarType};
use compiler::link::LinkedIR;
use compiler::{
    ACFGNode, DataId, NotifyMode, Operation, SyncPlaceholder, TransferPolicy, WorkerId, ACFG,
};

use crate::EmitError;

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Render the contents of `main.rs` for a multi-worker schedule.
///
/// Returns an `EmitError::UnsupportedFeature` when a structural
/// feature isn't supported (async transfers, distributed placement,
/// multiple consumer entities for one data symbol, etc.).
pub(crate) fn render_main_rs_multi(acfg: &ACFG, linked: &LinkedIR) -> Result<String, EmitError> {
    // Validate placements first — fail loud before doing any work.
    validate_placements(linked)?;
    let plan = Plan::build(acfg, linked)?;
    plan.emit()
}

// --------------------------------------------------------------------
// Validation
// --------------------------------------------------------------------

/// Reject placements this codegen cannot honour.
///
/// Specifically: any `place k on {w0, w1, ...}` set with more than one
/// worker. The link pass collapses such placements into a single
/// `WorkerEntity`; we want each kernel placed on exactly one worker
/// here because the projection assumes "this Operation runs on
/// exactly the workers in `op.workers`" with no iteration-space
/// partitioning.
fn validate_placements(linked: &LinkedIR) -> Result<(), EmitError> {
    for (kernel, entity) in &linked.kernel_workers {
        if entity.0.len() > 1 {
            return Err(EmitError::UnsupportedFeature(format!(
                "kernel `{kernel}` is placed on {} workers — distributed placement \
                 is not supported by the pthreads-sync backend yet (TASK-0117)",
                entity.0.len()
            )));
        }
    }
    Ok(())
}

// --------------------------------------------------------------------
// Plan: pre-computed structural info, then emit
// --------------------------------------------------------------------

/// Pre-computed view over the ACFG / LinkedIR that the emitter
/// consumes. Built once; consumed read-only by every per-worker
/// projection.
struct Plan<'a> {
    acfg: &'a ACFG,
    linked: &'a LinkedIR,
    /// Reverse map: WorkerId -> name. Built from `acfg.name_workers`.
    worker_name: BTreeMap<WorkerId, String>,
    /// Reverse map: DataId -> name.
    data_name: BTreeMap<DataId, String>,
    /// Reverse map: KernelId -> name.
    kernel_name: BTreeMap<KernelId, String>,
    /// Cross-worker data symbols: data D whose producer worker != at
    /// least one consumer worker. Keyed by DataId; value is
    /// (producer, consumer) WorkerId pair. We only support the
    /// one-producer-one-consumer-entity shape here; multi-consumer
    /// fan-out is rejected during build.
    xfers: BTreeMap<DataId, XferSpec>,
    /// Sync nodes seen in ACFG walk order, each assigned a stable ID
    /// (0, 1, 2, ...) that both the host and worker threads agree on.
    /// Indexed by structural-walk order — see `walk_assign_sync_ids`.
    sync_ids: BTreeMap<SyncKey, BarrierId>,
    /// All workers actually used by Operations. Excludes workers
    /// declared in the schedule but never placed on. Host is whichever
    /// worker has the name "host" in the schedule (no other special
    /// meaning — the convention comes from PRD §6.3).
    used_workers: BTreeSet<WorkerId>,
    /// Which worker is the main thread. We pick the worker named
    /// "host" if present, otherwise the lexicographically smallest.
    host_worker: WorkerId,
    /// For each cross-worker data D, the BarrierId-style index into a
    /// per-data Slot vector. Stable for cross-worker codegen.
    slot_ids: BTreeMap<DataId, SlotId>,
    /// For each (worker, data) where the worker writes D and D is
    /// cross-worker, the structural path identifying the LAST WRITE
    /// position in the ACFG. The push for D is emitted immediately
    /// after the structural element at the top level whose subtree
    /// contains that last write.
    ///
    /// Concretely the path is the chain of indices in the root
    /// Sequence -> Repeat body -> ... down to the producing
    /// Operation. We use only the TOP-LEVEL index for push placement.
    push_after_top_level_idx: BTreeMap<DataId, usize>,
    /// For each cross-worker D, the index of the top-level Sequence
    /// child where the consumer worker first reads D. The wait for D
    /// is emitted before that top-level child.
    wait_before_top_level_idx: BTreeMap<DataId, usize>,
}

/// Stable identifier for one Sync barrier. We hand them out in
/// ACFG walk order — both worker projections walk the tree in the
/// same order, so they agree on which barrier they are participating
/// in.
type BarrierId = usize;

/// Stable identifier for one Slot allocated for a cross-worker data
/// symbol.
type SlotId = usize;

/// Identifies a Sync node within the ACFG by a path of indices into
/// nested Sequences / Repeat bodies. Different Sync nodes in the
/// same tree have different paths; the same Sync node visited from
/// any worker's projection has the same path.
type SyncKey = Vec<usize>;

/// Kernel ID is opaque `u64`. We define our own newtype here so the
/// reverse map can use a typed key without pulling the compiler's
/// KernelId into every signature.
use compiler::KernelId;

/// Specification of one cross-worker data transfer.
#[derive(Debug, Clone)]
struct XferSpec {
    /// Producer worker (the source of the data).
    producer: WorkerId,
    /// Consumer worker. Multi-consumer fan-out is unsupported and
    /// rejected during plan build.
    consumer: WorkerId,
    /// Policy as resolved from the schedule's `transfer` directive.
    /// We require `synchronous = true` and `buffer = 1`; async / >1
    /// buffer is rejected at plan build.
    #[allow(dead_code)]
    policy: TransferPolicy,
}

impl<'a> Plan<'a> {
    fn build(acfg: &'a ACFG, linked: &'a LinkedIR) -> Result<Self, EmitError> {
        // Reverse name tables.
        let worker_name: BTreeMap<WorkerId, String> = acfg
            .name_workers
            .iter()
            .map(|(n, id)| (*id, n.clone()))
            .collect();
        let data_name: BTreeMap<DataId, String> = acfg
            .name_data
            .iter()
            .map(|(n, id)| (*id, n.clone()))
            .collect();
        let kernel_name: BTreeMap<KernelId, String> = acfg
            .name_kernels
            .iter()
            .map(|(n, id)| (*id, n.clone()))
            .collect();

        // Workers actually used.
        let mut used_workers: BTreeSet<WorkerId> = BTreeSet::new();
        collect_used_workers_helper(&acfg.root, &mut used_workers);

        // Choose host: the worker literally named "host", or the
        // lexicographically smallest if no such worker exists. PRD
        // §6.3 uses "host" by convention but the schedule grammar
        // doesn't require any worker to be named that.
        let host_worker = match acfg.name_workers.get("host") {
            Some(id) if used_workers.contains(id) => *id,
            _ => *used_workers
                .iter()
                .next()
                .expect("multi-worker emit requires at least one used worker"),
        };

        // Cross-worker data symbols. For each data symbol that has
        // both a recorded producer entity and at least one consumer
        // entity, check whether the entities differ. If yes, this is
        // a cross-worker transfer.
        //
        // We need single-producer + single-consumer-entity (the
        // entity collapses to one worker because we already rejected
        // distributed placements above).
        let mut xfers: BTreeMap<DataId, XferSpec> = BTreeMap::new();
        for (data_name_str, producer_entity) in &linked.data_producers {
            let data_id = match acfg.name_data.get(data_name_str) {
                Some(id) => *id,
                None => continue,
            };
            // Producer entity must be exactly one worker (we rejected
            // distributed placements above).
            let producer = match producer_entity.0.iter().next() {
                Some(name) => name.clone(),
                None => continue,
            };
            let producer_id = *acfg
                .name_workers
                .get(&producer)
                .expect("producer worker not in name table");

            // Consumer entities for this data symbol.
            let consumers = match linked.data_consumers.get(data_name_str) {
                Some(c) => c,
                None => continue,
            };
            if consumers.is_empty() {
                continue;
            }
            // We require a single consumer entity (multi-consumer
            // fan-out is unsupported here).
            if consumers.len() > 1 {
                return Err(EmitError::UnsupportedFeature(format!(
                    "data symbol `{data_name_str}` has {} consumer entities; \
                     multi-consumer fan-out is not supported at M1",
                    consumers.len()
                )));
            }
            let consumer_entity = consumers.iter().next().unwrap();
            let consumer_name = consumer_entity
                .0
                .iter()
                .next()
                .expect("consumer entity is non-empty");
            let consumer_id = *acfg
                .name_workers
                .get(consumer_name)
                .expect("consumer worker not in name table");

            // Skip intra-worker data.
            if producer_id == consumer_id {
                continue;
            }

            // Resolve the schedule's transfer policy for this data.
            // Per PRD §6.3.4, a directive that crosses workers must
            // exist; the schedule lowering currently does not fail on
            // a missing one (lenient), so we default to sync if the
            // schedule omitted it. The capability check (already run
            // before emit) would have rejected an unsupported policy.
            let policy: TransferPolicy = linked
                .sched
                .transfers
                .get(data_name_str)
                .map(policy_from_directive)
                .unwrap_or_default();
            // Re-check capability constraints in case the driver
            // skipped them. async / buffer>1 are rejected for
            // pthreads-sync.
            if !policy.synchronous {
                return Err(EmitError::UnsupportedFeature(format!(
                    "data symbol `{data_name_str}` requests `transfer=async`; \
                     pthreads-sync is sync-only"
                )));
            }
            if policy.buffer > 1 {
                return Err(EmitError::UnsupportedFeature(format!(
                    "data symbol `{data_name_str}` requests `buffer={}`; \
                     pthreads-sync supports buffer=1 only",
                    policy.buffer
                )));
            }
            if matches!(policy.notify, NotifyMode::Event | NotifyMode::Poll) {
                // Both notify=event and notify=poll are mapped to
                // the condvar-based slot; the schedule's stated
                // preference is honoured insofar as the backend has
                // only one mechanism. If a future capability check
                // distinguishes these we'd branch here. For now,
                // accept and proceed.
            }

            xfers.insert(
                data_id,
                XferSpec {
                    producer: producer_id,
                    consumer: consumer_id,
                    policy,
                },
            );
        }

        // Assign Sync IDs by walking the ACFG in canonical order.
        let mut sync_ids: BTreeMap<SyncKey, BarrierId> = BTreeMap::new();
        let mut next_id: BarrierId = 0;
        walk_assign_sync_ids(&acfg.root, &mut Vec::new(), &mut sync_ids, &mut next_id);

        // Slot IDs: one per cross-worker data, stable order by DataId.
        let slot_ids: BTreeMap<DataId, SlotId> =
            xfers.keys().enumerate().map(|(i, d)| (*d, i)).collect();

        // Compute push/wait top-level indices per cross-worker data.
        let (push_after_top_level_idx, wait_before_top_level_idx) =
            compute_xfer_positions(acfg, linked, &xfers);

        Ok(Plan {
            acfg,
            linked,
            worker_name,
            data_name,
            kernel_name,
            xfers,
            sync_ids,
            used_workers,
            host_worker,
            slot_ids,
            push_after_top_level_idx,
            wait_before_top_level_idx,
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
        // Slot type — one-shot rendezvous channel. Same shape as the
        // task description's sketch.
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

        // unused_mut / dead_code / unused_variables: same rationale as
        // the single-worker emitter. With distributed projection
        // many workers' code paths legitimately don't touch every
        // binding.
        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        // ---- Allocate slots and barriers. ----
        for (data_id, slot_id) in &self.slot_ids {
            let name = self.data_name.get(data_id).expect("data id has name");
            let ty = self
                .linked
                .algo
                .data
                .get(name)
                .map(|d| rust_type_of(&d.ty))
                .unwrap_or_else(|| "()".to_string());
            writeln!(
                out,
                "    let slot_{slot_id}: Arc<Slot<{ty}>> = Arc::new(Slot::new()); // data `{name}`",
            )
            .ok();
        }

        // Barriers: one per Sync node. Participant count comes from
        // the SyncPlaceholder.
        let barriers = self.collect_barriers();
        for (bid, participants) in &barriers {
            let cnt = participants.len();
            let part_names: Vec<&str> = participants
                .iter()
                .map(|w| self.worker_name.get(w).map(String::as_str).unwrap_or("?"))
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
        // Iterate used_workers in deterministic order.
        let mut handles: Vec<(WorkerId, String)> = Vec::new();
        for w in &self.used_workers {
            if *w == self.host_worker {
                continue;
            }
            let wname = self
                .worker_name
                .get(w)
                .cloned()
                .unwrap_or_else(|| format!("w{}", w.0));
            // Clone Arcs for this worker's projection.
            // We need to know which slots / barriers this worker
            // actually uses to avoid pulling unused clones.
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
            // Per-worker body. Inside the closure, refer to the
            // captured clones by their `{wname}_slot_{n}` / `{wname}_bar_{n}` names.
            let body = self.render_worker_body(*w, 2, &used_slots, &used_barriers)?;
            out.push_str(&body);
            writeln!(out, "    }});").ok();
            handles.push((*w, format!("{wname}_handle")));
            writeln!(out).ok();
        }

        // ---- Host body. ----
        let host_used_slots = self.slots_used_by(self.host_worker);
        let host_used_barriers = self.barriers_used_by(self.host_worker);
        // Inside main (no closure), the slots are addressed by their
        // bare names (slot_N / bar_N), so we don't add `_<wname>_`
        // prefix renames. We tell render_worker_body to use the bare
        // names by passing the host's prefix as an empty string.
        let host_body = self.render_worker_body_with_prefix(
            self.host_worker,
            1,
            "",
            &host_used_slots,
            &host_used_barriers,
        )?;
        out.push_str(&host_body);

        // ---- Join workers. ----
        writeln!(out).ok();
        for (_, h) in &handles {
            writeln!(out, "    {h}.join().expect(\"worker thread panicked\");").ok();
        }

        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// Map: BarrierId -> participants. Walk the ACFG once and
    /// recover every Sync node's participant set, keyed by the
    /// BarrierId we already assigned.
    fn collect_barriers(&self) -> BTreeMap<BarrierId, BTreeSet<WorkerId>> {
        let mut out: BTreeMap<BarrierId, BTreeSet<WorkerId>> = BTreeMap::new();
        let mut path: Vec<usize> = Vec::new();
        walk_collect_barriers(&self.acfg.root, &mut path, &self.sync_ids, &mut out);
        out
    }

    /// SlotIds used by a given worker — either as the producer
    /// (calls `push`) or consumer (calls `wait`).
    fn slots_used_by(&self, w: WorkerId) -> Vec<SlotId> {
        let mut out: Vec<SlotId> = Vec::new();
        for (data_id, spec) in &self.xfers {
            if spec.producer == w || spec.consumer == w {
                let sid = *self.slot_ids.get(data_id).expect("slot id exists");
                out.push(sid);
            }
        }
        out.sort_unstable();
        out
    }

    /// BarrierIds this worker participates in.
    fn barriers_used_by(&self, w: WorkerId) -> Vec<BarrierId> {
        let barriers = self.collect_barriers();
        let mut out: Vec<BarrierId> = barriers
            .iter()
            .filter(|(_, parts)| parts.contains(&w))
            .map(|(id, _)| *id)
            .collect();
        out.sort_unstable();
        out
    }

    fn render_worker_body(
        &self,
        worker: WorkerId,
        base_indent: usize,
        used_slots: &[SlotId],
        used_barriers: &[BarrierId],
    ) -> Result<String, EmitError> {
        let wname = self
            .worker_name
            .get(&worker)
            .cloned()
            .unwrap_or_else(|| format!("w{}", worker.0));
        let prefix = format!("{wname}_");
        self.render_worker_body_with_prefix(worker, base_indent, &prefix, used_slots, used_barriers)
    }

    /// Render the per-worker body. `prefix` is prepended to slot/bar
    /// identifiers — empty for the host (uses the bare top-level
    /// names), `<wname>_` for spawned workers (where Arc clones live
    /// in the closure's captured environment).
    fn render_worker_body_with_prefix(
        &self,
        worker: WorkerId,
        base_indent: usize,
        prefix: &str,
        _used_slots: &[SlotId],
        _used_barriers: &[BarrierId],
    ) -> Result<String, EmitError> {
        let mut out = String::new();
        let pad = "    ".repeat(base_indent);

        // Pre-init: for every cross-worker data this worker
        // consumes, we'll later overwrite the binding via wait().
        // For every cross-worker data this worker produces, the
        // producer kernel will write into a local binding then
        // push it.
        //
        // We additionally need pre-init for indexed-LHS data this
        // worker writes (e.g. `c[i] <-- add(...)` on w0).
        let pre_init = collect_pre_init_data_for_worker(&self.linked.algo, self.linked, worker);
        for (name, ty) in &pre_init {
            let rs_init = render_array_init(ty);
            writeln!(
                out,
                "{pad}let mut {name}: {} = {rs_init};",
                rust_type_of(ty)
            )
            .ok();
        }
        if !pre_init.is_empty() {
            writeln!(out).ok();
        }

        let ctx = WorkerRenderCtx {
            plan: self,
            worker,
            prefix,
        };

        // Walk the ACFG root (Sequence) child-by-child, emitting:
        // - For each top-level index i:
        //     * Before the child, emit waits for any cross-worker
        //       data D whose `wait_before_top_level_idx[D] == i` and
        //       worker is its consumer.
        //     * Emit the child's projection (Sync, Operation,
        //       Repeat, ...).
        //     * After the child, emit pushes for any cross-worker
        //       data D whose `push_after_top_level_idx[D] == i` and
        //       worker is its producer.
        let children = match &self.acfg.root {
            ACFGNode::Sequence(c) => c.as_slice(),
            // A single-statement program would still wrap in Sequence
            // per build_acfg; defensive arm.
            other => std::slice::from_ref(other),
        };

        for (i, child) in children.iter().enumerate() {
            // Pre-child waits.
            for (data_id, spec) in &self.xfers {
                if spec.consumer != worker {
                    continue;
                }
                if let Some(idx) = self.wait_before_top_level_idx.get(data_id) {
                    if *idx == i {
                        let name = self.data_name.get(data_id).expect("data id has name");
                        let sid = *self.slot_ids.get(data_id).expect("slot id");
                        let from = self
                            .worker_name
                            .get(&spec.producer)
                            .cloned()
                            .unwrap_or_else(|| format!("w{}", spec.producer.0));
                        writeln!(
                            out,
                            "{pad}{name} = {prefix}slot_{sid}.wait(); // recv `{name}` from {from}",
                        )
                        .ok();
                    }
                }
            }

            // Child projection.
            render_node(child, &mut out, base_indent, &ctx, &mut Vec::from([i]))?;

            // Post-child pushes.
            for (data_id, spec) in &self.xfers {
                if spec.producer != worker {
                    continue;
                }
                if let Some(idx) = self.push_after_top_level_idx.get(data_id) {
                    if *idx == i {
                        let name = self.data_name.get(data_id).expect("data id has name");
                        let sid = *self.slot_ids.get(data_id).expect("slot id");
                        let to = self
                            .worker_name
                            .get(&spec.consumer)
                            .cloned()
                            .unwrap_or_else(|| format!("w{}", spec.consumer.0));
                        writeln!(
                            out,
                            "{pad}{prefix}slot_{sid}.push({name}.clone()); // send `{name}` to {to}",
                        )
                        .ok();
                    }
                }
            }
        }

        Ok(out)
    }
}

// --------------------------------------------------------------------
// Walk helpers
// --------------------------------------------------------------------

fn collect_used_workers_helper(node: &ACFGNode, out: &mut BTreeSet<WorkerId>) {
    match node {
        ACFGNode::Operation(op) => {
            for w in &op.workers {
                out.insert(*w);
            }
        }
        ACFGNode::Repeat { body, .. } => collect_used_workers_helper(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_used_workers_helper(c, out);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Walk the ACFG and assign a stable BarrierId to each Sync node
/// based on its path of indices through Sequences and Repeat bodies.
fn walk_assign_sync_ids(
    node: &ACFGNode,
    path: &mut Vec<usize>,
    out: &mut BTreeMap<SyncKey, BarrierId>,
    next: &mut BarrierId,
) {
    match node {
        ACFGNode::Sync(_) => {
            out.insert(path.clone(), *next);
            *next += 1;
        }
        ACFGNode::Sequence(children) => {
            for (i, c) in children.iter().enumerate() {
                path.push(i);
                walk_assign_sync_ids(c, path, out, next);
                path.pop();
            }
        }
        ACFGNode::Repeat { body, .. } => {
            path.push(usize::MAX); // disambiguate "into Repeat body" from a sibling index
            walk_assign_sync_ids(body, path, out, next);
            path.pop();
        }
        ACFGNode::Operation(_) | ACFGNode::Xfer(_) => {}
    }
}

fn walk_collect_barriers(
    node: &ACFGNode,
    path: &mut Vec<usize>,
    sync_ids: &BTreeMap<SyncKey, BarrierId>,
    out: &mut BTreeMap<BarrierId, BTreeSet<WorkerId>>,
) {
    match node {
        ACFGNode::Sync(SyncPlaceholder { participants }) => {
            if let Some(bid) = sync_ids.get(path) {
                out.insert(*bid, participants.clone());
            }
        }
        ACFGNode::Sequence(children) => {
            for (i, c) in children.iter().enumerate() {
                path.push(i);
                walk_collect_barriers(c, path, sync_ids, out);
                path.pop();
            }
        }
        ACFGNode::Repeat { body, .. } => {
            path.push(usize::MAX);
            walk_collect_barriers(body, path, sync_ids, out);
            path.pop();
        }
        ACFGNode::Operation(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Compute the top-level Sequence child index where each
/// cross-worker data symbol's push and wait should be placed.
///
/// - The producer's push goes IMMEDIATELY AFTER the top-level child
///   whose subtree contains the LAST Operation writing D.
/// - The consumer's wait goes IMMEDIATELY BEFORE the top-level child
///   whose subtree contains the FIRST Operation reading D.
fn compute_xfer_positions(
    acfg: &ACFG,
    _linked: &LinkedIR,
    xfers: &BTreeMap<DataId, XferSpec>,
) -> (BTreeMap<DataId, usize>, BTreeMap<DataId, usize>) {
    let mut push_after: BTreeMap<DataId, usize> = BTreeMap::new();
    let mut wait_before: BTreeMap<DataId, usize> = BTreeMap::new();

    let children: &[ACFGNode] = match &acfg.root {
        ACFGNode::Sequence(c) => c,
        _ => return (push_after, wait_before),
    };

    for d in xfers.keys() {
        let mut last_write: Option<usize> = None;
        let mut first_read: Option<usize> = None;
        for (i, child) in children.iter().enumerate() {
            let (reads, writes) = subtree_reads_writes(child, *d);
            if reads && first_read.is_none() {
                first_read = Some(i);
            }
            if writes {
                last_write = Some(i);
            }
        }
        if let Some(i) = last_write {
            push_after.insert(*d, i);
        }
        if let Some(i) = first_read {
            wait_before.insert(*d, i);
        }
    }

    (push_after, wait_before)
}

/// Return (reads_data, writes_data) for a subtree's Operations.
fn subtree_reads_writes(node: &ACFGNode, target: DataId) -> (bool, bool) {
    match node {
        ACFGNode::Operation(op) => {
            let mut r = false;
            let mut w = false;
            for edge in &op.dataflow.edges {
                for d in &edge.data_in {
                    if *d == target {
                        r = true;
                    }
                }
                if edge.data_out == Some(target) {
                    w = true;
                }
            }
            (r, w)
        }
        ACFGNode::Sequence(children) => {
            let mut r = false;
            let mut w = false;
            for c in children {
                let (cr, cw) = subtree_reads_writes(c, target);
                r |= cr;
                w |= cw;
            }
            (r, w)
        }
        ACFGNode::Repeat { body, .. } => subtree_reads_writes(body, target),
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => (false, false),
    }
}

// --------------------------------------------------------------------
// Per-worker projection rendering
// --------------------------------------------------------------------

struct WorkerRenderCtx<'a> {
    plan: &'a Plan<'a>,
    worker: WorkerId,
    prefix: &'a str,
}

/// Render one ACFG node to a worker's projection. `path` is the
/// chain of indices into Sequences/Repeats used to look up Sync IDs.
fn render_node(
    node: &ACFGNode,
    out: &mut String,
    indent: usize,
    ctx: &WorkerRenderCtx<'_>,
    path: &mut Vec<usize>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    match node {
        ACFGNode::Operation(op) => {
            if op.workers.contains(&ctx.worker) {
                // Look up the source IrStmt — we need the original
                // call expression (with index arg expressions).
                let stmt = lookup_source_stmt(
                    &ctx.plan.acfg.root,
                    &ctx.plan.linked.algo,
                    path,
                    &ctx.plan.kernel_name,
                    op,
                )
                .ok_or_else(|| {
                    EmitError::UnsupportedFeature(format!(
                        "could not locate source IrStmt for Operation at path {path:?}"
                    ))
                })?;
                render_op_stmt(stmt, op, out, &pad, ctx)?;
            }
            Ok(())
        }
        ACFGNode::Repeat {
            iter_var: _,
            range,
            body,
        } => {
            // Locate the source `for` statement by walking the path.
            let stmt = lookup_source_for(&ctx.plan.acfg.root, &ctx.plan.linked.algo, path)
                .ok_or_else(|| {
                    EmitError::UnsupportedFeature(format!(
                        "could not locate source For at path {path:?}"
                    ))
                })?;
            let (var, lo_expr, hi_expr, body_stmts) = match stmt {
                IrStmt::For { var, lo, hi, body } => (var, lo, hi, body),
                _ => return Err(EmitError::UnsupportedFeature("expected For at path".into())),
            };
            // Decide whether this worker needs to enter the loop at
            // all. It enters if any node inside the body is relevant
            // to this worker: an Operation it owns, or a Sync it
            // participates in.
            if !subtree_relevant_to_worker(body, ctx.worker) {
                let _ = (range, lo_expr, hi_expr, var, body_stmts);
                return Ok(());
            }
            let lo_s = render_const_expr(lo_expr, &ctx.plan.linked.algo)?;
            let hi_s = render_const_expr(hi_expr, &ctx.plan.linked.algo)?;
            writeln!(out, "{pad}for {var} in ({lo_s})..({hi_s}) {{").ok();
            // Recurse into body. The body is an ACFGNode (typically
            // Sequence); render it at indent+1.
            path.push(usize::MAX);
            render_node(body, out, indent + 1, ctx, path)?;
            path.pop();
            writeln!(out, "{pad}}}").ok();
            Ok(())
        }
        ACFGNode::Sequence(children) => {
            for (i, c) in children.iter().enumerate() {
                path.push(i);
                render_node(c, out, indent, ctx, path)?;
                path.pop();
            }
            Ok(())
        }
        ACFGNode::Sync(s) => {
            if s.participants.contains(&ctx.worker) {
                let bid = ctx.plan.sync_ids.get(path).copied().ok_or_else(|| {
                    EmitError::UnsupportedFeature(format!(
                        "sync at path {path:?} has no assigned BarrierId"
                    ))
                })?;
                writeln!(out, "{pad}{}bar_{}.wait();", ctx.prefix, bid).ok();
            }
            Ok(())
        }
        ACFGNode::Xfer(_) => {
            // Xfer placeholders from the ACFG are intentionally
            // ignored — we synthesise transfers from
            // linked.data_producers/data_consumers (see module
            // docs). The Xfer nodes the transfer_inject pass
            // produces don't have matched pushes anyway, so honouring
            // them would deadlock.
            Ok(())
        }
    }
}

fn subtree_relevant_to_worker(node: &ACFGNode, worker: WorkerId) -> bool {
    match node {
        ACFGNode::Operation(op) => op.workers.contains(&worker),
        ACFGNode::Repeat { body, .. } => subtree_relevant_to_worker(body, worker),
        ACFGNode::Sequence(children) => children
            .iter()
            .any(|c| subtree_relevant_to_worker(c, worker)),
        ACFGNode::Sync(s) => s.participants.contains(&worker),
        ACFGNode::Xfer(_) => false,
    }
}

/// Locate the source-level `IrStmt` corresponding to an `Operation`
/// node at `path`. `path` is the chain of indices into nested
/// `Sequence` children (sentinel `usize::MAX` marks "into a Repeat
/// body"). The ACFG construction preserves source order: each
/// Sequence's Nth Operation/Repeat child corresponds to the Nth
/// "active" statement in the source's matching scope, modulo any
/// inserted Sync/Xfer nodes that don't consume an IrStmt slot.
///
/// We re-walk the path while skipping over Sync/Xfer siblings — those
/// are the injected placeholders that have no source counterpart.
fn lookup_source_stmt<'a>(
    acfg_root: &ACFGNode,
    algo: &'a AlgoIR,
    path: &[usize],
    _kernel_name: &BTreeMap<KernelId, String>,
    _op: &Operation,
) -> Option<&'a IrStmt> {
    lookup_source_irstmt(acfg_root, algo, path)
}

fn lookup_source_for<'a>(
    acfg_root: &ACFGNode,
    algo: &'a AlgoIR,
    path: &[usize],
) -> Option<&'a IrStmt> {
    lookup_source_irstmt(acfg_root, algo, path)
}

// --------------------------------------------------------------------
// Type / expression rendering helpers
// --------------------------------------------------------------------

/// Map a Nuc `ResolvedType` to the corresponding Rust surface type
/// used in the generated code. Arrays flatten to `Vec<T>` (matches
/// the single-worker emitter); scalars use the natural Rust spelling.
fn rust_type_of(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

fn rust_scalar_type(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize => "usize",
        ScalarType::Isize => "isize",
        ScalarType::U8 => "u8",
        ScalarType::U16 => "u16",
        ScalarType::U32 => "u32",
        ScalarType::U64 => "u64",
        ScalarType::I8 => "i8",
        ScalarType::I16 => "i16",
        ScalarType::I32 => "i32",
        ScalarType::I64 => "i64",
        ScalarType::F32 => "f32",
        ScalarType::F64 => "f64",
        ScalarType::Bool => "bool",
    }
}

fn rust_scalar_zero(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize | ScalarType::Isize => "0",
        ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 => "0",
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 => "0",
        ScalarType::F32 | ScalarType::F64 => "0.0",
        ScalarType::Bool => "false",
    }
}

fn render_array_init(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
    }
}

/// Decide which data symbols this worker needs as `let mut`
/// bindings up front. Two cases:
///
/// 1. **Cross-worker inputs the worker consumes.** Will be overwritten
///    by `slot.wait()` at the appropriate point. We pre-init to give
///    them a typed home and silence "possibly uninitialised" errors.
///
/// 2. **Indexed-LHS data the worker writes** (e.g. `c[i] <-- add(...)`).
///    Same logic as the single-worker emitter's `collect_pre_init_data`.
fn collect_pre_init_data_for_worker(
    algo: &AlgoIR,
    linked: &LinkedIR,
    worker: WorkerId,
) -> Vec<(String, ResolvedType)> {
    let mut out: Vec<(String, ResolvedType)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    // Data this worker consumes via cross-worker transfer.
    // We need the worker's name to look up consumer entities.
    // Reverse-name lookup via the schedule's workers map.
    let worker_name = linked
        .sched
        .workers
        .keys()
        .find(|name| {
            // We need to map name back to WorkerId. The acfg has the
            // mapping but isn't in scope; we do it again from
            // linked.sched.workers, which is keyed by name and the
            // assignment-order matches build_acfg's name_workers.
            // For determinism we just sort and enumerate.
            let mut all: Vec<&String> = linked.sched.workers.keys().collect();
            all.sort();
            all.iter()
                .position(|n| *n == name.as_str())
                .map(|i| WorkerId(i as u64))
                .map(|wid| wid == worker)
                .unwrap_or(false)
        })
        .cloned();

    if let Some(wname) = worker_name.as_ref() {
        for (data_name, consumers) in &linked.data_consumers {
            let consumed_here = consumers.iter().any(|entity| entity.0.contains(wname));
            if !consumed_here {
                continue;
            }
            // Skip if the worker is also the producer (intra-worker).
            if let Some(prod) = linked.data_producers.get(data_name) {
                if prod.0.contains(wname) {
                    continue;
                }
            }
            if let Some(d) = algo.data.get(data_name) {
                if seen.insert(data_name.clone()) {
                    out.push((data_name.clone(), d.ty.clone()));
                }
            }
        }
    }

    // Data the worker writes via indexed LHS.
    let mut whole: BTreeSet<String> = BTreeSet::new();
    let mut indexed: BTreeSet<String> = BTreeSet::new();
    walk_assign_kinds_for_worker(&algo.stmts, linked, worker, &mut whole, &mut indexed);
    for name in &indexed {
        if whole.contains(name) {
            continue;
        }
        if let Some(d) = algo.data.get(name) {
            if seen.insert(name.clone()) {
                out.push((name.clone(), d.ty.clone()));
            }
        }
    }
    out
}

fn walk_assign_kinds_for_worker(
    stmts: &[IrStmt],
    linked: &LinkedIR,
    worker: WorkerId,
    whole: &mut BTreeSet<String>,
    indexed: &mut BTreeSet<String>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                if let IrExpr::Call { callee, .. } = rhs {
                    if !kernel_runs_on(linked, callee, worker) {
                        continue;
                    }
                    if lhs.indices.is_empty() {
                        whole.insert(lhs.name.clone());
                    } else {
                        indexed.insert(lhs.name.clone());
                    }
                }
            }
            IrStmt::Effect { .. } => {}
            IrStmt::For { body, .. } => {
                walk_assign_kinds_for_worker(body, linked, worker, whole, indexed)
            }
        }
    }
}

fn kernel_runs_on(linked: &LinkedIR, kernel: &str, worker: WorkerId) -> bool {
    let entity = match linked.kernel_workers.get(kernel) {
        Some(e) => e,
        None => return false,
    };
    let names: Vec<&String> = entity.0.iter().collect();
    // worker_id -> name via the schedule's sorted keys (matches
    // build_acfg's name_workers assignment).
    let mut keys: Vec<&String> = linked.sched.workers.keys().collect();
    keys.sort();
    let wname = match keys.get(worker.0 as usize) {
        Some(n) => *n,
        None => return false,
    };
    names.iter().any(|n| n.as_str() == wname.as_str())
}

fn render_const_expr(e: &IrExpr, algo: &AlgoIR) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}_i64")),
        IrExpr::Ident(n) => {
            if let Some(c) = algo.consts.get(n) {
                Ok(format!("{}_i64", c.value))
            } else {
                Ok(n.clone())
            }
        }
        IrExpr::Neg(inner) => Ok(format!("-({})", render_const_expr(inner, algo)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_const_expr(l, algo)?;
            let rs = render_const_expr(r, algo)?;
            let op_s = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            Ok(format!("({ls} {op_s} {rs})"))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside a const expression (loop bound)".into(),
        )),
    }
}

fn render_int_expr(e: &IrExpr) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}")),
        IrExpr::Ident(n) => Ok(n.clone()),
        IrExpr::Neg(inner) => Ok(format!("-({})", render_int_expr(inner)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_int_expr(l)?;
            let rs = render_int_expr(r)?;
            let op_s = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            Ok(format!("({ls} {op_s} {rs})"))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside an integer index expression".into(),
        )),
    }
}

fn render_flat_index(r: &IndexedRef, algo: &AlgoIR) -> Result<String, EmitError> {
    if r.indices.is_empty() {
        return Err(EmitError::UnsupportedFeature(
            "render_flat_index called on a non-indexed reference".into(),
        ));
    }
    if r.indices.len() == 1 {
        let i0 = render_int_expr(&r.indices[0])?;
        return Ok(format!("({i0}) as usize"));
    }
    let shape = algo.data.get(&r.name).map(|d| d.ty.dims.clone());
    let dims = match shape {
        Some(d) if d.len() == r.indices.len() => d,
        _ => {
            return Err(EmitError::UnsupportedFeature(format!(
                "data `{}` rank/shape mismatch with index list",
                r.name
            )));
        }
    };
    let mut terms: Vec<String> = Vec::with_capacity(r.indices.len());
    for (k, idx_expr) in r.indices.iter().enumerate() {
        let stride: usize = dims[k + 1..].iter().copied().product();
        let rendered = render_int_expr(idx_expr)?;
        if stride == 1 {
            terms.push(format!("({rendered})"));
        } else {
            terms.push(format!("({rendered}) * {stride}"));
        }
    }
    Ok(format!("({}) as usize", terms.join(" + ")))
}

fn render_call_args(callee: &str, args: &[IrExpr], algo: &AlgoIR) -> Result<String, EmitError> {
    let kernel = algo.kernels.get(callee).ok_or_else(|| {
        EmitError::UnsupportedFeature(format!(
            "kernel `{callee}` not in AlgoIR (link should have caught)"
        ))
    })?;
    let mut parts = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let param_ty = kernel.params.get(i);
        parts.push(render_call_arg(arg, param_ty, algo)?);
    }
    Ok(parts.join(", "))
}

fn render_call_arg(
    arg: &IrExpr,
    param_ty: Option<&ResolvedType>,
    algo: &AlgoIR,
) -> Result<String, EmitError> {
    match arg {
        IrExpr::DataRef(r) => {
            if r.indices.is_empty() {
                Ok(r.name.clone())
            } else {
                let idx = render_flat_index(r, algo)?;
                Ok(format!("{}[{idx}]", r.name))
            }
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) | IrExpr::Neg(_) | IrExpr::BinOp(_, _, _) => {
            let rendered = render_int_expr(arg)?;
            if let Some(pty) = param_ty {
                if pty.is_scalar() {
                    return Ok(format!("({rendered}) as {}", rust_scalar_type(&pty.scalar)));
                }
            }
            Ok(rendered)
        }
        IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "nested kernel call inside an argument expression".into(),
        )),
    }
}

/// Render one source IR statement at the per-worker level. The
/// caller has already verified the kernel runs on this worker.
fn render_op_stmt(
    stmt: &IrStmt,
    _op: &Operation,
    out: &mut String,
    pad: &str,
    ctx: &WorkerRenderCtx<'_>,
) -> Result<(), EmitError> {
    match stmt {
        IrStmt::Dataflow { lhs, rhs } => {
            let (callee, args) = match rhs {
                IrExpr::Call { callee, args } => (callee, args),
                other => {
                    return Err(EmitError::UnsupportedFeature(format!(
                        "dataflow RHS shape not supported: {other:?}"
                    )))
                }
            };
            let rendered_args = render_call_args(callee, args, &ctx.plan.linked.algo)?;
            if lhs.indices.is_empty() {
                // Whole-array. The single-worker emitter uses
                // `let mut D = kernels::call(...)`; here, the same
                // binding may have already been pre-init'd (e.g. it's
                // a cross-worker INPUT to a *different* worker — but
                // we're the producer here so it's not pre-init'd on
                // us). So `let mut` is correct.
                writeln!(
                    out,
                    "{pad}let mut {} = kernels::{callee}({rendered_args});",
                    lhs.name
                )
                .ok();
            } else {
                let idx = render_flat_index(lhs, &ctx.plan.linked.algo)?;
                writeln!(
                    out,
                    "{pad}{}[{idx}] = kernels::{callee}({rendered_args});",
                    lhs.name
                )
                .ok();
            }
            Ok(())
        }
        IrStmt::Effect { callee, args } => {
            let rendered_args = render_call_args(callee, args, &ctx.plan.linked.algo)?;
            writeln!(out, "{pad}kernels::{callee}({rendered_args});").ok();
            Ok(())
        }
        IrStmt::For { .. } => Err(EmitError::UnsupportedFeature(
            "render_op_stmt called on a For; the Repeat arm should have handled this".into(),
        )),
    }
}

// --------------------------------------------------------------------
// Source IrStmt lookup by ACFG path
// --------------------------------------------------------------------

/// Translate an ACFG path (chain of indices) to the corresponding
/// source `IrStmt`. Sync/Xfer ACFG nodes shift sibling indices
/// relative to the source statement list; this function walks both
/// the ACFG and `algo.stmts` in parallel to find the source stmt.
///
/// Placed here (not in the impl block) so `render_node` can call it
/// without threading the ACFG through every level — the plan is
/// borrowed via `ctx`.
fn lookup_source_irstmt<'a>(
    acfg_root: &ACFGNode,
    algo: &'a AlgoIR,
    path: &[usize],
) -> Option<&'a IrStmt> {
    // We walk: at each step, we're in some Sequence in the ACFG and
    // its matching slice of source IrStmts. The ACFG-child index `i`
    // is what we have; the source-child index `s` is the count of
    // non-placeholder ACFG children at positions < i.
    let mut acfg_cursor: &ACFGNode = acfg_root;
    let mut source_cursor: &[IrStmt] = &algo.stmts;

    // The root is a Sequence wrapping top-level IrStmts. We start
    // peeling the path.
    let mut path_idx = 0;
    while path_idx < path.len() {
        match acfg_cursor {
            ACFGNode::Sequence(children) => {
                let acfg_idx = path[path_idx];
                path_idx += 1;
                if acfg_idx == usize::MAX {
                    // Shouldn't appear at a Sequence level.
                    return None;
                }
                if acfg_idx >= children.len() {
                    return None;
                }
                // Count active siblings.
                let mut src_idx = 0usize;
                for c in &children[..acfg_idx] {
                    if is_active_acfg_node(c) {
                        src_idx += 1;
                    }
                }
                let target_acfg_child = &children[acfg_idx];
                if !is_active_acfg_node(target_acfg_child) {
                    // Path points at a placeholder; no source.
                    return None;
                }
                if src_idx >= source_cursor.len() {
                    return None;
                }
                acfg_cursor = target_acfg_child;
                source_cursor = std::slice::from_ref(&source_cursor[src_idx]);
                // source_cursor now contains exactly one IrStmt.
                // If we go deeper, we'll need to drill into it.
            }
            ACFGNode::Repeat { body, .. } => {
                // Path step should be usize::MAX (into the body).
                let step = path[path_idx];
                path_idx += 1;
                if step != usize::MAX {
                    return None;
                }
                // Source side: source_cursor is a slice of one IrStmt
                // which must be a For. Drill into its body.
                let for_stmt = source_cursor.first()?;
                let body_stmts = match for_stmt {
                    IrStmt::For { body, .. } => body.as_slice(),
                    _ => return None,
                };
                acfg_cursor = body;
                source_cursor = body_stmts;
            }
            _ => return None,
        }
    }
    // We've walked the path. Return the IrStmt the cursor points at,
    // if any. After a Sequence step that hit an active child, the
    // source cursor narrowed to a single stmt — return that.
    source_cursor.first()
}

fn is_active_acfg_node(n: &ACFGNode) -> bool {
    matches!(n, ACFGNode::Operation(_) | ACFGNode::Repeat { .. })
}

// --------------------------------------------------------------------
// Policy lowering
// --------------------------------------------------------------------

fn policy_from_directive(dir: &compiler::sched::ResolvedTransferDirective) -> TransferPolicy {
    use compiler::sched::ResolvedTransferOption;
    let mut p = TransferPolicy::default();
    for opt in &dir.options {
        match opt {
            ResolvedTransferOption::Sync => p.synchronous = true,
            ResolvedTransferOption::Async => p.synchronous = false,
            ResolvedTransferOption::Buffer(n) => p.buffer = *n,
            ResolvedTransferOption::Notify(k) => p.notify = (*k).into(),
        }
    }
    p
}
