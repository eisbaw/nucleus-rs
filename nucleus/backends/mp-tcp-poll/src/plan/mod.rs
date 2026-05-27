//! Multi-process emit plan (mp-tcp-poll). The `Plan` struct holds
//! every cross-worker invariant that codegen depends on: the used-
//! worker list, host election, cross-worker `XferId` registry,
//! per-(DataId, SeqTag) `IterTile` map, plus typed accessors.
//!
//! Sibling of `nucleus/backends/mp-tcp-bufsync/src/plan/mod.rs`. The
//! plan is wait-primitive-agnostic — host election, xfer registry,
//! slice-paste tile derivation, accumulator classification, and the
//! per-pair FIFO host-mediated barrier guard all carry over verbatim.
//! The poll/bufsync difference lives EXCLUSIVELY in the emit layer
//! (the `wire::*_poll` call-site swap in `events.rs` + the
//! `apply_nonblocking` line in `worker_program.rs`).
//!
//! Sub-module map (all sibling files, all extending the same
//! `impl<'a> Plan<'a>` block):
//! - `worker_program.rs` — `Plan::render_worker_program` (the per-worker
//!   `src/bin/<name>.rs` body emitter).
//! - `events.rs` — `Plan::render_events` (the event-walk codegen,
//!   called from `render_worker_program`).
//! - `relay.rs` — `Plan::data_conn_var` / `Plan::relay_schedule` /
//!   `Plan::render_relay_phase` / `Plan::collect_pre_init` /
//!   `Plan::render_run_sh` / `Plan::max_payload_bytes`.

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
    /// XferPlaceholder. Drives the receiver-side slice-paste in
    /// `backend_common::multi_worker_walker::render_wait_assign`.
    /// Lifted to the shared helper in TASK-0296 cycle 116.
    pub(crate) pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343 cycle 189). mp-tcp-poll uses the same
    /// emit-time shape as mp-tcp-bufsync — direct render_wait_assign
    /// call from plan/events.rs (bypassing the shared walker because
    /// the per-pair FIFO topology needs different connection-variable
    /// rendering). The accumulate set is consulted at the Event::Wait
    /// emit site to pass the `accumulate: bool` flag.
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
        // `backend_common::host_election` for the canonical rule —
        // identical choice across every shipped tier-1 backend's
        // `multi_worker::Plan::build` AND the driver's pre-codegen
        // pass wirings (TASK-0336 cycle 164 lift). mp-tcp-poll joins
        // that set at TASK-0044.02.02 cycle 195.
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

        // Per-pair tiles for slice-aware Wait gathers (TASK-0296
        // cycle 116, hoisted to `collect_pair_tiles` in cycle 130).
        let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> =
            collect_pair_tiles(per_worker.values());

        // Barrier identity by the contract-carried `SyncTag`
        // (TASK-0172). Distinct tags are independent barriers, so a
        // partial/non-uniform barrier lowers correctly.
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
        // the mediating hub. True for the cycle-195 tier-1 set
        // (02-split: {host,w0}, etc). For host-excluding barriers
        // (e.g. 03-reduction/distributed) the driver applies
        // `apply_host_mediation_inject` BEFORE codegen, turning the
        // barrier into an N+1-party star through host — same dispatch
        // wiring as mp-tcp-bufsync + mp-tcp-event (cycle 195 widened
        // the gate to include mp-tcp-poll). This check below is
        // defense-in-depth: it should never fire for ACFGs that came
        // through the driver's pipeline; it still bites loud if an
        // upstream change ever removes the mediation pass.
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "barrier #{bid} participants {parts:?} exclude the host \
                     worker; mp-tcp-poll's one-connection-per-(host,worker) \
                     topology requires host as the barrier hub (same shape \
                     as mp-tcp-bufsync). The driver's apply_host_mediation_inject \
                     pass adds host as a mediating hub before codegen; this \
                     check fires only if that pass was removed/skipped."
                )));
            }
        }

        // Defensive ContractGap for the wait-before-push host-relay
        // deadlock — same shape as mp-tcp-bufsync TASK-0332 cycle 151.
        // mp-tcp-poll inherits bufsync's per-pair FIFO constraint;
        // apply_safe_push_reorder is wired on mp-tcp-event ONLY.
        detect_wait_before_push_hazard(per_worker, host_worker)?;

        // Per-worker overlapping-write accumulator classification
        // (TASK-0343 cycle 189) — same shape as mp-tcp-bufsync.
        let mut accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)> = BTreeSet::new();
        for w in &used_workers {
            let per_worker_set =
                collect_accumulate_waits(&per_worker[w], sidecar, &pair_tiles);
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

    /// Non-host used workers (every used worker in the tier-1 set), in
    /// WorkerId order.
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
