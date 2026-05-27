//! Multi-process emit plan (mp-tcp-bufsync). The `Plan` struct holds
//! every cross-worker invariant that codegen depends on: the used-
//! worker list, host election, cross-worker `XferId` registry, per-
//! `(DataId, SeqTag)` `IterTile` map for slice-paste-aware Wait
//! gathers, plus typed accessors. Originally inline in `lib.rs` before
//! the slice-4 split — the body of `Plan::build` + the small
//! `worker_name` / `data_name` / `non_host_workers` / `ctrl_var`
//! accessors live in this module file. The heavy renderers (
//! `render_worker_program`, `render_events`, the relay/file emitters)
//! are siblings in this same `plan/` sub-module tree.
//!
//! Sub-module map (all sibling files, all extending the same
//! `impl<'a> Plan<'a>` block):
//! - `worker_program.rs` — `Plan::render_worker_program` (the per-worker
//!   `src/bin/<name>.rs` body emitter).
//! - `events.rs` — `Plan::render_events` (the event-walk codegen, called
//!   from `render_worker_program`).
//! - `relay.rs` — `Plan::data_conn_var` / `Plan::relay_schedule` /
//!   `Plan::render_relay_phase` / `Plan::collect_pre_init` /
//!   `Plan::render_run_sh` / `Plan::max_payload_bytes` (the cycle-148
//!   host-relay codegen + the `run.sh` + pre-init glue).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::elect_host_from_worker_names;
use backend_common::multi_worker_walker::{collect_accumulate_waits, collect_pair_tiles};

use crate::walkers::{
    collect_barriers_by_tag, collect_xfer_data, detect_wait_before_push_hazard,
};
use crate::EmitError;
use crate::NameTables;

mod events;
mod relay;
mod worker_program;

/// Stable identifier for one cross-worker data symbol, by sorted
/// `DataId` (deterministic; same order pthreads-sync's slot ids use).
pub(crate) type XferId = usize;

pub(crate) struct Plan<'a> {
    pub(crate) per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    pub(crate) names: &'a NameTables,
    pub(crate) sidecar: &'a NameSidecar,
    pub(crate) used_workers: Vec<WorkerId>,
    pub(crate) host_worker: WorkerId,
    /// Cross-worker data symbols sorted by DataId.
    pub(crate) xfer_ids: BTreeMap<DataId, XferId>,
    /// Per-(DataId,SeqTag) iteration tile from the originating
    /// XferPlaceholder. Drives the receiver-side leading-axis /
    /// 2D row-loop slice-paste in `backend_common::multi_worker_walker
    /// ::render_wait_assign`. Lifted to the shared helper as of TASK-0296
    /// cycle 116 — before that, mp-tcp-bufsync's Event::Wait emit
    /// rendered `{name} = {dec}` (whole-array overwrite), silently
    /// dropping partition-band slicing on the host gather. The
    /// silent-sibling defect surfaced on 06-separable-filter/distributed
    /// × mp-tcp-bufsync (`tmp` row-band gather: each worker's recv
    /// overwrote the whole `tmp` instead of pasting its band).
    /// pthreads-async + mp-tcp-event currently route through the same
    /// helper via their `WalkerCtx`; if either grows a backend-private
    /// Wait emit path, that path must also call `render_wait_assign`
    /// (= the silent-sibling memory pattern that motivated this fix).
    pub(crate) pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343 cycle 189). mp-tcp-bufsync bypasses
    /// the shared event walker (see field doc on `pair_tiles` for the
    /// historical reason) and calls `render_wait_assign` directly from
    /// `plan/events.rs`. The accumulate set is computed at Plan::build
    /// time the same way the other three tier-1 backends do
    /// (`walker::collect_accumulate_waits` per worker, unioned with
    /// WorkerId), and consulted at the Event::Wait emit site to pass
    /// the `accumulate: bool` flag to the shared helper. Empty for
    /// every cell without an overlapping-write fan-in.
    pub(crate) accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
}

impl<'a> Plan<'a> {
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

        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule. IDENTICAL choice across the four shipped
        // (M1-M4) tier-1 backends' `multi_worker::Plan::build`
        // (pthreads-sync, pthreads-async, mp-tcp-event,
        // mp-tcp-bufsync) AND the three compiler-level driver
        // wirings (cycles 160 / 162 / 163). The three M6 skeleton
        // backends (openmp-rs, mp-tcp-poll, mp-uds-event) do NOT
        // yet exercise this path — their `emit()` ContractGaps
        // before Plan::build is ever called (per TASK-0044.01 /
        // 0044.02 / 0044.03 skeleton scope). The cross-backend
        // bit-identical differential (PRD §10.1) needs every
        // shipped backend to elect the same host given the same
        // input; the helper is the single source of truth
        // (TASK-0336 cycle 164 lift).
        let host_worker = elect_host_from_worker_names(&names.worker, &used_workers)
            .ok_or_else(|| {
                EmitError::ContractGap(
                    "multi-worker emit requires at least one used worker".to_string(),
                )
            })?;

        let mut xfer_data: BTreeSet<DataId> = BTreeSet::new();
        for evs in per_worker.values() {
            collect_xfer_data(evs, &mut xfer_data);
        }
        let xfer_ids: BTreeMap<DataId, XferId> =
            xfer_data.iter().enumerate().map(|(i, d)| (*d, i)).collect();

        // Collect per-pair tiles for slice-aware Wait gathers (TASK-0296
        // cycle 116, hoisted to `collect_pair_tiles` in cycle 130 per
        // TASK-0300). The shared helper preserves deterministic first-
        // sighting-wins on `(DataId, SeqTag)`; both endpoints carry the
        // same tile by XferPlaceholder construction (TASK-0018).
        let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> =
            collect_pair_tiles(per_worker.values());

