//! The shared multi-process emit `Plan` for the sync-TCP backends.
//!
//! `Plan<'a, W>` holds every cross-worker invariant codegen depends on:
//! the used-worker list, host election, cross-worker `XferId` registry,
//! per-`(DataId, SeqTag)` `IterTile` map for slice-paste-aware Wait
//! gathers, the overlapping-write accumulator set, plus typed
//! accessors. The `W: WirePrimitives` type parameter is the SOLE axis
//! of variation between mp-tcp-bufsync (blocking) and mp-tcp-poll
//! (nonblocking poll); `Plan` carries no `W` value, only a
//! `PhantomData<fn() -> W>`, and dispatches the wire-primitive
//! variation through `W::method(..)` at the emit sites in `events.rs`,
//! `relay.rs`, and `worker_program.rs` (sibling files).
//!
//! Lifted from the two backends' verbatim-duplicate `plan/mod.rs`
//! (TASK-0044.02.03).

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::elect_host_from_worker_names;
use crate::multi_worker_walker::{collect_accumulate_waits, collect_pair_tiles};
use crate::tcp_plan::walkers::{
    collect_barriers_by_tag, collect_xfer_data, detect_wait_before_push_hazard,
};
use crate::tcp_plan::WirePrimitives;
use crate::EmitError;

/// Stable identifier for one cross-worker data symbol, by sorted
/// `DataId` (deterministic; same order pthreads-sync's slot ids use).
pub type XferId = usize;

/// Multi-process emit plan parameterised over the per-backend wire
/// primitives `W`. See the module docstring.
pub struct Plan<'a, W: WirePrimitives> {
    pub per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    pub used_workers: Vec<WorkerId>,
    pub host_worker: WorkerId,
    /// Cross-worker data symbols sorted by DataId.
    pub xfer_ids: BTreeMap<DataId, XferId>,
    /// Per-(DataId,SeqTag) iteration tile from the originating
    /// XferPlaceholder. Drives the receiver-side leading-axis / 2D
    /// row-loop slice-paste in
    /// [`crate::multi_worker_walker::render_wait_assign`]. Both
    /// sync-TCP backends bypass the shared event walker and call
    /// `render_wait_assign` directly from `events.rs`, so they consume
    /// the same pair-tile map populated here (TASK-0296 cycle 116).
    pub pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343 cycle 189). Computed at build time the
    /// same way the other tier-1 backends do
    /// (`collect_accumulate_waits` per worker, unioned with WorkerId)
    /// and consulted at the Event::Wait emit site to pass the
    /// `accumulate: bool` flag to the shared `render_wait_assign`
    /// helper. Empty for every cell without an overlapping-write fan-in.
    pub accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
    /// Zero-sized witness of the per-backend wire primitives. `Plan`
    /// has no `W` value; the variation is dispatched through `W`'s
    /// associated functions/consts at the emit sites.
    pub(crate) _wire: PhantomData<fn() -> W>,
}

impl<'a, W: WirePrimitives> Plan<'a, W> {
    pub fn build(
        per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
        names: &'a NameTables,
        sidecar: &'a NameSidecar,
    ) -> Result<Self, EmitError> {
        let used_workers: Vec<WorkerId> = per_worker
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(w, _)| *w)
            .collect();

        // Host election: shared helper. See `crate::host_election`
        // module docstring for the canonical rule. IDENTICAL choice
        // across every shipped tier-1 backend's `Plan::build` AND the
        // three compiler-level driver wirings (cycles 160 / 162 / 163);
        // the helper is the single source of truth (TASK-0336 cycle
        // 164 lift). The cross-backend bit-identical differential (PRD
        // §10.1) needs every backend to elect the same host given the
        // same input.
        let host_worker =
            elect_host_from_worker_names(&names.worker, &used_workers).ok_or_else(|| {
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

        // Per-pair tiles for slice-aware Wait gathers (TASK-0296 cycle
        // 116, hoisted to `collect_pair_tiles` in cycle 130 per
        // TASK-0300). The shared helper preserves deterministic first-
        // sighting-wins on `(DataId, SeqTag)`; both endpoints carry the
        // same tile by XferPlaceholder construction (TASK-0018).
        let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> =
            collect_pair_tiles(per_worker.values());

        // Barrier identity by the contract-carried `SyncTag` (TASK-
        // 0172). Each Event::Sync names its own barrier; the projection
        // clones the same participant set into every participant's
        // list, so the first sighting of a tag fixes its participants.
        // No pre-order-index heuristic and no uniform-barrier
        // validation: distinct tags are independent barriers, so a
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
        // the mediating hub. The driver's `apply_host_mediation_inject`
        // pass (TASK-0329, Done cycle 160) adds host as a mediating hub
        // BEFORE codegen for host-excluding barriers, turning each into
        // an N+1-party star through host. This check is therefore
        // defense-in-depth: it should never fire for ACFGs that came
        // through the driver's pipeline; if an upstream change ever
        // removes the mediation pass it bites loud, never a wrong
        // binary. The message names the backend via `W::BACKEND_NAME`
        // and contains the test-pinned substring "exclude the host
        // worker".
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "barrier #{bid} participants {parts:?} exclude the host \
                     worker; {backend}'s one-connection-per-(host,worker) \
                     topology requires host as the barrier hub. The driver's \
                     apply_host_mediation_inject pass adds host as a \
                     mediating hub before codegen; this check fires only if \
                     that pass was removed/skipped (filed historically as \
                     TASK-0175).",
                    backend = W::BACKEND_NAME,
                )));
            }
        }

        // TASK-0332 (cycle 151 AC#2): defensive ContractGap for the
        // wait-before-push host-relay deadlock. Conservative-but-sound:
        // rejects every schedule whose first top-level w2w event for
        // ANY non-host worker is a Wait. No in-tree sync-TCP schedule
        // triggers this today; the guard remains for fail-loud hygiene
        // if a future capability lift exposes a sync-TCP-compatible
        // wait-before-push schedule. See `walkers::detect_wait_before_
        // push_hazard` for the full design narrative.
        detect_wait_before_push_hazard::<W>(per_worker, host_worker)?;

        // Per-worker overlapping-write accumulator classification
        // (TASK-0343 cycle 189) — mirrors the other tier-1 backends'
        // `accumulate_waits` computation field-for-field. The emit-time
        // consultation lives in `events.rs` at the Event::Wait branch
        // (both sync-TCP backends bypass the shared walker).
        let mut accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)> = BTreeSet::new();
        for w in &used_workers {
            let per_worker_set = collect_accumulate_waits(&per_worker[w], sidecar, &pair_tiles);
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
            _wire: PhantomData,
        })
    }

    pub fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    pub fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }

    /// Non-host used workers that exchange anything with the host
    /// (every used worker, in the tier-1 set), in WorkerId order.
    pub fn non_host_workers(&self) -> Vec<WorkerId> {
        self.used_workers
            .iter()
            .copied()
            .filter(|w| *w != self.host_worker)
            .collect()
    }

    /// Control-channel variable a given worker uses to barrier with
    /// `peer`. Host: `ctrl_<peer>`; non-host worker: `ctrl_host`.
    pub fn ctrl_var(&self, self_is_host: bool, peer: WorkerId) -> String {
        if self_is_host {
            format!("ctrl_{}", self.worker_name(peer))
        } else {
            "ctrl_host".to_string()
        }
    }
}
