//! pthreads-sync backend. PRD §7.1, TASK-0020, TASK-0124.
//!
//! Tier-1 CPU backend that consumes the **EventList contract**
//! (PRD §7.4 / §8.3) — the per-worker [`Event`] projection of the
//! ACFG plus the [`NameSidecar`] (TASK-0160/0169) and the ACFG
//! name tables — and emits a standalone Cargo project containing
//! runnable Rust. Single-worker ("naive schedule") uses the
//! straight-line emitter; multi-worker uses `std::thread::spawn` +
//! `std::sync::Barrier` + `Slot<T>` channels.
//!
//! ## Why the backend now consumes the EventList, not AlgoIR (TASK-0124)
//!
//! The original M1 backend (TASK-0020) walked `LinkedIR::algo`
//! statements directly because the ACFG stripped index expressions
//! and the per-worker EventList did not yet carry loop structure or
//! value bindings. Three contract extensions closed that gap:
//!
//! - **TASK-0156** put the per-firing value binding (`FireBinding`)
//!   on [`Event::Fire`] — a backend reconstructs the exact kernel
//!   call (callee, indexed args, output slice) from the event alone.
//! - **TASK-0159** made the projection structure-preserving:
//!   [`Event::Loop`] mirrors `ACFGNode::Repeat` (iter var + range +
//!   nested body) instead of unrolling it, so a rolled `for` is
//!   re-emittable.
//! - **TASK-0160/0169** added the [`NameSidecar`]: per-`DataId`
//!   `ResolvedType` (pre-init sizing + slot typing + scalar casts),
//!   const values, the *unevaluated* symbolic loop bounds (so
//!   `for y : 1 .. H-1` re-renders as `(1_i64)..((16_i64 - 1_i64))`
//!   verbatim, not the folded `1..15`), and per-`KernelId`
//!   signatures (scalar-arg cast decisions without `algo.kernels`).
//!
//! With `(EventList, name tables, NameSidecar)` the codegen path is
//! **fully AlgoIR-/LinkedIR-free** (AC#2): this module imports
//! neither `nucleus_compiler::algo` nor `nucleus_compiler::link` for code emission.
//! `IrExpr` is used only as the inert index/scalar-expression grammar
//! the EventList already carries (it is the single source of truth
//! for "what an index is"; no pass evaluates it here).
//!
//! ## Generated artefact layout
//!
//! Under the user-provided `out_dir`:
//!
//! ```text
//! out_dir/
//!   Cargo.toml         -- standalone project, depends on nothing exotic
//!   src/
//!     main.rs          -- runs the algorithm
//!     kernels.rs       -- copy of the user's kernels.rs
//!   run.sh             -- builds + runs the binary with input.bin -> output.bin
//! ```
//!
//! `kernels.rs` is *copied* (not `include!`-ed): the generated
//! project is fully self-contained and movable. Trade-off: two files
//! reflect one source-of-truth; the expected workflow is "run
//! codegen, then build" — editing kernels.rs is followed by
//! re-running codegen.
//!
//! ## Error handling
//!
//! Failures bubble up as [`EmitError`] variants with the offending
//! path / reason attached. No silent fallbacks. The
//! [`EmitError::ContractGap`] variant is the fail-loud seam for "the
//! EventList/sidecar did not carry something the backend needs"
//! (e.g. a `DataId` with no sidecar type) — it must never be papered
//! over with a default.
//!
//! ## Honest limitations
//!
//! - **Single-worker straight-line + multi-worker thread/barrier.**
//!   Distributed placements (`place k on {w0,w1,w2}`) are still
//!   rejected (TASK-0117). Async / `buffer>1` transfers rejected
//!   (sync-only backend).
//! - **Aggregate I/O kernels.** `() -> Vec<T>` / `(Vec<T>) -> ()`
//!   recognised via the sidecar element type; whole-array
//!   binding/move calls emitted accordingly.
//! - **No error recovery in generated code.** A panic in any kernel
//!   aborts the whole binary (`panic = "abort"`).
//! - **No identity-copy support** (`d <-- e` with a bare DataRef
//!   RHS). The contract carries this as a `Fire` with a kernel; a
//!   non-`Call` dataflow shape never reaches a `Fire`, so this hole
//!   is inherited from the front passes, not introduced here.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// AlgoIR-free: this crate is now AlgoIR-FREE — every type used here
// comes through `backend_common::render` (which re-exports from
// `nucleus_compiler::algo` where needed for its OWN typed signatures).
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

// Shared codegen primitives (TASK-0244 cycle 37). The expression /
// index / kernel-call / loop-bound / type renderers, the check_frame
// emit templates, and the per-worker multi-worker event walker all
// live in backend-common — every backend (this one, pthreads-async,
// mp-tcp-bufsync) consumes the SAME implementation, no drift.
use backend_common::project_skeleton::single_binary::{render_cargo_toml, render_run_sh};
// Re-export the codegen-time error type so downstream callers (the
// driver, tests, other backends that delegate to this crate's
// single-worker emitter) continue to spell it `pthreads_sync::
// EmitError`. The canonical definition lives in backend-common since
// every backend re-exports it identically.
pub use backend_common::render::EmitError;

// The SHARED single-worker straight-line `main.rs` emitter
// (`render_single_worker_main` + its variants, plus the `events` /
// `break_loop` Event-rendering tree it calls) was relocated to
// `backend_common::single_worker_main` in TASK-0455.11 — it was the
// LAST inter-backend arrow (it lived here only because pthreads-sync
// was the first backend). Re-export ONLY the one renderer this
// crate's own tests still spell as `pthreads_sync::...`
// (`break_cond_codegen.rs`, `reuse_marker.rs`); the `_with_kernels_
// attr` / `_with_signature` variants had zero remaining consumers
// under the old path after every backend was re-pointed to
// backend-common, so per the remove-dead-re-exports rule they are
// NOT re-exported (wave-3 review P2-1).
pub use backend_common::single_worker_main::render_single_worker_main;

