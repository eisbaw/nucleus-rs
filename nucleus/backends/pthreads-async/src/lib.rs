//! pthreads-async backend. PRD §7.1, TASK-0042.01.
//!
//! Tier-1 CPU backend: `std::thread` + `std::sync::Condvar` +
//! bounded ring buffer per `(DataId, SeqTag)`. Shared-memory
//! transport, single binary (single-worker) or thread-per-worker
//! (multi-worker). **supports_async=true, supports_buffer=true** —
//! the third tier-1 backend, completing the async/buffered surface
//! that pthreads-sync and mp-tcp-bufsync cannot.
//!
//! # Implementation status (TASK-0042.01 cycle 17, 2026-05-22)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`) is IMPLEMENTED
//!   under TASK-0226. Delegates to `pthreads_sync::render_single_worker_main`
//!   plus the SHARED `pthreads_sync::render_cargo_toml` and
//!   `pthreads_sync::render_run_sh`, so the emitted artefact is
//!   byte-identical to pthreads-sync's single-worker output for any
//!   naive schedule. The cross-backend differential invariant
//!   ("same algorithm + same naive schedule -> bit-identical output
//!   across backends") holds by construction.
//! - **Multi-worker arm** (`used_workers.len() >= 2`) is NOT YET
//!   implemented (TASK-0228 — the headline work, per-(DataId,
//!   SeqTag) ring buffer + Condvar + thread/Plan structure). Returns
//!   [`EmitError::ContractGap`] with a precise forward-link.
//! - **check_frame codegen** (single-worker) is deferred to TASK-0227.
//!   `inject_check_frames` populates `Event::Loop.check_frame`; the
//!   single-worker delegation to `pthreads_sync::render_single_worker_main`
//!   inherits the pthreads-sync check_frame codegen by construction —
//!   any `check loop V` directive on a single-worker pthreads-async
//!   schedule emits the SAME instrumentation as pthreads-sync.
//!
//! # Why split single-worker vs multi-worker into separate cycles
//!
//! Single-worker pthreads-async has NO cross-worker `Push`/`Wait`
//! events (the pthreads-sync single-worker emitter `ContractGap`s on
//! either; same is true here, inherited via delegation). The ring
//! buffer + Condvar machinery only fires across two or more workers.
//! Splitting the single-worker arm off as TASK-0226 keeps it a
//! genuinely single-cycle unit (mechanical delegation) and quarantines
//! the multi-cycle ring-buffer headline work under TASK-0228.
//!
//! # Forward-carried context for TASK-0228 (multi-worker)
//!
//! 1. **Ring buffer contract** (post-TASK-0213): `Mutex<VecDeque<T>>`
//!    plus `Condvar` (not_empty) plus `Condvar` (not_full). Capacity
//!    = `transfer DATA : buffer=N` (the `N`). Ring STARTS EMPTY (D
//!    is the analysis-only sizing invariant, NOT a runtime pre-fill).
//! 2. **Per-fan-out-pair sizing** (TASK-0216): one ring per
//!    `(DataId, SeqTag, src_worker, dst_worker)` tuple — fan-out
//!    splits one data symbol into N rings.
//! 3. **Same-worker carveout** (TASK-0214): a transfer whose producer
//!    plus consumer share one worker emits NO ring; the EventList
//!    already omits the cross-worker Push/Wait per transfer_inject's
//!    src==dst skip.
//! 4. **Multi-worker check_frame** (TASK-0052.05): file-scope shared
//!    `static AtomicU64` per UNIQUE sanitized ident; Drop guard on
//!    host thread; panic=abort SIGABRT gotcha applies (worker thread
//!    panic SIGABRTs the whole process; tests must accept exit
//!    code = `None`).
//! 5. **EventList contract** (TASK-0124): `D` is NOT yet carried on
//!    `Event::Push`/`Wait` — derive sizing from `buffer=N` directly
//!    (Option (c) from the post-TASK-0213 contract carry).
//!
//! # Generated artefact layout
//!
//! Identical to pthreads-sync's: under the user-provided `out_dir`,
//! `Cargo.toml` + `src/main.rs` + `src/kernels.rs` + `run.sh`. The
//! `[[bin]]` name + the package name are backend-agnostic so the
//! generated project is movable.
//!
//! # Error handling
//!
//! Failures bubble up as [`EmitError`] variants (re-exported from
//! `pthreads-sync` — same shared-surface precedent as mp-tcp-bufsync;
//! see TASK-0230 for the cosmetic Display-prefix cleanup).
//!
//! # Honest limitations
//!
//! - Multi-worker codegen is `ContractGap` (TASK-0228). Until
//!   TASK-0228 lands, the *only* schedules this backend runs
//!   end-to-end are those whose worker count is 0 or 1 — which is
//!   already covered by pthreads-sync. Single-worker pthreads-async
//!   is therefore a *capability-check gateway* (capability surface
//!   satisfiable) more than a new runtime — the real value lands
//!   when TASK-0228 + TASK-0229 ship the multi-worker arm + the
//!   e2e cells.
//! - Async/buffer-N schedules whose `used_workers >= 2` (e.g.
//!   13-cnn-inference/pipeline_parallel) still hit the multi-worker
//!   ContractGap. These are the headline targets of TASK-0229.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// Re-export the SHARED error + reverse name tables from pthreads-sync.
// Same precedent as mp-tcp-bufsync (lib.rs:81): the driver builds
// `NameTables` ONCE and feeds every backend the identical input, which
// is required by the cross-backend differential gate (PRD §8.4 — same
// algorithm + same schedule -> bit-identical output across backends).
//
// Note (TASK-0230, closed cycle 32): the inner `EmitError::Display`
// prefix was historically "pthreads-sync: ..." regardless of which
// re-exporter surfaced the error — a cross-backend cosmetic lie. The
// per-backend prefix is now owned solely by the driver dispatch site
// (`driver/src/main.rs:406/426/448`, which wraps every Display with
// "<backend> codegen error:"). The Display arms in `pthreads-sync`
// emit the message without a backend literal, so user-visible text
// from any re-exporter (this crate, mp-tcp-bufsync) reads cleanly.
pub use backend_common::EmitError;
pub use compiler::NameTables;

