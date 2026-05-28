//! mp-tcp-poll backend. PRD §7.1 row 5, TASK-0044.02 + TASK-0044.02.02.
//!
//! Tier-1 backend: workers are OS PROCESSES, transport is TCP loopback,
//! notify is NONBLOCKING POLL (yield wait loop), no buffer, sync.
//! **supports_async=false, supports_buffer=false** — same SCHEDULE-
//! visible capability surface as mp-tcp-bufsync. The cross-backend
//! differential gains a SECOND sync-TCP row; every schedule that
//! compiles against mp-tcp-bufsync MUST compile against mp-tcp-poll and
//! produce bit-identical output.
//!
//! # Implementation status (TASK-0044.02.02 cycle 195, 2026-05-27)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`) is IMPLEMENTED
//!   (cycle 192). Delegates to
//!   `pthreads_sync::render_single_worker_main_with_kernels_attr` plus
//!   `backend_common::project_skeleton::multi_binary::{render_cargo_toml,
//!   render_run_sh_single}` — the SAME shared renderers
//!   mp-tcp-bufsync's single-process arm consumes. Emitted artefact is
//!   BYTE-IDENTICAL to mp-tcp-bufsync's single-process output (and
//!   therefore arithmetic byte-identical to pthreads-sync's single
//!   process).
//! - **Multi-worker arm** (`used_workers.len() >= 2`) is IMPLEMENTED
//!   (cycle 195). Plan-shaped per-worker codegen consuming the
//!   nonblocking-poll wire primitives (`wire::read_msg_expect_poll`,
//!   `wire::write_msg_poll`, `wire::barrier_cross_poll`,
//!   `wire::apply_nonblocking`). The Plan/walkers/encode substrate now
//!   lives ONCE in `backend_common::tcp_plan`, parameterised over the
//!   `WirePrimitives` trait (lifted TASK-0044.02.03, cycle 235); this
//!   crate's `plan.rs` supplies only the `PollWire` impl plus a `Plan`
//!   type alias.
//!
//! # Generated artefact layout
//!
//! Identical to mp-tcp-bufsync's: under the user-provided `out_dir`,
//! `Cargo.toml` + `src/bin/<worker>.rs` (per used worker; for
//! single-worker just `src/bin/nuc-generated.rs`) + `src/wire.rs`
//! (copied wire-v0 protocol) + `src/kernels.rs` + `run.sh`.
//!
//! # The nonblocking-poll wait primitive
//!
//! PRD §7.1 row 5 fixes the wait primitive as nonblocking poll.
//! The mp-tcp-common `wire::read_msg_expect_poll` helper loops on
//! `WouldBlock` with `std::thread::yield_now` (not a sleep — latency
//! hit, not a busy-spin — full-core burn). A bounded deadline
//! (`NUC_POLL_DEADLINE_MS` env, default 30 s) ensures a never-sending
//! peer surfaces as a loud panic naming seq + elapsed, not a silent
//! spin (AC#7 of TASK-0044.02.02; memory
//! `project-mp-tcp-event-vs-bufsync-safety-profile` analog).
//!
//! Seq-tag mismatches still panic loud — the poll loop does NOT mask
//! contract violations as "wrong frame, keep waiting" (that would
//! mask the fail-loud guarantee `read_msg_expect` provides on
//! mp-tcp-bufsync). See `mp_tcp_common::wire::read_msg_expect_poll`
//! docstring + `read_msg_expect_poll_rejects_wrong_seq` unit test.
//!
//! # Honest limitations
//!
//! - Capability-mismatch schedules (05/distributed, 05/distributed-2d,
//!   09/pipelined, 11/pipelined, 13/pipeline_parallel) — async +
//!   buffer + event — are rejected upstream at the capability-compat
//!   check, NOT at codegen, and stay [[skip]] forever per PRD §7.1
//!   row mp-tcp-poll (sync capability surface is pinned).
//! - The poll/bufsync difference lives EXCLUSIVELY in the
//!   `WirePrimitives` impl (the `wire::*_poll` call-site swap and the
//!   `apply_nonblocking` line); all analysis (host election, xfer
//!   registry, slice-paste, accumulator classification, FIFO-shape
//!   hazards) is the shared `backend_common::tcp_plan` substrate
//!   (lifted TASK-0044.02.03, cycle 235 — no longer duplicated).
//! - Wait-before-push hazard rejection is unconditional on
//!   mp-tcp-poll (same as mp-tcp-bufsync). The
//!   `apply_safe_push_reorder` driver pass that lifts the constraint
//!   on mp-tcp-event is NOT wired for mp-tcp-poll because the
//!   per-pair FIFO single-stream topology has the same race shape as
//!   bufsync (nonblocking-read changes how the receiver waits, NOT
//!   the on-wire frame order). See memory
//!   `project-mp-tcp-event-vs-bufsync-safety-profile`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