mod multi_worker;

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

// NameTables moved to `nucleus-compiler` as of TASK-0238 (cycle 25;
// crate previously named `compiler`, renamed in TASK-0084 cycle 76).
// Re-exported here so historic `pthreads_sync::NameTables` paths
// continue to work (mp-tcp-bufsync's `pub use pthreads_sync::NameTables`
// + pthreads-async's same re-export are unchanged; both transitively
// re-export the now-`nucleus_compiler::NameTables` definition).
// Cycle-24 review-gate B.1 found that the struct holds zero
// pthreads-sync-specific content, and its prior home prevented the
// cross-backend test-helper crate `test-common` from depending on
// pthreads-sync (would cycle). Moving to nucleus-compiler dissolves
// both constraints.
pub use nucleus_compiler::NameTables;

/// Paths to the files [`emit`] wrote, returned for callers that want
/// to inspect or invoke them.
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

// EmitError moved to backend-common (TASK-0244). The canonical type
// is `backend_common::render::EmitError`; this crate re-exports it at
// the top of this file (`pub use backend_common::render::EmitError`)
// so historic `pthreads_sync::EmitError` paths keep working.

/// Emit a runnable Cargo project from the per-worker EventList.
///
/// AC#1 signature (TASK-0124): the backend consumes the per-worker
/// [`Event`] lists, the [`NameTables`] (reverse `name_*`), and the
/// [`NameSidecar`] — NOT `&ACFG` / `&LinkedIR`. `kernels_rs_path` is
/// copied verbatim into the generated project.
///
/// Single- vs multi-worker is chosen by counting workers whose
/// EventList is non-empty: 0/1 → straight-line emitter, ≥2 →
/// thread/barrier emitter. (`acfg_to_events` seeds every declared
/// worker with an empty list, so an unused declared worker does not
/// trip the multi-worker path — exactly the old `collect_used_workers`
/// semantics, now read off the EventList.)
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // ---- Pick a code path: single-worker vs multi-worker. ----
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    // ---- Read user kernels.rs ----
    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    // ---- Create the output skeleton. ----
    let src_dir = out_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let kernels_rs = src_dir.join("kernels.rs");
    let run_sh = out_dir.join("run.sh");

    // ---- Render Cargo.toml ----
    // `extra_dependencies = None` keeps the emitted Cargo.toml
    // byte-identical to its pre-cycle-196 shape (TASK-0044.01.01
    // added the parameter for openmp-rs's multi-worker `rayon` dep;
    // pthreads-sync has no extra dep).
    write_file(&cargo_toml, &render_cargo_toml(None))?;

    // ---- Copy kernels.rs verbatim ----
    write_file(&kernels_rs, &kernels_src)?;

    // ---- Render main.rs ----
    let main_rs_src = if used_workers.len() <= 1 {
        let events = used_workers
            .first()
            .and_then(|w| per_worker.get(w))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        // The SHARED single-worker emitter now lives in backend-common
        // (TASK-0455.11); `render_single_worker_main` ==
        // `render_main_rs(events, names, sidecar, "", "fn main()")` so
        // the bare default-skeleton form stays byte-identical.
        render_single_worker_main(events, names, sidecar)?
    } else {
        multi_worker::render_main_rs_multi(per_worker, names, sidecar)?
    };
    write_file(&main_rs, &main_rs_src)?;

    // ---- Render run.sh ----
    write_file(&run_sh, &render_run_sh())?;
    // Best-effort: mark run.sh executable. Failure here is non-fatal.
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

// --------------------------------------------------------------------
// Project-skeleton renderers (Cargo.toml + run.sh)
// --------------------------------------------------------------------
//
// MOVED in TASK-0246 (cycle 38) to
// `backend_common::project_skeleton::single_binary::{render_cargo_toml,
// render_run_sh}`. The templates are byte-stable inert strings with
// zero pthreads-sync-specific content; lifting them out closes the
// last non-semantic dependency from pthreads-async on pthreads-sync.
// The only inter-backend arrow that survives is
// `render_single_worker_main` (a real semantic delegation: pthreads-
// async's single-worker arm IS pthreads-sync's straight-line main.rs,
// byte-identical by construction; mp-tcp-bufsync's single-process arm
// likewise wraps it). The renderers are imported at the top of this
// file from backend-common and consumed verbatim by `emit()`.

// --------------------------------------------------------------------
// EventList -> main.rs codegen (single-worker / straight-line)
// --------------------------------------------------------------------
//
// MOVED in TASK-0455.11 to `backend_common::single_worker_main`: the
// shared single-worker straight-line `main.rs` emitter
// (`render_single_worker_main` + the `_with_kernels_attr` /
// `_with_signature` variants), its private `render_main_rs` core, the
// pre-init helpers (`collect_pre_init_data`, `walk_fire_outputs`,
// `render_array_init`), and the `events` / `break_loop` Event-rendering
// tree it calls. It lived here only because pthreads-sync was the first
// backend; relocating it cut the LAST inter-backend arrow (every other
// backend imported it via `pthreads_sync::*`). One renderer is
// re-exported at the top of this file from
// `backend_common::single_worker_main` for this crate's own tests; the
// emitted bytes are byte-identical (emit-oracle A/B verified).

// --------------------------------------------------------------------
// File-write helper
// --------------------------------------------------------------------

fn write_file(path: &Path, contents: &str) -> Result<(), EmitError> {
    fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
