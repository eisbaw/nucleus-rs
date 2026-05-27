//! openmp-rs backend. PRD §7.1 row 1, TASK-0044.01 (single-worker
//! cycle 191) + TASK-0044.01.01 (multi-worker cycle 196).
//!
//! Tier-1 CPU backend: rayon threads + shared-memory transport +
//! barrier-style notify. **supports_async=false, supports_buffer=false**
//! — same SCHEDULE-visible capability surface as pthreads-sync. The
//! cross-backend differential gains a THIRD sync-shared-memory row;
//! every schedule that compiles against pthreads-sync MUST compile
//! against openmp-rs and produce bit-identical OUTPUT (output.bin vs
//! reference.bin) — the generated source is NOT byte-identical to
//! pthreads-sync's in the multi-worker arm (substrate swap: rayon::scope
//! vs std::thread::spawn + Cargo.toml `rayon = "1"` dep).
//!
//! # Implementation status (TASK-0044.01.01 cycle 196)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`) — delegates to
//!   `pthreads_sync::render_single_worker_main` plus the SHARED
//!   `backend_common::project_skeleton::single_binary::
//!   {render_cargo_toml, render_run_sh}` with
//!   `extra_dependencies = None`, so the emitted artefact is
//!   BYTE-IDENTICAL to pthreads-sync's / pthreads-async's single-worker
//!   output for any naive schedule. The cross-backend single-worker
//!   differential invariant holds by construction.
//! - **Multi-worker arm** (`used_workers.len() >= 2`) — delegates to
//!   `multi_worker::render_main_rs_multi`, a twin of pthreads-sync's
//!   multi-worker emitter with the spawn-site substrate swapped from
//!   `std::thread::spawn` + `handle.join()` to `rayon::scope` +
//!   `s.spawn(move |_| ...)`. The emitted Cargo.toml gains
//!   `rayon = "1"` in its `[dependencies]` block via the cycle-196
//!   addition `extra_dependencies = Some(_)`. Slot<T> + Arc<Barrier>
//!   primitives carry over verbatim (std::sync — work identically
//!   inside rayon::scope). Bit-identical OUTPUT on the 8 multi-worker
//!   SYNC schedules (02/split, 03/distributed, 06/distributed +
//!   distributed2, 07/distributed + distributed-2d, 08/distributed,
//!   13/batch_parallel) is the falsifiability gate, verified by the
//!   e2e cross-backend differential.
//!
//! # Why the multi-worker arm is NOT byte-identical to pthreads-sync
//!
//! The generated `src/main.rs` differs in four classes (the legal
//! differences canonicalised by the byte-equivalence test):
//! 1. `rayon::scope(|s| { ... });` wraps the entire spawn loop.
//! 2. `s.spawn(move |_| { ... })` replaces `thread::spawn(move || ...)`.
//! 3. No `use std::thread;`, no `let h = thread::spawn(...)` handles,
//!    no explicit `handle.join()` loop (rayon::scope's implicit join).
//! 4. The host body lives INSIDE the rayon::scope closure (so it
//!    sees the bare `slot_*` / `bar_*` bindings declared above the
//!    scope), one indent level deeper than pthreads-sync's host body.
//!
//! The runtime OUTPUT (output.bin) is bit-identical because the
//! schedule's sync contract (per-pair Slot rendezvous + per-tag
//! Barrier participation) is independent of the spawn primitive.
//!
//! # Driver-gate audit (TASK-0044.01.01 cycle 196)
//!
//! openmp-rs joins NONE of {apply_host_mediation_inject,
//! apply_host_data_relay_inject, apply_safe_push_reorder}:
//!
//! - `apply_host_mediation_inject` is for TCP-star backends
//!   (mp-tcp-bufsync / mp-tcp-event / mp-tcp-poll). openmp-rs uses
//!   shared-memory Arc<Barrier>, which handles host-excluding
//!   barriers natively (same path pthreads-sync / pthreads-async take).
//! - `apply_host_data_relay_inject` is mp-tcp-event-specific (routes
//!   w↔w Push through host as 4 hops). openmp-rs has direct
//!   shared-memory Slot channels — no host mediation needed.
//! - `apply_safe_push_reorder` is mp-tcp-event-specific (breaks the
//!   wait-before-push deadlock on the synchronous host-relay).
//!   openmp-rs's direct Slot channels have no such deadlock surface.
//!
//! Verified vs the cycle-195b silent-sibling-defect recurrence:
//! grepped `nucleus/nucleus-compiler/src/passes/host_mediation_inject.rs`
//! for backend-name tuples — the docstring at lines 5, 15, 66-71
//! already enumerates the {pthreads-sync, pthreads-async, openmp-rs}
//! SHARED-MEMORY exclusion class (cycle-191 single-worker landing
//! added openmp-rs). No widening or new docstring sites required.
//!
//! # Generated artefact layout
//!
//! Under the user-provided `out_dir`, identical SHAPE to pthreads-sync:
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
//! - **Capability-mismatch schedules** (05/distributed,
//!   05/distributed-2d, 09/pipelined, 11/pipelined,
//!   13/pipeline_parallel) — async + buffer + event — are rejected
//!   upstream at the capability-compat check, NOT at codegen, and
//!   stay [[skip]] forever per PRD §7.1 row openmp-rs (sync + barrier
//!   capability surface is pinned).
//! - **The `rayon = "1"` dep tracks rayon 1.x** (broadly compatible
//!   API surface). A future rayon 2.0 release would require bumping
//!   this string + verifying the rayon::scope API hasn't shifted.
//! - **Per-spawn closure overhead** (rayon's work-stealing scheduler
//!   has per-task bookkeeping). For schedules with very few workers
//!   and long-running per-worker bodies the overhead is negligible;
//!   for schedules with N>>cores and short bodies, rayon::scope is
//!   not necessarily faster than std::thread::spawn (rayon's
//!   sweet-spot is fine-grained parallelism). This backend chooses
//!   rayon for the *substrate semantics* (PRD §7.1 row 1), not as a
//!   performance claim.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