use backend_common::project_skeleton::multi_binary;
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use pthreads_sync::render_single_worker_main_with_kernels_attr;

mod plan;

use plan::Plan;

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
/// - `used_workers <= 1` → SINGLE-PROCESS. Delegates to the SHARED
///   `pthreads_sync::render_single_worker_main_with_kernels_attr` so
///   the emitted binary body is byte-identical to mp-tcp-bufsync's
///   single-process body.
/// - `used_workers >= 2` → MULTI-PROCESS. Plan-shaped per-worker
///   codegen consuming the nonblocking-poll wire primitives. Same
///   topology as mp-tcp-bufsync (one TcpStream per (host, worker)
///   ordered pair; rendezvous-file port handshake; barrier-over-TCP;
///   host-mediated star + host-relay for w2w pushes); the only
///   schedule-visible difference is the wait-primitive swap
///   (`*_poll` vs blocking) plus the `apply_nonblocking` setup line.
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

    // Shared modules every worker binary `#[path]`-includes. `wire.rs`
    // is emitted PRISTINE: byte-for-byte copy of
    // `mp_tcp_common::WIRE_RUNTIME_SRC` (includes the cycle-195 poll
    // additions). Even the single-process arm emits `wire.rs` for
    // shape uniformity with multi-process builds.
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&wire_rs, mp_tcp_common::WIRE_RUNTIME_SRC)?;

    if used_workers.len() <= 1 {
        // ---- Single-process arm (cycle 192). ----
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
        return Ok(EmitResult {
            project_dir: out_dir.to_path_buf(),
            cargo_toml,
            worker_bins: vec![bin_path],
            kernels_rs,
            wire_rs,
            run_sh,
        });
    }

    // ---- Multi-process arm (cycle 195, TASK-0044.02.02). ----
    let plan = Plan::build(per_worker, names, sidecar)?;
    let mut bin_names: Vec<String> = Vec::new();
    let mut worker_bins: Vec<PathBuf> = Vec::new();
    for w in &plan.used_workers {
        let wname = plan.worker_name(*w);
        let body = plan.render_worker_program(*w)?;
        let bin_path = bin_dir.join(format!("{wname}.rs"));
        write_file(&bin_path, &body)?;
        bin_names.push(wname);
        worker_bins.push(bin_path);
    }

    write_file(
        &cargo_toml,
        &multi_binary::render_cargo_toml(&bin_names, None),
    )?;
    write_file(&run_sh, &plan.render_run_sh()?)?;
    mark_executable(&run_sh);

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        worker_bins,
        kernels_rs,
        wire_rs,
        run_sh,
    })
}

/// Per-backend SO_BUF commentary block interpolated by the shared
/// [`multi_binary::render_run_sh_multi`] before the
/// `export NUC_SO_BUF=...` line. mp-tcp-poll has the same capability
/// surface as mp-tcp-bufsync (sync, buffer=1) so the sizing
/// requirement is identical: one message in flight per channel.
pub(crate) const SO_BUF_COMMENT_POLL: &str =
    "# Socket buffer requirement from the schedule's per-channel\n\
     # buffer needs (largest single transfer payload, sync=1 msg).\n\
     # mp-tcp-poll: same sizing as mp-tcp-bufsync (shared capability surface).\n";

/// `#[path]` attribute block that redirects the shared single-worker
/// renderer's `mod kernels;` at the copied sibling `../kernels.rs`,
/// because mp-tcp-poll (like mp-tcp-bufsync / mp-tcp-event) emits the
/// binary under `src/bin/` rather than as a sibling of
/// `src/kernels.rs`. Passed to
/// `pthreads_sync::render_single_worker_main_with_kernels_attr` as a
/// typed parameter (TASK-0177).
///
/// Byte-identical to the same const in mp-tcp-bufsync (the cross-
/// backend differential invariant: same const → same emitted header
/// → same compilation behaviour).
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

// `io` is referenced via `EmitError` (re-exported from backend_common)
// for the `KernelsReadFailed`/`WriteFailed` variants; the alias keeps
// the use explicit for readers.
#[allow(unused_imports)]
use io as _io;
