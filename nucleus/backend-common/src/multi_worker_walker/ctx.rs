//! Walker context: the per-call bundle threaded through the shared
//! multi-worker event walker. Defines [`WalkerCtx`] (the per-backend
//! field set the walker reads) and [`RendezvousId`] (stable
//! `(DataId, SeqTag) -> usize` index shared with each backend's
//! per-backend rendezvous-id alias). See the parent
//! [`super`] module doc for the design rationale and per-backend
//! cross-walks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

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
/// `rendezvous_prefix` field is the per-backend knob that names the
/// generated rendezvous variable (`"slot"`, `"ring"`, `"mpi"`, …);
/// mp-tcp-bufsync bypasses `render_worker_events` entirely and calls
/// `render_wait_assign` directly with no prefix involvement. Every
/// other field is shared verbatim across all `render_worker_events`-
/// using backends.
///
/// The authoritative (non-stale) list of prefix values is whatever the
/// field-init sites set — `grep -rn 'rendezvous_prefix:' nucleus/backends`
/// (line numbers deliberately omitted here: precomputed citations went
/// stale repeatedly, see memory `feedback-comment-doc-lie-recurring`).
pub struct WalkerCtx<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// The per-backend rendezvous variable-name prefix (e.g. `"slot"`
    /// for pthreads-sync / openmp-rs, `"ring"` for pthreads-async,
    /// `"chan"` for the mp-tcp/uds event backends, `"mpi"` for
    /// mpi-blocking). Used in the two emit-string substitutions in the
    /// sibling `event_walker` module (`super::event_walker`):
    /// `{prefix}{rendezvous_prefix}_{id}.push(...)` (the `Event::Push`
    /// branch) and `{prefix}{rendezvous_prefix}_{id}.wait()` (the
    /// `Event::Wait` branch, fed into `render_wait_assign`). The two
    /// emit-template sites are pinned parametrically by `task0321_*` /
    /// `task0322_*` in `nucleus/backend-common/tests/wait_assign_slice.rs`
    /// (`grep -rn '{rendezvous_prefix}_' nucleus/backend-common/src` for
    /// the current locations — line numbers omitted, they drift).
    pub rendezvous_prefix: &'a str,
    /// Cross-worker Push/Wait pair -> rendezvous index. Both backends
    /// key by `(DataId, SeqTag)` and assign indices ascending.
    pub rendezvous_ids: &'a BTreeMap<(DataId, SeqTag), RendezvousId>,
    /// Per-pair iteration tile from the originating XferPlaceholder
    /// (TASK-0117). Drives the receiver-side leading-axis slice-paste
    /// in `render_wait_assign` (1D leading-axis path TASK-0117 + 2D
    /// row-loop path TASK-0294).
    pub pair_tiles: &'a BTreeMap<(DataId, SeqTag), IterTile>,
    /// Per-(worker, data, seq) accumulate fan-in classification
    /// (TASK-0343, cycle 189). A `(worker, data, seq)` triple in this
    /// set means the per-worker Event::Wait MUST emit element-wise
    /// `wrapping_add` accumulate (sum identity) instead of the default
    /// whole-array overwrite assign. The set is computed per-Plan by
    /// each backend via `collect_accumulate_waits` over each worker's
    /// projected events and then unioned with the WorkerId.
    ///
    /// Empty by default — preserves pre-cycle-189 emit for every
    /// `(worker, data, seq)` not classified as a fan-in accumulator.
    pub accumulate_waits: &'a BTreeSet<(WorkerId, DataId, SeqTag)>,
    /// Per-worker classification (TASK-0349, cycle 220) of DataIds
    /// whose pre-init `let mut <name>: Vec<..> = vec![0; N];` is
    /// provably dead because every Wait of them is a whole-array
    /// recv (and they are NOT accumulate-fan-in nor indexed-Fire-
    /// written). For these data the walker emits
    /// `let <name> = <rhs>;` (declare-and-assign) at the first Wait
    /// site, and the per-backend pre-init pass omits them. Computed
    /// per-(Plan, worker) via `collect_let_at_wait_data` (sibling
    /// `collect.rs`). The set is keyed on `DataId` alone — Plan-side
    /// each worker has its own set, so passing-by-reference per
    /// worker is the natural shape.
    ///
    /// Empty by default — preserves pre-cycle-220 emit (`<name> =
    /// <rhs>;`) for every Wait whose `data` is not in this set.
    pub let_at_wait_data: &'a BTreeSet<DataId>,
}

impl WalkerCtx<'_> {
    /// Render context for the shared expression renderers
    /// (`render_fire_args_pub`, `render_const_expr_pub`, etc.).
    pub(super) fn render_ctx(&self) -> RenderCtxPub<'_> {
        RenderCtxPub::new(self.names, self.sidecar)
    }

    /// Shared static empty accumulate-waits set (TASK-0343 cycle 189).
    /// Convenience for tests + non-multi-worker call sites that have
    /// no overlapping-write fan-in to classify — pre-cycle-189 emit
    /// is identical when this set is empty, so passing this helper
    /// is the "no accumulate" default. Production multi-worker
    /// Plan::build builds its own per-Plan accumulate_waits set via
    /// `collect_accumulate_waits` (sibling `collect.rs`).
    pub fn empty_accumulate_set() -> &'static BTreeSet<(WorkerId, DataId, SeqTag)> {
        static EMPTY: OnceLock<BTreeSet<(WorkerId, DataId, SeqTag)>> = OnceLock::new();
        EMPTY.get_or_init(BTreeSet::new)
    }

    /// Shared static empty let-at-wait set (TASK-0349 cycle 220).
    /// Convenience for tests + non-multi-worker call sites that have
    /// no whole-array-recv pre-init candidates to classify — pre-
    /// cycle-220 emit is identical when this set is empty, so passing
    /// this helper is the "no let-at-wait" default. Production multi-
    /// worker Plan::build builds its own per-worker let_at_wait set
    /// via `collect_let_at_wait_data` (sibling `collect.rs`).
    pub fn empty_let_at_wait_set() -> &'static BTreeSet<DataId> {
        static EMPTY: OnceLock<BTreeSet<DataId>> = OnceLock::new();
        EMPTY.get_or_init(BTreeSet::new)
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
