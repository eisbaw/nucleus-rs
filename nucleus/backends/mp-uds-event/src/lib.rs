//! mp-uds-event backend. PRD §7.1 row 7, TASK-0044.03.
//!
//! Tier-1 backend: workers are OS PROCESSES, transport is Unix domain
//! sockets, notify is mio/epoll event readiness, supports async +
//! buffer. **supports_async=true, supports_buffer=true** — same
//! SCHEDULE-visible capability surface as mp-tcp-event. The cross-
//! backend differential gains a FOURTH async+buffered+event row
//! (alongside pthreads-async + mp-tcp-event) and the FIRST UDS row.
//!
//! # Implementation status (TASK-0044.03 cycle 175, 2026-05-26)
//!
//! - **SKELETON**: [`emit`] returns [`EmitError::ContractGap`] for
//!   ALL inputs. The crate, capabilities file, public entry point,
//!   workspace registration, and driver dispatch arm are all wired
//!   through.
//! - The capability-compat check IS real. Schedules whose demands fall
//!   within the mp-uds-event capability surface (async + uds + event)
//!   compat-check cleanly; the subsequent codegen call surfaces the
//!   skeleton via a typed [`EmitError::ContractGap`] forward-link.
//! - Substantive codegen (Unix domain socket reactor + per-(DataId,
//!   SeqTag) ring buffer + epoll readiness + pthreads-sync-delegating
//!   single-worker arm + e2e cells) is the remaining surface of
//!   TASK-0044.03 — landed in subsequent cycles.
//!
//! # Generated artefact layout (when substantive codegen lands)
//!
//! Identical to mp-tcp-event's: under the user-provided `out_dir`,
//! `Cargo.toml`, `src/bin/<worker>.rs` (per used worker), `src/wire.rs`,
//! `src/runtime.rs` (the mio reactor + ring buffer + chan substrate;
//! only present on multi-worker emit), `src/kernels.rs`, and `run.sh`.
//!
//! # Why Unix domain sockets?
//!
//! Same multi-process model as mp-tcp-event, but using filesystem
//! socket paths instead of TCP loopback. Two wins per the cycle-171
//! brief: (a) faster than TCP-loopback (no IP stack), (b) no
//! port-picker dance (filesystem paths are deterministic, scratch-dir
//! scoped). mio's `UnixListener` / `UnixStream` share the `Source`
//! trait with `TcpListener` / `TcpStream` so the reactor surface stays
//! identical — the cycle-171 brief flags the codegen cycle as a
//! CANDIDATE FOR LIFTING (if mp-tcp-event's reactor is parametric
//! over listener type via a trait, the lift may be cheap; the
//! decision is deferred to the codegen cycle per phased-AC policy).
//!
//! # TASK-0337 promotion-trigger guard (inherited from cycle-171 AC#9)
//!
//! mp-uds-event INHERITS the (D)+(B2) compensating-pass tower from
//! mp-tcp-event (cycles 149/151/162/163/165: host-relay + defensive
//! guard + reorder + ACFG relay-inject + 13-arm). IF a UDS-specific
//! schedule shape forces a 5th compensating pass to land for
//! mp-uds-event to be bit-identical, that IS the promotion trigger
//! named in TASK-0337's "Promotion triggers" section. STOP and
//! consult before adding compensating pass #5. The honest outcome is
//! to PROMOTE TASK-0337 (Option E full w↔w mesh, the root-cause
//! TASK-0175 closure) and obsolete the tower, NOT silently grow it.
//!
//! # Honest limitations
//!
//! - SKELETON cycle. The only end-to-end behaviour today is the
//!   capability-compat success path followed by the
//!   `EmitError::ContractGap` for any actual codegen request. Mirrors
//!   the openmp-rs (cycle 173) + mp-tcp-poll (cycle 174) skeleton
//!   precedents.
//! - UDS path-length cap (~104 chars on macOS, ~108 on Linux). Not
//!   relevant at the skeleton cycle (no socket is bound); the codegen
//!   cycle's run.sh must check + reject paths exceeding the cap. Per
//!   cycle-171 AC#8.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Paths to the files [`emit`] writes. Same shape as
/// `mp_tcp_event::EmitResult` — multi-binary with an optional
/// `runtime_rs` (only present on multi-worker emit, per the
/// mp-tcp-event precedent at TASK-0042.05 cycle 79).
///
/// In the SKELETON cycle (TASK-0044.03 cycle 175) the struct exists so
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
    /// Path to the emitted `src/wire.rs`.
    pub wire_rs: PathBuf,
    /// Path to the emitted `src/runtime.rs` (the mio reactor + ring
    /// buffer substrate). Only present on the multi-worker emit path;
    /// single-worker emit skips this file because the delegated
    /// single-worker renderer has no cross-worker Push/Wait.
    pub runtime_rs: Option<PathBuf>,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

/// Emit a runnable multi-process Cargo project from the per-worker EventList.
///
/// **SKELETON (TASK-0044.03 cycle 175):** returns
/// [`EmitError::ContractGap`] for ALL inputs. Single- and multi-worker
/// codegen lands in subsequent cycles of TASK-0044.03. The capability-
/// compat check upstream of this call is REAL — a schedule that does
/// NOT compat-check against the mp-uds-event capabilities never
/// reaches this function.
pub fn emit(
    _per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    _names: &NameTables,
    _sidecar: &NameSidecar,
    _kernels_rs_path: &Path,
    _out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    Err(EmitError::ContractGap(
        "mp-uds-event codegen: backend is in SKELETON phase (TASK-0044.03 cycle 175 \
         landed crate + capabilities.toml + driver dispatch + workspace member). \
         Substantive emit (Unix domain socket reactor + per-(DataId, SeqTag) ring \
         buffer + epoll readiness + pthreads-sync-delegating single-worker arm + \
         e2e cells) lands in subsequent cycles of TASK-0044.03. The schedule's \
         capability compat-check has succeeded; only the codegen body is outstanding."
            .to_string(),
    ))
}
