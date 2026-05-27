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
//! # Implementation status (TASK-0044.02 cycle 192, 2026-05-27)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`) is IMPLEMENTED.
//!   Delegates to `pthreads_sync::render_single_worker_main_with_kernels_attr`
//!   plus `backend_common::project_skeleton::multi_binary::{render_cargo_toml,
//!   render_run_sh_single}` — the SAME shared renderers
//!   mp-tcp-bufsync's single-process arm consumes. Emitted artefact is
//!   BYTE-IDENTICAL to mp-tcp-bufsync's single-process output (and
//!   therefore arithmetic byte-identical to pthreads-sync's single
//!   process). The cross-backend differential invariant ("same
//!   algorithm + same naive schedule -> bit-identical output across
//!   backends") holds by construction.
//! - **Multi-worker arm** (`used_workers.len() >= 2`) is NOT YET
//!   implemented. Returns [`EmitError::ContractGap`] forward-linking
//!   the multi-worker follow-up sub-task of TASK-0044.02
//!   (TASK-0044.02.02 — the nonblocking-read poll loop + wire framing
//!   + Plan-shaped per-worker codegen).
//!
//! # Why split single-worker vs multi-worker into separate cycles
//!
//! Single-worker mp-tcp-poll has NO cross-worker `Push`/`Wait` events
//! (mp-tcp-bufsync's single-process arm sidesteps wire framing
//! entirely; same is true here). The nonblocking-poll wait primitive
//! only fires across two or more workers. Splitting the single-worker
//! arm off keeps it a genuinely single-cycle unit (mechanical
//! delegation, no new runtime substrate) and quarantines the multi-
//! cycle nonblocking-poll headline work under TASK-0044.02.02. Same
//! precedent as TASK-0226 (pthreads-async single-worker) →
//! TASK-0228 (pthreads-async multi-worker) and TASK-0044.01 cycle 191
//! (openmp-rs single-worker) → TASK-0044.01.01 (openmp-rs
//! multi-worker).
//!
//! # Generated artefact layout
//!
//! Identical to mp-tcp-bufsync's: under the user-provided `out_dir`,
//! `Cargo.toml` + `src/bin/<worker>.rs` (per used worker; for
//! single-worker just `src/bin/nuc-generated.rs`) + `src/wire.rs`
//! (copied wire-v0 protocol) + `src/kernels.rs` + `run.sh`. The
//! multi-binary shape is the same as mp-tcp-bufsync / mp-tcp-event.
//!
//! # Why the busy/yield poll loop?
//!
//! PRD §7.1 row 5 fixes the wait primitive as nonblocking poll. The
//! tradeoff vs blocking (mp-tcp-bufsync) is CPU usage during waits:
//! nonblocking-poll BURNS CPU, blocking yields it. The honest scope
//! note in TASK-0044.02 (cycle 171) commits to picking
//! `std::thread::yield_now` as the yield primitive when substantive
//! multi-worker codegen lands — not a sleep (latency hit) and not pure
//! busy-spin (full-core burn). Hazard: a peer that never sends becomes
//! a spin-deadlock with no error; the codegen cycle must bound the
//! retry / add a deadline. See memory
//! project-mp-tcp-event-vs-bufsync-safety-profile for the analog
//! mask-failure-modes warning on the async sibling. Single-worker
//! does not exercise this primitive — the poll-loop only fires across
//! workers — so this hazard is exclusive to TASK-0044.02.02.
//!
//! # Honest limitations
//!
//! - Multi-worker SYNC schedules (02/split, 03/distributed,
//!   06/distributed, 06/distributed2, 07/distributed, 07/distributed-2d,
//!   08/distributed, 13/batch_parallel) WOULD compile under the
//!   mp-tcp-poll capability surface but currently hit the multi-worker
//!   ContractGap — promote via TASK-0044.02.02.
//! - Capability-mismatch schedules (05/distributed, 05/distributed-2d,
//!   09/pipelined, 11/pipelined, 13/pipeline_parallel) — async +
//!   buffer + event — are rejected upstream at the capability-compat
//!   check, NOT at codegen, and stay [[skip]] forever per PRD §7.1
//!   row mp-tcp-poll (sync capability surface is pinned).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

// Shared project-skeleton renderers — same single source of truth as
// mp-tcp-bufsync and mp-tcp-event (TASK-0257, backend_common::
// project_skeleton::multi_binary). Using these IS how the cross-
// backend differential invariant on naive schedules holds: mp-tcp-poll's
// emitted Cargo.toml + run.sh are byte-equal to mp-tcp-bufsync's for
// any single-worker schedule.
use backend_common::project_skeleton::multi_binary;

// Single-worker main.rs body — the only inter-backend arrow that
// genuinely is a semantic delegation (NOT inert string templating).
// The `_with_kernels_attr` variant injects the `#[path="../kernels.rs"]`
// header so the emitted file works under `src/bin/`. Same precedent as
// mp-tcp-bufsync (cycle 24) and mp-tcp-event (cycle 41).
use pthreads_sync::render_single_worker_main_with_kernels_attr;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Paths to the files [`emit`] writes. Same shape as
/// `mp_tcp_bufsync::EmitResult` and `mp_tcp_event::EmitResult` because
/// mp-tcp-poll also produces a multi-process Cargo project (per-worker
/// binaries + shared wire.rs + run.sh). The shape is intentionally
/// identical for driver-side dispatch uniformity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Per-worker `src/bin/<worker>.rs` paths (single-process builds
    /// have exactly one entry).
    pub worker_bins: Vec<PathBuf>,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `src/wire.rs` (the copied protocol v0).
    pub wire_rs: PathBuf,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