// Shared codegen — the single source of truth for the project skeleton
// (Cargo.toml + run.sh) AND the single-worker main.rs body. Reusing
// these IS how the cross-backend differential invariant on naive
// schedules holds: pthreads-async's emitted artefact is byte-identical
// to pthreads-sync's for any `used_workers <= 1` input.
//
// `render_cargo_toml` / `render_run_sh` / `render_single_worker_main`
// stay in pthreads-sync (TASK-0244 cycle 37): they are the pthreads-
// sync-specific project skeleton + straight-line emitter, and the
// dependency arrow is genuinely a delegation (not a leak of shared
// codegen, which now lives in backend-common).
use pthreads_sync::{render_cargo_toml, render_run_sh, render_single_worker_main};

// AlgoIR-free: the only `compiler::*` surface this crate uses is the
// inert per-worker EventList carrier (`compiler::event::{Event, WorkerId}`)
// + the NameSidecar (`compiler::sidecar`). When TASK-0228 lands the
// multi-worker ring-buffer codegen, it will additionally import the
// inert expression grammar (`compiler::algo::{IrExpr, IrBinOp,
// ResolvedType, ScalarType}`) the EventList already carries — exactly
// the surface pthreads-sync + mp-tcp-bufsync consume.
use compiler::event::{Event, WorkerId};
use compiler::sidecar::NameSidecar;

// Multi-worker runtime substrate emitters (TASK-0228 Wave A, cycle 18).
// Pure-function emit helpers; NOT YET integrated into `emit()` —
// integration is Wave B's job (will land alongside the per-worker
// thread::spawn machinery). Surfaced now via `pub use` so a future
// implementer can call them directly without re-importing from the
// inner module path.
pub mod ring_buffer;
pub use ring_buffer::{emit_ring_instance_decl, emit_ring_struct_decl};

