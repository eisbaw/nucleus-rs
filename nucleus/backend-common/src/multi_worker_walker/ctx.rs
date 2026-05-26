//! Walker context: the per-call bundle threaded through the shared
//! multi-worker event walker. Defines [`WalkerCtx`] (the per-backend
//! field set the walker reads) and [`RendezvousId`] (stable
//! `(DataId, SeqTag) -> usize` index shared with each backend's
//! per-backend rendezvous-id alias). See the parent
//! [`super`] module doc for the design rationale and per-backend
//! cross-walks.

use std::collections::BTreeMap;

use nucleus_compiler::event::{DataId, IterTile, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::render::{EmitError, RenderCtxPub};

/// Stable identifier for one rendezvous channel (slot or ring) keyed
/// by `(DataId, SeqTag)` ordered ascending. Same shape as the
/// per-backend `SlotId` / `RingId` aliases — both are `usize`, so the
/// map is shared.
pub type RendezvousId = usize;

/// Walker-time bundle of every fact the per-worker event walk needs.
///
/// Mirrors the per-backend `Plan` field set, but only the references
/// the walker actually reads — no ownership transfer, no copying. The
/// `rendezvous_prefix` field is the per-backend knob that distinguishes
/// the three prefix-using backends (`"slot"` for pthreads-sync, `"ring"`
/// for pthreads-async, `"chan"` for mp-tcp-event); mp-tcp-bufsync
/// bypasses `render_worker_events` entirely and calls `render_wait_assign`
/// directly with no prefix involvement. Every other field is shared
/// verbatim across all `render_worker_events`-using backends.
///
/// Grep witness (cycle 141 TASK-0322 fold-back, line stamps re-
/// verified cycle 142b fold-back after qa P2 caught off-by-+2/+6
/// drift): `grep -n 'rendezvous_prefix:'
/// nucleus/backends/*/src/multi_worker.rs` yields exactly three
/// field-init sites (pthreads-sync `"slot"` line 538, pthreads-
/// async `"ring"` line 522, mp-tcp-event `"chan"` line 493).
pub struct WalkerCtx<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// `"slot"` for pthreads-sync, `"ring"` for pthreads-async,
    /// `"chan"` for mp-tcp-event. Used in the two emit-string
    /// substitutions in this file: `{prefix}{rendezvous_prefix}_{id}.push(...)`
    /// (the `Event::Push` branch) and `{prefix}{rendezvous_prefix}_{id}.wait()`
    /// (the `Event::Wait` branch, fed into `render_wait_assign`).
    ///
    /// Grep witness (cycle 141 TASK-0322 fold-back, line stamps
    /// updated cycle 142 TASK-0323 module-doc sweep): `grep -n
    /// '{rendezvous_prefix}_' nucleus/backend-common/src/` yields
    /// exactly two emit-template sites (Push at line 833, Wait at
    /// line 853 — both pinned parametrically by
    /// `task0321_*` / `task0322_*` in
    /// `nucleus/backend-common/tests/wait_assign_slice.rs`).
    pub rendezvous_prefix: &'a str,
    /// Cross-worker Push/Wait pair -> rendezvous index. Both backends
    /// key by `(DataId, SeqTag)` and assign indices ascending.
    pub rendezvous_ids: &'a BTreeMap<(DataId, SeqTag), RendezvousId>,
    /// Per-pair iteration tile from the originating XferPlaceholder
    /// (TASK-0117). Drives the receiver-side leading-axis slice-paste
    /// in `render_wait_assign` (1D leading-axis path TASK-0117 + 2D
    /// row-loop path TASK-0294).
    pub pair_tiles: &'a BTreeMap<(DataId, SeqTag), IterTile>,
}

impl WalkerCtx<'_> {
    /// Render context for the shared expression renderers
    /// (`render_fire_args_pub`, `render_const_expr_pub`, etc.).
    pub(super) fn render_ctx(&self) -> RenderCtxPub<'_> {
        RenderCtxPub::new(self.names, self.sidecar)
    }

    /// Worker name from the reverse NameTables, falling back to
    /// `w<id>` if the table is missing the entry (defensive — should
    /// never happen for an in-Plan WorkerId).
    pub(super) fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    /// Data name lookup; fails LOUD via [`EmitError::ContractGap`]
    /// when a DataId in the event stream has no name in the tables.
    pub(super) fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }
}