/// Emit a runnable multi-process Cargo project from the per-worker
/// EventList. Same signature/contract as `mp_tcp_bufsync::emit`.
///
/// Dispatch:
///
/// - `used_workers <= 1` → SINGLE-PROCESS. Delegates to the SHARED
///   `pthreads_sync::render_single_worker_main_with_kernels_attr` so
///   the emitted binary body is byte-identical to mp-tcp-bufsync's
///   single-process binary (and therefore byte-identical arithmetic
///   to pthreads-sync's single process). The Cargo.toml + run.sh
///   come from `backend_common::project_skeleton::multi_binary` for
///   the same reason; `wire.rs` is copied verbatim from
///   `mp_tcp_common::WIRE_RUNTIME_SRC` (the single-process binary
///   does not consume any wire surface, but the file is emitted for
///   shape uniformity with multi-process builds).
/// - `used_workers >= 2` → MULTI-PROCESS. Returns
///   [`EmitError::ContractGap`] pointing at the nonblocking-poll
///   multi-worker follow-up sub-task TASK-0044.02.02.
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    let src_dir = out_dir.join("src");
    let bin_dir = src_dir.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: bin_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let kernels_rs = src_dir.join("kernels.rs");
    let wire_rs = src_dir.join("wire.rs");
    let run_sh = out_dir.join("run.sh");

    // Shared modules every worker binary `#[path]`-includes. Even the
    // single-process path emits `wire.rs` for shape uniformity with
    // multi-process builds (the single-process binary just does not
    // import any wire surface). Same precedent as mp-tcp-bufsync.
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&wire_rs, mp_tcp_common::WIRE_RUNTIME_SRC)?;

    if used_workers.len() >= 2 {
        // ---- Multi-worker arm: nonblocking-poll codegen (NOT YET LANDED). ----
        return Err(EmitError::ContractGap(
            "mp-tcp-poll codegen: multi-worker arm (used_workers >= 2) is \
             not yet implemented — the nonblocking-read poll loop + \
             wire-framed Push/Wait + Plan-shaped per-worker codegen is \
             the headline follow-up of TASK-0044.02, filed as \
             TASK-0044.02.02 (see also AC#7 there: the deadlock-bound \
             contract — never-sending peer MUST surface as a loud error, \
             not a silent spin). The schedule's capability compat-check \
             has succeeded; only the multi-worker codegen body is \
             outstanding. Single-worker schedules (used_workers <= 1) ARE \
             supported and emit byte-identical artefacts to \
             mp-tcp-bufsync's single-process output."
                .to_string(),
        ));
    }

    // ---- Single-process arm (TASK-0044.02 cycle 192). ----
    //
    // Delegate to the SHARED renderers in pthreads-sync +
    // backend-common. The emitted binary body is byte-identical to
    // mp-tcp-bufsync's single-process body by construction (same
    // function, same inputs, same `KERNELS_MOD_ATTR_FOR_SRC_BIN`); the
    // Cargo.toml + run.sh ditto. Single-worker check_frame codegen
    // (Panic / Log / Count) is inherited from
    // `render_single_worker_main_with_kernels_attr` automatically —
    // no per-backend Log/Count emit needed for the single-worker case.
    let events = used_workers
        .first()
        .and_then(|w| per_worker.get(w))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let body = render_single_worker_main_with_kernels_attr(
        events,
        names,
        sidecar,
        KERNELS_MOD_ATTR_FOR_SRC_BIN,
    )?;
    let bin_path = bin_dir.join("nuc-generated.rs");
    write_file(&bin_path, &body)?;
    write_file(
        &cargo_toml,
        &multi_binary::render_cargo_toml(&[String::from("nuc-generated")], None),
    )?;
    write_file(&run_sh, &multi_binary::render_run_sh_single())?;
    mark_executable(&run_sh);

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        worker_bins: vec![bin_path],
        kernels_rs,
        wire_rs,
        run_sh,
    })
}

/// `#[path]` attribute block that redirects the shared single-worker
/// renderer's `mod kernels;` at the copied sibling `../kernels.rs`,
/// because mp-tcp-poll (like mp-tcp-bufsync / mp-tcp-event) emits the
/// binary under `src/bin/` rather than as a sibling of
/// `src/kernels.rs`. Passed to
/// `pthreads_sync::render_single_worker_main_with_kernels_attr` as a
/// typed parameter (TASK-0177).
///
/// Byte-identical to the same const in mp-tcp-bufsync (the cross-
/// backend differential invariant: same const → same emitted
/// header → same compilation behaviour).
const KERNELS_MOD_ATTR_FOR_SRC_BIN: &str = "#[path = \"../kernels.rs\"]\n#[allow(dead_code)]\n";

fn write_file(path: &Path, contents: &str) -> Result<(), EmitError> {
    fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}

fn mark_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}