// Multi-worker codegen (TASK-0228 Waves B-1 + B-2, cycles 20-26).
// Wave B-1 landed the Plan data structure; Wave B-2 lands the
// `render_main_rs_multi` entry point + the per-worker thread::spawn
// emission. The Plan stays `pub(crate)`; the entry point is the
// only multi-worker symbol consumed below.
mod multi_worker;
use multi_worker::render_main_rs_multi;

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

/// Paths to the files [`emit`] wrote. Same shape as
/// `pthreads_sync::EmitResult` because pthreads-async also produces a
/// single Cargo project + single binary (shared-memory transport) —
/// only the per-loop codegen body differs (ring buffer + Condvar
/// instead of sequential barrier, once TASK-0228 lands). Mirrors the
/// field set so the driver can pattern-match the dispatch arm against
/// either single-binary tier-1 backend identically.
///
/// Defined locally (not a re-export of `pthreads_sync::EmitResult`)
/// because the *backend identity* is part of the result — a caller
/// inspecting `EmitResult` should not be lied to about which crate
/// produced it. The shape is intentionally identical for ease of
/// driver-side dispatch.
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
/// Wire contract (AC#1, TASK-0124): consumes the per-worker [`Event`]
/// lists + the [`NameTables`] (reverse `name_*`) + the [`NameSidecar`].
/// **No `&ACFG` / `&LinkedIR` access**, exactly like pthreads-sync and
/// mp-tcp-bufsync.
///
/// Dispatch:
///
/// - `used_workers <= 1` → SINGLE-WORKER (TASK-0226). Delegates to the
///   SHARED `pthreads_sync::render_single_worker_main` so the emitted
///   `main.rs` is byte-identical to pthreads-sync's. The Cargo.toml
///   and run.sh come from `pthreads_sync::render_cargo_toml` +
///   `render_run_sh` for the same reason.
/// - `used_workers >= 2` → MULTI-WORKER. Returns
///   [`EmitError::ContractGap`] pointing at TASK-0228 (the
///   ring-buffer + Condvar + thread/Plan headline work).
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
    // pthreads-sync.
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    if used_workers.len() >= 2 {
        // ---- Multi-worker arm (TASK-0228 Wave B-2). ----
        //
        // Emit the file-scope Ring<T> substrate + per-pair Arc<Ring<T>>
        // instances sized from `transfer_buffer_for_seq` (TASK-0233),
        // plus per-worker `thread::spawn` bodies whose Push/Wait
        // dispatch into `ring_<id>.push(v)` / `ring_<id>.wait()`. The
        // structural shape (barriers, Fire, Loop, Sync, check_frame
        // instrumentation, Wait gather) mirrors pthreads-sync's
        // multi-worker emit byte-for-byte modulo the slot→ring
        // substitution. TASK-0239 covers the de-dup follow-up.
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

        let main_rs_src = render_main_rs_multi(per_worker, names, sidecar)?;

        write_file(&cargo_toml, &render_cargo_toml())?;
        write_file(&kernels_rs, &kernels_src)?;
        write_file(&main_rs, &main_rs_src)?;
        write_file(&run_sh, &render_run_sh())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&run_sh) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&run_sh, perms);
            }
        }

        return Ok(EmitResult {
            project_dir: out_dir.to_path_buf(),
            cargo_toml,
            main_rs,
            kernels_rs,
            run_sh,
        });
    }

    // ---- Single-worker arm (TASK-0226). ----
    //
    // Delegate to the SHARED renderers in pthreads-sync. The emitted
    // main.rs is byte-identical to pthreads-sync's by construction
    // (same function, same inputs); Cargo.toml + run.sh ditto. The
    // single-worker check_frame codegen (Panic / Log / Count) is
    // inherited from pthreads-sync's render_single_worker_main →
    // render_main_rs path automatically — no per-backend Log/Count
    // emit is needed for the single-worker case (TASK-0227 carries
    // multi-worker check_frame, which lands with TASK-0228).
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
    // (mirrors pthreads-sync precedent at lib.rs:300-309).
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