        // Barrier identity by the contract-carried `SyncTag`
        // (TASK-0172). Each Event::Sync names its own barrier; the
        // projection clones the same participant set into every
        // participant's list, so the first sighting of a tag fixes its
        // participants. No pre-order-index heuristic and no
        // uniform-barrier validation: distinct tags are independent
        // barriers, so a partial/non-uniform barrier lowers correctly.
        let mut barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        // Topology constraint (UNRELATED to TASK-0172): one stream per
        // (host, worker) pair, so every barrier must include host as
        // the mediating hub. True for the tier-1 set (02-split:
        // {host,w0}). The CTRL-arm host-mediated barrier mediation
        // (TASK-0329, Done cycle 160) lifts the underlying limitation
        // via `apply_host_mediation_inject` in the driver — DATA arm
        // was lifted as TASK-0327 (cycles 148/149) plus TASK-0329.01.02
        // (cycles 163-164b for in-`Repeat`-body w↔w). The check below
        // is now defense-in-depth: it should never fire for ACFGs that
        // came through the driver's pipeline; if an upstream change
        // ever removes the mediation pass it bites loud, never a wrong
        // binary.
        //
        // NB: the ContractGap message text below intentionally still
        // says "filed as TASK-0175" — test-pinned by
        // `nucleus/backends/mp-tcp-event/tests/multi_worker_emit.rs`
        // and `host_relay_emit.rs` for cross-backend differential
        // stability. The forward-link in the prose ABOVE supersedes;
        // do not propose updating the literal message string here.
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "barrier #{bid} participants {parts:?} exclude the host \
                     worker; mp-tcp-bufsync's one-connection-per-(host,worker) \
                     topology requires host as the barrier hub. A \
                     host-excluding barrier needs a worker-to-worker mesh \
                     (filed as TASK-0175)."
                )));
            }
        }

        // TASK-0332 (cycle 151 AC#2): defensive ContractGap for the
        // wait-before-push host-relay deadlock. Cycle-148's
        // synchronous host-relay (`Plan::render_relay_phase`) emits
        // a FLAT relay block whose hops `wire::read_msg_expect(
        // data_<src>, seq)` block on the wire-FIFO. If any non-host
        // worker's first top-level w2w event is a Wait (rather than
        // a Push), host's read blocks for that worker's first Push
        // — which the worker can't reach because it's blocked at
        // its initial Wait. The same defect class fires on
        // mp-tcp-event (cycle-150 empirical reproducer:
        // 05-stencil/distributed-2d × mp-tcp-event); cycle 151 adds
        // this defensive check to BOTH backends per the cycle-148/
        // 149 paired-lift discipline (see
        // [[feedback-silent-sibling-defect]] 10th firing).
        //
        // Conservative-but-sound: rejects every schedule whose first
        // top-level w2w event for ANY non-host worker is a Wait.
        //
        // **Cycle-162 update (TASK-0329.01.01 slice 1, Option D):**
        // the landed architectural fix on the sibling backend is
        // `apply_safe_push_reorder` (a driver-side pass), which
        // hoists hoistable w2w Pushes ahead of preceding w2w Waits.
        // The reorder pass is NOT applied on mp-tcp-bufsync — its
        // per-pair FIFO single-stream constraint 3 (cycle-148 design)
        // makes the analogous splice-point lift unsafe (see memory
        // `project-mp-tcp-event-vs-bufsync-safety-profile`). This
        // detector on mp-tcp-bufsync therefore behaves UNCHANGED
        // from cycle 151: it rejects every wait-before-push shape
        // unconditionally.
        //
        // No in-tree mp-tcp-bufsync schedule triggers this today
        // (the only candidate, 05-stencil/distributed-2d, is
        // capability-skipped on TASK-0042: async + buffer + event
        // not supported by mp-tcp-bufsync's sync transport). The
        // guard remains paired-lifted for fail-loud hygiene if a
        // future capability lift exposes a bufsync-compatible
        // wait-before-push schedule.
        detect_wait_before_push_hazard(per_worker, host_worker)?;

        // Per-worker overlapping-write accumulator classification
        // (TASK-0343 cycle 189) — mirrors the other three tier-1
        // backends' Plan::build accumulate_waits computation
        // field-for-field. The actual emit-time consultation lives
        // in `plan/events.rs` at the Event::Wait branch (since
        // mp-tcp-bufsync bypasses the shared walker — see field doc).
        let mut accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)> = BTreeSet::new();
        for w in &used_workers {
            let per_worker_set = collect_accumulate_waits(
                &per_worker[w],
                sidecar,
                &pair_tiles,
            );
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
            xfer_ids,
            pair_tiles,
            accumulate_waits,
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

    /// Non-host used workers that exchange anything with the host
    /// (every used worker, in the tier-1 set), in WorkerId order.
    pub(crate) fn non_host_workers(&self) -> Vec<WorkerId> {
        self.used_workers
            .iter()
            .copied()
            .filter(|w| *w != self.host_worker)
            .collect()
    }

    /// Control-channel variable a given worker uses to barrier with
    /// `peer`. Host: `ctrl_<peer>`; non-host worker: `ctrl_host`.
    pub(crate) fn ctrl_var(&self, self_is_host: bool, peer: WorkerId) -> String {
        if self_is_host {
            format!("ctrl_{}", self.worker_name(peer))
        } else {
            "ctrl_host".to_string()
        }
    }
}
