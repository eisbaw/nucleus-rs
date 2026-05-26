//! mp-tcp-poll backend. PRD §7.1 row 5, TASK-0044.02.
//!
//! Tier-1 backend: workers are OS PROCESSES, transport is TCP loopback,
//! notify is NONBLOCKING POLL (busy/yield wait loop), no buffer, sync.
//! **supports_async=false, supports_buffer=false** — same SCHEDULE-
//! visible capability surface as mp-tcp-bufsync. The cross-backend
//! differential gains a SECOND sync-TCP row; every schedule that
//! compiles against mp-tcp-bufsync MUST compile against mp-tcp-poll and
//! produce bit-identical output.
//!
//! # Implementation status (TASK-0044.02 cycle 174, 2026-05-26)
//!
//! - **SKELETON**: [`emit`] returns [`EmitError::ContractGap`] for
//!   ALL inputs (single- and multi-worker). The crate, capabilities
//!   file, public entry point, workspace registration, and driver
//!   dispatch arm are all wired through.
//! - The capability-compat check IS real. Schedules whose demands fall
//!   within the mp-tcp-poll capability surface (sync + tcp + poll/
//!   barrier/blocking) compat-check cleanly; the subsequent codegen
//!   call surfaces the skeleton via a typed [`EmitError::ContractGap`]
//!   forward-link.
//! - Substantive codegen (nonblocking-read wait loop + the
//!   mp-tcp-common wire framing + pthreads-sync-delegating single-
//!   worker arm + e2e cells) is the remaining surface of TASK-0044.02
//!   — landed in subsequent cycles.
//!
//! # Generated artefact layout (when substantive codegen lands)
//!
//! Identical to mp-tcp-bufsync's: under the user-provided `out_dir`,
//! `Cargo.toml` + `src/bin/<worker>.rs` (per used worker) + `src/wire.rs`
//! (copied wire protocol) + `src/kernels.rs` + `run.sh`. The multi-binary
//! shape is the same as mp-tcp-bufsync / mp-tcp-event.
//!
//! # Why the busy/yield poll loop?
//!
//! PRD §7.1 row 5 fixes the wait primitive as nonblocking poll. The
//! tradeoff vs blocking (mp-tcp-bufsync) is CPU usage during waits:
//! nonblocking-poll BURNS CPU, blocking yields it. The honest scope
//! note in TASK-0044.02 (cycle 171) commits to picking
//! `std::thread::yield_now` as the yield primitive when substantive
//! codegen lands — not a sleep (latency hit) and not pure busy-spin
//! (full-core burn). Hazard: a peer that never sends becomes a
//! spin-deadlock with no error; the codegen cycle must bound the
//! retry / add a deadline. See memory
//! project-mp-tcp-event-vs-bufsync-safety-profile for the analog
//! mask-failure-modes warning on the async sibling.
//!
//! # Honest limitations
//!
//! - SKELETON cycle. The only end-to-end behaviour today is the
//!   capability-compat success path followed by the
//!   `EmitError::ContractGap` for any actual codegen request. Until
//!   the substantive emit lands, no schedule runs end-to-end on this
//!   backend. Mirrors the openmp-rs (TASK-0044.01 cycle 173),
//!   pthreads-async (TASK-0042.01 cycle 16), and mp-tcp-event
//!   (TASK-0042.02 cycle 41 Stage 1) skeleton precedents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Paths to the files [`emit`] writes. Same shape as
/// `mp_tcp_bufsync::EmitResult` and `mp_tcp_event::EmitResult` because
/// mp-tcp-poll also produces a multi-process Cargo project (per-worker
/// binaries + shared wire.rs + run.sh). The shape is intentionally
/// identical for driver-side dispatch uniformity.
///
/// In the SKELETON cycle (TASK-0044.02 cycle 174) the struct exists so
/// the public surface is stable for downstream callers; [`emit`] does
/// not yet construct one (always returns [`EmitError::ContractGap`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Per-worker `src/bin/<worker>.rs` paths.
    pub worker_bins: Vec<PathBuf>,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `src/wire.rs` (the copied wire protocol).
    pub wire_rs: PathBuf,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

/// Emit a runnable multi-process Cargo project from the per-worker EventList.
///
/// **SKELETON (TASK-0044.02 cycle 174):** returns
/// [`EmitError::ContractGap`] for ALL inputs. Single- and multi-worker
/// codegen lands in subsequent cycles of TASK-0044.02. The capability-
/// compat check upstream of this call is REAL — a schedule that does
/// NOT compat-check against the mp-tcp-poll capabilities never reaches
/// this function.
///
/// Wire contract (mirrors mp-tcp-bufsync / mp-tcp-event):
/// consumes the per-worker [`Event`] lists + the [`NameTables`] (reverse
/// `name_*`) + the [`NameSidecar`]. No `&ACFG` / `&LinkedIR` access.
pub fn emit(
    _per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    _names: &NameTables,
    _sidecar: &NameSidecar,
    _kernels_rs_path: &Path,
    _out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    Err(EmitError::ContractGap(
        "mp-tcp-poll codegen: backend is in SKELETON phase (TASK-0044.02 cycle 174 \
         landed crate + capabilities.toml + driver dispatch + workspace member). \
         Substantive emit (nonblocking-read wait loop + mp-tcp-common wire framing + \
         pthreads-sync-delegating single-worker arm + e2e cells) lands in subsequent \
         cycles of TASK-0044.02. The schedule's capability compat-check has succeeded; \
         only the codegen body is outstanding."
            .to_string(),
    ))
}
