//! openmp-rs backend. PRD §7.1 row 1, TASK-0044.01.
//!
//! Tier-1 CPU backend: rayon threads + shared-memory transport +
//! barrier-style notify. **supports_async=false, supports_buffer=false**
//! — same SCHEDULE-visible capability surface as pthreads-sync. The
//! cross-backend differential gains a THIRD sync-shared-memory row;
//! every schedule that compiles against pthreads-sync MUST compile
//! against openmp-rs and produce bit-identical output.
//!
//! # Implementation status (TASK-0044.01 cycle 191, 2026-05-27)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`) is IMPLEMENTED.
//!   Delegates to `pthreads_sync::render_single_worker_main` plus the
//!   SHARED `backend_common::project_skeleton::single_binary::
//!   {render_cargo_toml, render_run_sh}` (TASK-0246), so the emitted
//!   artefact is BYTE-IDENTICAL to pthreads-sync's and pthreads-async's
//!   single-worker output for any naive schedule. The cross-backend
//!   differential invariant ("same algorithm + same naive schedule ->
//!   bit-identical output across backends") holds by construction.
//! - **Multi-worker arm** (`used_workers.len() >= 2`) is NOT YET
//!   implemented. Returns [`EmitError::ContractGap`] forward-linking
//!   the multi-worker follow-up sub-task of TASK-0044.01 (the rayon-
//!   scope / per-pair barrier codegen — the headline rayon work).
//!
//! # Why split single-worker vs multi-worker into separate cycles
//!
//! Single-worker openmp-rs has NO cross-worker `Push`/`Wait` events
//! (the pthreads-sync single-worker emitter `ContractGap`s on either;
//! same is true here, inherited via delegation). The rayon-scope
//! machinery only fires across two or more workers. Splitting the
//! single-worker arm off keeps it a genuinely single-cycle unit
//! (mechanical delegation, no new runtime substrate) and quarantines
//! the multi-cycle rayon-scope headline work under the multi-worker
//! follow-up. Identical precedent to pthreads-async's
//! TASK-0226 -> TASK-0228 split.
//!
//! # Generated artefact layout
//!
//! Identical to pthreads-sync's: under the user-provided `out_dir`,
//! `Cargo.toml` + `src/main.rs` + `src/kernels.rs` + `run.sh`. The
//! `[[bin]]` name + the package name are backend-agnostic so the
//! generated project is movable.
//!
//! # Why the name "openmp-rs"?
//!
//! Metaphorical. There is no C OpenMP runtime, no `#pragma omp parallel`
//! — the name signals "parallel-for shared-memory" semantics, matching
//! how `rayon` is used in practice. PRD §7.1 row 1 fixes the name; this
//! crate follows.
//!
//! # Error handling
//!
//! Failures bubble up as [`EmitError`] variants (re-exported from
//! `backend-common` — same shared-surface precedent as pthreads-async;
//! the user-visible `Display` text carries no per-backend prefix, and
//! the dispatch site in `nucleus/driver/src/main.rs` wraps each with
//! "openmp-rs codegen error:" so the surfaced error reads cleanly).
//!
//! # Honest limitations
//!
//! - Multi-worker codegen is `ContractGap`. Until the follow-up lands,
//!   the *only* schedules this backend runs end-to-end are those whose
//!   worker count is 0 or 1 — which is already covered by pthreads-sync
//!   and pthreads-async. Single-worker openmp-rs is therefore a
//!   *capability-check gateway* (capability surface satisfiable) more
//!   than a new runtime — the real value lands when the multi-worker
//!   rayon-scope codegen lands.
//! - Multi-worker SYNC schedules (02/split, 03/distributed,
//!   06/distributed, 06/distributed2, 07/distributed, 07/distributed-2d,
//!   08/distributed, 13/batch_parallel) WOULD compile under the
//!   openmp-rs capability surface but currently hit the multi-worker
//!   ContractGap — promote via TASK-0044.01.01.
//! - Capability-mismatch schedules (05/distributed, 05/distributed-2d,
//!   09/pipelined, 11/pipelined, 13/pipeline_parallel) — async +
//!   buffer + event — are rejected upstream at the capability-compat
//!   check, NOT at codegen, and stay [[skip]] forever per PRD §7.1
//!   row openmp-rs (sync + barrier capability surface is pinned).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