// Shared project-skeleton renderers — same single source of truth as
// pthreads-sync and pthreads-async (TASK-0246 + TASK-0044.01.01
// cycle-196 `extra_dependencies` parameter for the multi-worker
// `rayon = "1"` dep).
use backend_common::project_skeleton::single_binary::{render_cargo_toml, render_run_sh};

// Single-worker main.rs body — the only inter-backend arrow that
// genuinely is a semantic delegation (NOT inert string templating).
use pthreads_sync::render_single_worker_main;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

mod multi_worker;

/// The `[dependencies]` block injected into the emitted Cargo.toml
/// when openmp-rs's multi-worker arm fires. `rayon = "1"` accepts any
/// 1.x release — broadly compatible API surface (rayon::scope +
/// Scope::spawn have been stable since 1.0). The single-worker arm
/// passes `extra_dependencies = None`, so the single-worker emit
/// remains byte-identical to pthreads-sync's.
///
/// Kept as a module constant rather than inlined so the cycle-196
/// byte-equivalence test (`tests/multi_worker_emit.rs`) can grep for
/// the exact string the emit produces.
const RAYON_DEPENDENCY_BLOCK: &str = "rayon = \"1\"\n";

/// Paths to the files [`emit`] writes. Same shape as
/// `pthreads_sync::EmitResult` and `pthreads_async::EmitResult`.
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
///   single_binary::{render_cargo_toml, render_run_sh}` with
///   `extra_dependencies = None` for the same byte-identity reason.
/// - `used_workers >= 2` → MULTI-WORKER. Delegates to
///   `multi_worker::render_main_rs_multi` (rayon::scope spawn site +
///   verbatim Slot<T> / Arc<Barrier> rendezvous). The emitted
///   Cargo.toml gains `rayon = "1"` via `extra_dependencies = Some(_)`.
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // `acfg_to_events` seeds every declared worker with an empty list
    // so an unused-but-declared worker does not falsely trip the
    // multi-worker path. Same `collect_used_workers` semantics as
    // pthreads-sync and pthreads-async.
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
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let kernels_rs = src_dir.join("kernels.rs");
    let run_sh = out_dir.join("run.sh");

    // Pick single-vs-multi-worker code paths AND the corresponding
    // Cargo.toml extra-deps. The two decisions MUST move in lockstep:
    // single-worker uses pthreads-sync's straight-line emit + the
    // dep-less Cargo.toml (byte-stable vs pthreads-sync); multi-worker
    // uses the rayon::scope emit + the `rayon = "1"` Cargo.toml.
    // Computing them in one branch keeps a future contributor from
    // updating one without the other (the cycle-195b silent-sibling
    // recurrence pattern at a smaller scale).
    let (main_rs_src, cargo_extra_deps) = if used_workers.len() <= 1 {
        // ---- Single-worker arm. ----
        //
        // Delegate to the SHARED renderer in pthreads-sync. The
        // emitted main.rs is byte-identical to pthreads-sync's by
        // construction (same function, same inputs).
        let events = used_workers
            .first()
            .and_then(|w| per_worker.get(w))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let body = render_single_worker_main(events, names, sidecar)?;
        (body, None)
    } else {
        // ---- Multi-worker arm (TASK-0044.01.01 cycle 196). ----
        //
        // Delegate to the twin emitter in `multi_worker`. The
        // emitted main.rs uses `rayon::scope` for the spawn site;
        // the rest of the substrate (Slot<T>, Arc<Barrier>) is the
        // pthreads-sync shape verbatim because both paths consume
        // the SAME `backend_common::multi_worker_walker`. The
        // emitted Cargo.toml gains `rayon = "1"` so the generated
        // project builds standalone.
        let body = multi_worker::render_main_rs_multi(per_worker, names, sidecar)?;
        (body, Some(RAYON_DEPENDENCY_BLOCK))
    };

    write_file(&cargo_toml, &render_cargo_toml(cargo_extra_deps))?;
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
