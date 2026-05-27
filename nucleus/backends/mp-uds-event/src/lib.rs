//! mp-uds-event backend. PRD §7.1 row 7, TASK-0044.03.
//!
//! Tier-1 backend: workers are OS PROCESSES, transport is Unix domain
//! sockets, notify is mio/epoll event readiness, supports async +
//! buffer. **supports_async=true, supports_buffer=true** — same
//! SCHEDULE-visible capability surface as mp-tcp-event. The cross-
//! backend differential gains a FOURTH async+buffered+event row
//! (alongside pthreads-async + mp-tcp-event) and the FIRST UDS row.
//!
//! # Implementation status (TASK-0044.03 cycle 194, 2026-05-27)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`) is IMPLEMENTED.
//!   Delegates to `pthreads_sync::render_single_worker_main_with_kernels_attr`
//!   plus `backend_common::project_skeleton::multi_binary::{render_cargo_toml,
//!   render_run_sh_single}` — the SAME shared renderers mp-tcp-event's
//!   and mp-tcp-poll's single-process arms consume. SOURCE-byte-identical
//!   to mp-tcp-event's single-process binary (pinned by
//!   tests/single_worker_emit.rs — same emitter, same arguments).
//!   Source diverges from pthreads-sync's single-process emit because
//!   pthreads-sync targets `src/main.rs` with bare `mod kernels;` while
//!   mp-uds-event targets `src/bin/nuc-generated.rs` with the
//!   `#[path="../kernels.rs"]` attribute injection. The cross-backend
//!   differential at the COMPILED-OUTPUT level (`output.bin` is
//!   bit-identical across all backends per the e2e gate) is the actual
//!   project-level invariant — pinned end-to-end by the e2e harness
//!   running each emitted Cargo project and diffing the output.
//! - **Multi-worker arm** (`used_workers.len() >= 2`) is NOT YET
//!   implemented. Returns [`EmitError::ContractGap`] forward-linking
//!   the multi-worker follow-up sub-task of TASK-0044.03
//!   (TASK-0044.03.01 — the Unix domain socket reactor + per-(DataId,
//!   SeqTag) ring buffer + epoll readiness codegen; cycle-171 brief
//!   flags as candidate for lifting the transport layer from
//!   mp-tcp-event).
//!
//! # Why split single-worker vs multi-worker into separate cycles
//!
//! Single-worker mp-uds-event has NO cross-worker `Push`/`Wait` events,
//! so the UDS-specific reactor (mio's `UnixListener` / `UnixStream`)
//! is unused; the single-process binary delegates byte-for-byte to
//! pthreads-sync's straight-line renderer. Splitting the single-worker
//! arm off keeps it a genuinely single-cycle unit (mechanical
//! delegation, no new transport substrate) and quarantines the
//! multi-cycle UDS-reactor headline work under TASK-0044.03.01. Same
//! precedent as cycles 191 (openmp-rs single-worker) and 192
//! (mp-tcp-poll single-worker).
//!
//! # wire.rs note (cross-transport honesty)
//!
//! Single-worker mp-uds-event emits `src/wire.rs` byte-identical to
//! `mp_tcp_common::WIRE_RUNTIME_SRC` for shape uniformity with
//! multi-process builds — the SAME pattern mp-tcp-event uses. The
//! emitted single-process binary does NOT `mod wire;`, so the file
//! sits as a sibling that cargo's reachability analysis never compiles
//! into the bin target. The wire.rs content is TCP-specific code that
//! will be REPLACED with a UDS-specific runtime when TASK-0044.03.01
//! lands the multi-worker arm (per cycle-171 brief: "lift transport
//! layer from mp-tcp-event" CANDIDATE). The cross-backend
//! byte-identical invariant on single-worker artefacts holds against
//! mp-tcp-event (both backends emit the same wire.rs verbatim from
//! the shared const today).
//!
//! # Generated artefact layout
//!
//! Identical to mp-tcp-event's: under the user-provided `out_dir`,
//! `Cargo.toml`, `src/bin/<worker>.rs` (per used worker; for
//! single-worker just `src/bin/nuc-generated.rs`), `src/wire.rs`
//! (copied wire-v0 protocol), `src/kernels.rs`, and `run.sh`. The
//! `runtime.rs` file (mio reactor) is only emitted on multi-worker
//! emit — `EmitResult::runtime_rs` is `None` for single-worker.
//!
//! # TASK-0337 promotion-trigger guard (inherited from cycle-171 AC#9)
//!
//! mp-uds-event INHERITS the (D)+(B2) compensating-pass tower from
//! mp-tcp-event (cycles 149/151/162/163/165: host-relay + defensive
//! guard + reorder + ACFG relay-inject + 13-arm). When the multi-worker
//! arm lands (TASK-0044.03.01), if a UDS-specific schedule shape
//! forces a 5th compensating pass, that IS the promotion trigger
//! named in TASK-0337's "Promotion triggers" section. Single-worker
//! does NOT exercise the tower; this guard is exclusive to
//! TASK-0044.03.01.
//!
//! # Honest limitations
//!
//! - Multi-worker async/buffered/event schedules (02/split,
//!   03/distributed, 05/distributed{,-2d}, 06/distributed{,2},
//!   07/distributed{,-2d}, 08/distributed, 09/pipelined, 11/pipelined,
//!   13/batch_parallel, 13/pipeline_parallel) WOULD compile under the
//!   mp-uds-event capability surface (which is a SUPERSET of
//!   mp-tcp-event's; same SCHEDULE-visible surface) but currently hit
//!   the multi-worker ContractGap — promote via TASK-0044.03.01.
//!   There are NO capability-mismatch [[skip]] cells for mp-uds-event
//!   (unlike openmp-rs / mp-tcp-poll) because the async + buffer +
//!   event capabilities are exactly what mp-uds-event supports.
//! - UDS path-length cap (~104 chars on macOS, ~108 on Linux). Not
//!   relevant at the single-worker cycle (no socket is bound); the
//!   multi-worker cycle's run.sh must check + reject paths exceeding
//!   the cap. Per cycle-171 AC#8 forward-carried to TASK-0044.03.01.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