// Shared project-skeleton renderers — same single source of truth as
// pthreads-sync and pthreads-async (TASK-0246, backend_common::
// project_skeleton::single_binary). Using these IS how the cross-
// backend differential invariant on naive schedules holds: openmp-rs's
// emitted Cargo.toml + run.sh are byte-equal to the other single-
// binary backends' for any single-worker schedule.
use backend_common::project_skeleton::single_binary::{render_cargo_toml, render_run_sh};

// Single-worker main.rs body — the only inter-backend arrow that
// genuinely is a semantic delegation (NOT inert string templating).
// See `backend_common::project_skeleton` module-doc for the rationale
// for keeping this in pthreads-sync vs lifting it.
use pthreads_sync::render_single_worker_main;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Paths to the files [`emit`] writes. Same shape as
/// `pthreads_sync::EmitResult` and `pthreads_async::EmitResult` because
/// openmp-rs also produces a single Cargo project + single binary
/// (shared-memory transport). The shape is intentionally identical for
/// driver-side dispatch uniformity.
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
/// Wire contract: consumes the per-worker [`Event`] lists + the
/// [`NameTables`] (reverse `name_*`) + the [`NameSidecar`]. **No
/// `&ACFG` / `&LinkedIR` access**, exactly like pthreads-sync /
/// pthreads-async / mp-tcp-bufsync.
///
/// Dispatch:
///
/// - `used_workers <= 1` → SINGLE-WORKER. Delegates to the SHARED
///   `pthreads_sync::render_single_worker_main` so the emitted
///   `main.rs` is byte-identical to pthreads-sync's. The Cargo.toml
///   and run.sh come from `backend_common::project_skeleton::
///   single_binary::{render_cargo_toml, render_run_sh}` for the same
///   reason.
/// - `used_workers >= 2` → MULTI-WORKER. Returns
///   [`EmitError::ContractGap`] pointing at the rayon-scope multi-
///   worker follow-up sub-task of TASK-0044.01.
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // ---- Pick code path: single- vs multi-worker. ----
    //
    // `acfg_to_events` seeds every declared worker with an empty list
    // so an unused-but-declared worker does not falsely trip the
    // multi-worker path. Same `collect_used_workers` semantics as
    // pthreads-sync and pthreads-async.
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    if used_workers.len() >= 2 {
        // ---- Multi-worker arm: rayon-scope codegen (NOT YET LANDED). ----
        return Err(EmitError::ContractGap(
            "openmp-rs codegen: multi-worker arm (used_workers >= 2) is \
             not yet implemented — the rayon-scope / per-pair barrier \
             codegen is the headline follow-up of TASK-0044.01, filed as \
             TASK-0044.01.01. The schedule's capability compat-check has \
             succeeded; only the multi-worker codegen body is outstanding. \
             Single-worker schedules (used_workers <= 1) ARE supported \
             and emit byte-identical artefacts to pthreads-sync / \
             pthreads-async."
                .to_string(),
        ));
    }

    // ---- Single-worker arm (TASK-0044.01 cycle 191). ----
    //
    // Delegate to the SHARED renderers in pthreads-sync +
    // backend-common. The emitted main.rs is byte-identical to
    // pthreads-sync's by construction (same function, same inputs);
    // Cargo.toml + run.sh ditto. The single-worker check_frame codegen
    // (Panic / Log / Count) is inherited from pthreads-sync's
    // render_single_worker_main → render_main_rs path automatically —
    // no per-backend Log/Count emit is needed for the single-worker
    // case.
    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    let src_dir = out_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let kernels_rs = src_dir.join("kernels.rs");
    let run_sh = out_dir.join("run.sh");

    let events = used_workers
        .first()
        .and_then(|w| per_worker.get(w))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let main_rs_src = render_single_worker_main(events, names, sidecar)?;

    write_file(&cargo_toml, &render_cargo_toml())?;
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&main_rs, &main_rs_src)?;
    write_file(&run_sh, &render_run_sh())?;

    // Best-effort: mark run.sh executable. Failure here is non-fatal
    // (mirrors pthreads-sync / pthreads-async precedent).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(&run_sh) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&run_sh, perms);
        }
    }

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        main_rs,
        kernels_rs,
        run_sh,
    })
}

/// Write `content` to `path`, mapping io errors to
/// [`EmitError::WriteFailed`] with the offending path attached.
fn write_file(path: &Path, content: &str) -> Result<(), EmitError> {
    fs::write(path, content).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
