//! openmp-rs backend. PRD §7.1 row 1, TASK-0044.01.
//!
//! Tier-1 CPU backend: rayon threads + shared-memory transport +
//! barrier-style notify. **supports_async=false, supports_buffer=false**
//! — same SCHEDULE-visible capability surface as pthreads-sync. The
//! cross-backend differential gains a THIRD sync-shared-memory row;
//! every schedule that compiles against pthreads-sync MUST compile
//! against openmp-rs and produce bit-identical output.
//!
//! # Implementation status (TASK-0044.01 cycle 173, 2026-05-26)
//!
//! - **SKELETON**: [`emit`] returns [`EmitError::ContractGap`] for
//!   ALL inputs (single- and multi-worker). The crate, capabilities
//!   file, public entry point, workspace registration, and driver
//!   dispatch arm are all wired through.
//! - The capability-compat check IS real. Schedules whose demands fall
//!   within the openmp-rs capability surface (sync + shared-memory +
//!   barrier/blocking) compat-check cleanly; the subsequent codegen
//!   call surfaces the skeleton via a typed [`EmitError::ContractGap`]
//!   forward-link.
//! - Substantive codegen (rayon scope / par_iter for the multi-worker
//!   arm; pthreads-sync-delegating single-worker arm; e2e cells) is the
//!   remaining surface of TASK-0044.01 — landed in subsequent cycles.
//!
//! # Generated artefact layout (when substantive codegen lands)
//!
//! Identical to pthreads-sync's: under the user-provided `out_dir`,
//! `Cargo.toml` + `src/main.rs` + `src/kernels.rs` + `run.sh`. The
//! single-binary shape is the same as pthreads-sync / pthreads-async;
//! single-worker emit will delegate to the shared
//! `pthreads_sync::render_single_worker_main` (the same byte-identical
//! pattern pthreads-async uses).
//!
//! # Why the name "openmp-rs"?
//!
//! Metaphorical. There is no C OpenMP runtime, no `#pragma omp parallel`
//! — the name signals "parallel-for shared-memory" semantics, matching
//! how `rayon` is used in practice. PRD §7.1 row 1 fixes the name; this
//! crate follows.
//!
//! # Honest limitations
//!
//! - SKELETON cycle. The only end-to-end behaviour today is the
//!   capability-compat success path (a compatible schedule is accepted)
//!   followed by the `EmitError::ContractGap` for any actual codegen
//!   request. Until the substantive emit lands, the only schedules
//!   this backend "supports" are ones that are blocked at codegen by
//!   the ContractGap — i.e. nothing runs end-to-end. Mirrors the
//!   pthreads-async (TASK-0042.01 cycle 16) and mp-tcp-event
//!   (TASK-0042.02 cycle 41 Stage 1) skeleton precedents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Paths to the files [`emit`] writes. Same shape as
/// `pthreads_sync::EmitResult` and `pthreads_async::EmitResult` because
/// openmp-rs also produces a single Cargo project + single binary
/// (shared-memory transport). The shape is intentionally identical for
/// driver-side dispatch uniformity.
///
/// In the SKELETON cycle (TASK-0044.01 cycle 173) the struct exists so
/// the public surface is stable for downstream callers; [`emit`] does
/// not yet construct one (always returns [`EmitError::ContractGap`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Path to the emitted `src/main.rs`.
    pub main_rs: PathBuf,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

/// Emit a runnable Cargo project from the per-worker EventList.
///
/// **SKELETON (TASK-0044.01 cycle 173):** returns
/// [`EmitError::ContractGap`] for ALL inputs. Single- and multi-worker
/// codegen lands in subsequent cycles of TASK-0044.01. The capability-
/// compat check upstream of this call is REAL — a schedule that does
/// NOT compat-check against the openmp-rs capabilities never reaches
/// this function.
///
/// Wire contract (mirrors pthreads-sync / pthreads-async / mp-tcp-event):
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
        "openmp-rs codegen: backend is in SKELETON phase (TASK-0044.01 cycle 173 \
         landed crate + capabilities.toml + driver dispatch + workspace member). \
         Substantive emit (rayon-based multi-worker; pthreads-sync-delegating \
         single-worker; e2e cells) lands in subsequent cycles of TASK-0044.01. \
         The schedule's capability compat-check has succeeded; only the codegen \
         body is outstanding."
            .to_string(),
    ))
}