// Shared project-skeleton renderers — same single source of truth as
// mp-tcp-bufsync + mp-tcp-event + mp-tcp-poll (TASK-0257, backend_common::
// project_skeleton::multi_binary). Using these IS how the cross-backend
// byte-identical invariant holds for single-worker artefacts.
use backend_common::project_skeleton::multi_binary;

// Single-worker main.rs body — the only inter-backend arrow that
// genuinely is a semantic delegation (NOT inert string templating).
// The `_with_kernels_attr` variant injects the `#[path="../kernels.rs"]`
// header so the emitted file works under `src/bin/` (TASK-0177
// typed-parameter lift; same pattern mp-tcp-bufsync + mp-tcp-poll
// cycles 191/192 use, which is byte-equivalent to mp-tcp-event's older
// `wrap_single_worker` post-hoc `replacen` per the cycle-171 honest-
// scope plan).
use pthreads_sync::render_single_worker_main_with_kernels_attr;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Paths to the files [`emit`] writes. Same shape as
/// `mp_tcp_event::EmitResult` — multi-binary with an optional
/// `runtime_rs` (only present on multi-worker emit, per the
/// mp-tcp-event precedent at TASK-0042.05 cycle 79).
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

/// Emit a runnable multi-process Cargo project from the per-worker
/// EventList. Same signature/contract as `mp_tcp_event::emit`.
///
/// Dispatch:
///
/// - `used_workers <= 1` → SINGLE-PROCESS. Delegates to the SHARED
///   `pthreads_sync::render_single_worker_main_with_kernels_attr` so
///   the emitted binary body is byte-identical to mp-tcp-event's
///   single-process binary (and therefore byte-identical arithmetic
///   to pthreads-sync's single process). The Cargo.toml + run.sh
///   come from `backend_common::project_skeleton::multi_binary` for
///   the same reason; `wire.rs` is copied verbatim from
///   `mp_tcp_common::WIRE_RUNTIME_SRC` (the single-process binary
///   does not consume any wire surface, but the file is emitted for
///   shape uniformity with multi-process builds; see module-doc
///   "wire.rs note" for the cross-transport honesty rationale).
///   `runtime_rs` is `None` (no mio reactor needed).
/// - `used_workers >= 2` → MULTI-PROCESS. Returns
///   [`EmitError::ContractGap`] pointing at the UDS-reactor multi-
///   worker follow-up sub-task TASK-0044.03.01.
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
    // `mod wire;`, so cargo's reachability analysis never compiles it
    // into the bin target). Same precedent as mp-tcp-event +
    // mp-tcp-poll. The TCP-specific wire content will be REPLACED with
    // a UDS-specific runtime when TASK-0044.03.01 lands the multi-worker
    // arm.
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&wire_rs, mp_tcp_common::WIRE_RUNTIME_SRC)?;

    if used_workers.len() >= 2 {
        // ---- Multi-worker arm: UDS-reactor codegen (NOT YET LANDED). ----
        return Err(EmitError::ContractGap(
            "mp-uds-event codegen: multi-worker arm (used_workers >= 2) \
             is not yet implemented — the Unix domain socket reactor + \
             per-(DataId, SeqTag) ring buffer + epoll readiness + \
             Plan-shaped per-worker codegen is the headline follow-up \
             of TASK-0044.03, filed as TASK-0044.03.01 (see also \
             cycle-171 brief: candidate for LIFTING transport layer \
             from mp-tcp-event if its reactor is parametric over \
             listener type via a trait). The schedule's capability \
             compat-check has succeeded; only the multi-worker codegen \
             body is outstanding. Single-worker schedules (used_workers \
             <= 1) ARE supported and emit byte-identical artefacts to \
             mp-tcp-event's single-process output."
                .to_string(),
        ));
    }

    // ---- Single-process arm (TASK-0044.03 cycle 194). ----
    //
    // Delegate to the SHARED renderers in pthreads-sync +
    // backend-common. The emitted binary body is byte-identical to
    // mp-tcp-event's single-process body by construction (same
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
        runtime_rs: None,
        run_sh,
    })
}

/// `#[path]` attribute block that redirects the shared single-worker
/// renderer's `mod kernels;` at the copied sibling `../kernels.rs`,
/// because mp-uds-event (like mp-tcp-bufsync / mp-tcp-event /
/// mp-tcp-poll) emits the binary under `src/bin/` rather than as a
/// sibling of `src/kernels.rs`. Passed to
/// `pthreads_sync::render_single_worker_main_with_kernels_attr` as a
/// typed parameter (TASK-0177).
///
/// Byte-identical to the same const in mp-tcp-bufsync + mp-tcp-poll
/// (and to the byte-output of mp-tcp-event's older `wrap_single_worker`
/// post-hoc `replacen` — the cycle-171 honest-scope plan to migrate
/// mp-tcp-event to the typed-parameter API is filed separately).
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
