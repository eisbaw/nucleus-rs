//! `mpi-nonblocking` backend — the SECOND tier-2 backend and the M8
//! milestone (PRD §7.2 / §11 M8, TASK-0046).
//!
//! Workers map to MPI ranks; transport is MPI. Unlike the blocking
//! mpi-blocking backend, `Push` lowers to a NON-BLOCKING BUFFERED send
//! (`MPI_Ibsend`) and `Wait` to a non-blocking receive (`MPI_Imrecv` /
//! `MPI_Irecv`) with an explicit `MPI_Wait`. Codegen is **SPMD**: ONE
//! Rust binary that dispatches on `MPI_Comm_rank` (PRD §7.2), launched via
//! `mpiexec -n N`. Output is hosted Rust + the rsmpi (`mpi` crate)
//! binding.
//!
//! # Why a non-blocking backend (the deadlock motivation)
//!
//! mpi-blocking lowers `Push` to standard-mode `MPI_Send`, which may block
//! for a message above the MPI eager limit until its matching `Recv` is
//! posted. The async schedules target the fully-async pthreads `Slot`
//! model, where a worker can post all its sends before any receive. Under
//! blocking send that ordering can deadlock (e.g. 05-stencil/distributed's
//! host broadcasts `img_in` to four workers that are sitting at a barrier
//! before they post their receives; 05-stencil/distributed-2d's
//! worker↔worker halo strips are waited BEFORE they are pushed). This
//! backend's buffered send (`MPI_Ibsend`) copies the payload into the
//! process-attached send buffer and **completes locally** — it does not
//! block on the matching receive — so those orderings are deadlock-immune
//! regardless of message size. That is why mpi-blocking
//! capability-REJECTS these schedules and this backend ACCEPTS them.
//!
//! # The `MPI_Request` / send-buffer lifetime trap (AC#5)
//!
//! `MPI_Isend(&buf)` returns a request immediately but the send buffer
//! must stay alive + unmutated until a matching `Wait`/`Test` completes;
//! a naive `chan.push(data.clone())` that drops the temporary clone at
//! end-of-statement is a use-after-free the network may read AFTER the
//! drop — silent, timing-dependent corruption an eager-size `-n N` run can
//! pass by luck. This backend dissolves the trap THREE ways:
//!   1. **Buffered mode.** `MPI_Ibsend` copies the payload into the
//!      attached buffer before the request even completes, so the user
//!      buffer is free to drop the instant `Wait` returns.
//!   2. **Lexical scope.** Each send/recv request is created inside
//!      `mpi::request::scope(|s| { ...; req.wait(); })`, whose closure runs
//!      the `Wait` before the owned buffer goes out of scope. The borrow
//!      checker enforces buffer-outlives-request via the scope lifetime.
//!   3. **rsmpi backstop.** `mpi::request::Request` PANICS on drop if left
//!      uncompleted, so a future refactor that forgets a `Wait` fails LOUD
//!      at runtime rather than corrupting silently.
//!
//! See [`multi_worker`] for the emitted prelude and the buffered-send
//! buffer attach (`MPI_Buffer_attach` via `Universe::set_buffer_size`).
//!
//! # Tier-2 ship bar = COMPILE (PRD §7.4), runtime best-effort
//!
//! The generated project depends on the `mpi` crate and builds + runs
//! ONLY under `nix develop .#mpi` (OpenMPI + libclang/bindgen). It is
//! EXCLUDED from the tier-1 bit-identical e2e RUNTIME matrix (that matrix
//! runs in the DEFAULT shell, which has no MPI). Acceptance is the
//! dedicated `just check-mpi-nonblocking` recipe (compile under `.#mpi` +
//! a localhost `mpiexec -n N` byte-exact run + a forced-rendezvous run to
//! defeat timing-luck) — the same compile-only-with-best-effort-runtime
//! precedent as mpi-blocking / `embedded-pattern` / `renode`.
//!
//! # Status: single-worker + multi-worker SPMD arms
//!
//! [`emit`] handles the 0/1-used-worker case (single-worker arm — no
//! cross-worker transfers, so it reuses the shared single-worker renderer
//! verbatim, identical to mpi-blocking's single-worker arm) AND the
//! ≥2-used-worker case (multi-worker SPMD arm) whose `Push`/`Wait` lower
//! to buffered `MPI_Ibsend` / `MPI_Imrecv` (tag = rendezvous id) and whose
//! `Sync` lowers to a whole-world `MPI_Barrier`. See [`multi_worker`].
//!
//! # Honest limitations (AC#6)
//!
//! - **No real-cluster CI.** Acceptance is localhost `mpiexec -n N` only
//!   (PRD §10.2). Multi-node correctness is unproven.
//! - **Point-to-point + barrier only** — collective recognition
//!   (`MPI_Allreduce`/`MPI_Scatter`) is deliberately deferred (PRD §7.2).
//! - **No derived-type packing.** One element-typed transfer per `Push`;
//!   `MPI_Type_contiguous` is not used (AC#6). Correct, just suboptimal.
//! - **Heuristic buffered-send buffer size.** The attached `MPI_Bsend`
//!   buffer is sized from the schedule's data footprint with headroom and
//!   an `NUC_MPI_BSEND_BYTES` env override; a pathological in-flight count
//!   could exhaust it and fail LOUD (`MPI_ERR_BUFFER`) — never silent
//!   corruption. See [`multi_worker`].
//! - **Non-whole-world barriers: Comm_split (TASK-0045.02). Multi-worker
//!   check-loops: rejected.** Inherited from the shared multi-worker
//!   Plan: a barrier whose participants are a strict subset of the used
//!   workers now lowers to `MPI_Comm_split` + a sub-communicator barrier
//!   (TASK-0045.02), emitted once per distinct subset OUTSIDE the rank
//!   dispatch. A multi-worker `check loop` still needs
//!   per-rank-vs-aggregate reporter semantics (TASK-0045.03) and is
//!   rejected with a typed [`EmitError`] rather than mis-emitted. With
//!   Comm_split landed, 09-producer-consumer/pipelined (host-excluding
//!   barrier `{producer,consumer}`) EMITS correctly; wiring it into the
//!   standing `check-mpi-nonblocking` gate as a value-correct cell is
//!   TASK-0046.01.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::project_skeleton::single_binary;
use backend_common::project_skeleton::CargoDependencies;
use backend_common::single_worker_main::render_single_worker_main_with_signature;
pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

mod multi_worker;

/// The rsmpi dependency line interpolated into the generated project's
/// `[dependencies]`. Pinned to the same `0.8` series the M7 foundation
/// smoke (`tests/mpi/rsmpi-smoke`) de-risked against OpenMPI 5.0.10, and
/// the series whose `immediate_buffered_send` / `immediate_matched_receive_into`
/// / `request::scope` API this backend's prelude targets.
const MPI_DEP: &str = "mpi = \"0.8\"\n";

/// The compute-entry signature the SPMD wrapper calls (single-worker arm).
/// Identical contract to mpi-blocking: the shared renderer's body is
/// emitted as this `pub fn` in `src/compute.rs` and `fn main` calls
/// `compute::nuc_compute()` after MPI_Init.
const COMPUTE_FN_SIGNATURE: &str = "pub fn nuc_compute()";

/// `#[path]` redirect so the compute module's `mod kernels;` resolves to
/// the sibling `src/kernels.rs`. Typed-attribute injection — NOT a textual
/// replace (memory: feedback-textual-replace-codegen-unsafe).
const KERNELS_MOD_ATTR: &str = "#[path = \"kernels.rs\"]\n#[allow(dead_code)]\n";

/// Paths to the files [`emit`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Path to the emitted `src/main.rs` (the SPMD MPI wrapper for the
    /// single-worker arm, or the full SPMD rank-dispatched program for
    /// the multi-worker arm).
    pub main_rs: PathBuf,
    /// Path to the emitted `src/compute.rs` (the shared single-worker
    /// compute body the wrapper calls). `None` for the multi-worker arm:
    /// the rank-dispatched program lives entirely in `main.rs`.
    pub compute_rs: Option<PathBuf>,
    /// Path to the emitted `src/kernels.rs` (verbatim copy).
    pub kernels_rs: PathBuf,
    /// Path to the emitted `run.sh` (an `mpiexec` launcher).
    pub run_sh: PathBuf,
}

/// Emit a runnable SPMD MPI Cargo project from the per-worker EventList.
/// Same `(per_worker, names, sidecar, kernels_rs_path, out_dir)`
/// contract as every tier-1 backend.
///
/// 0/1 used workers → a single binary whose compute body is the *shared*
/// single-worker renderer (byte-identical arithmetic to pthreads-sync),
/// wrapped in MPI_Init/Finalize + a `rank == 0` guard. ≥2 used workers →
/// the rank-dispatched program whose Push/Wait lower to buffered
/// `MPI_Ibsend` / `MPI_Imrecv` (see [`multi_worker`]).
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
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let compute_rs = src_dir.join("compute.rs");
    let kernels_rs = src_dir.join("kernels.rs");
    let run_sh = out_dir.join("run.sh");

    // ---- Multi-worker SPMD arm. ----
    // ≥2 used workers => one rank-dispatched binary (`match world.rank()`)
    // whose Push/Wait lower to buffered MPI Ibsend / Imrecv (tag =
    // rendezvous id) and whose Sync lowers to a whole-world `MPI_Barrier`.
    // The whole program lives in `main.rs`; there is no separate compute.rs.
    if used_workers.len() > 1 {
        let main_body = multi_worker::render_main_rs_multi(per_worker, names, sidecar)?;
        let ranks = used_workers.len();
        write_file(&kernels_rs, &kernels_src)?;
        write_file(&main_rs, &main_body)?;
        write_file(&cargo_toml, &single_binary::render_cargo_toml(CargoDependencies::section_body(MPI_DEP)))?;
        write_file(&run_sh, &render_run_sh(ranks))?;
        mark_executable(&run_sh);
        return Ok(EmitResult {
            project_dir: out_dir.to_path_buf(),
            cargo_toml,
            main_rs,
            compute_rs: None,
            kernels_rs,
            run_sh,
        });
    }

    // ---- Single-worker SPMD arm. ----
    // A single-worker schedule has NO cross-worker transfers, so there is
    // nothing to send: the compute body is the SHARED single-worker
    // renderer (byte-identical arithmetic to pthreads-sync), wrapped in
    // MPI_Init/Finalize + a `rank == 0` guard — identical to mpi-blocking's
    // single-worker arm (no buffered send involved).
    let events = used_workers
        .first()
        .and_then(|w| per_worker.get(w))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let compute_body = render_single_worker_main_with_signature(
        events,
        names,
        sidecar,
        KERNELS_MOD_ATTR,
        COMPUTE_FN_SIGNATURE,
    )?;

    write_file(&kernels_rs, &kernels_src)?;
    write_file(&compute_rs, &compute_body)?;
    write_file(&main_rs, MPI_SPMD_MAIN)?;
    write_file(&cargo_toml, &single_binary::render_cargo_toml(CargoDependencies::section_body(MPI_DEP)))?;
    write_file(&run_sh, &render_run_sh(1))?;
    mark_executable(&run_sh);

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        main_rs,
        compute_rs: Some(compute_rs),
        kernels_rs,
        run_sh,
    })
}

/// The SPMD `src/main.rs` wrapper for the SINGLE-worker arm. CONSTANT (no
/// per-schedule variation): MPI_Init via rsmpi `initialize()` (the
/// `Universe` guard runs MPI_Finalize on drop), then the shared compute
/// body runs on `rank == 0` only. No buffered-send buffer is attached —
/// a single-worker schedule has no `Push`. With `mpiexec -n 1` rank 0 is
/// the only rank; with `-n N` the extra ranks are idle.
const MPI_SPMD_MAIN: &str = "\
//! Generated by the nucleus pre-compiler (mpi-nonblocking backend, SPMD).
//! Do not edit; rerun `nucleus build` to regenerate.
//!
//! SPMD: one binary, every MPI rank runs `main`; the compute body runs
//! on rank 0. MPI_Init / MPI_Finalize wrap execution (rsmpi
//! `initialize()` + `Universe` drop). Launch with `mpiexec -n N` (PRD
//! \u{00A7}7.2). Single-worker schedule => all work is rank 0's; there are
//! no cross-worker transfers, so no buffered-send buffer is attached.

#[path = \"compute.rs\"]
mod compute;

use mpi::traits::Communicator as _;

fn main() {
    // MPI_Init. The `Universe` guard runs MPI_Finalize when dropped at
    // end of `main` — on EVERY rank, which is the required collective.
    let universe = mpi::initialize().expect(\"MPI_Init failed\");
    let world = universe.world();
    if world.rank() == 0 {
        // Byte-identical straight-line compute to the tier-1
        // single-worker backends (the shared renderer's body, emitted as
        // `pub fn nuc_compute` so it is reachable from here).
        compute::nuc_compute();
    }
}
";

/// Render the `run.sh` launcher. Builds the release binary then runs it
/// under localhost `mpiexec` (PRD §10.2). Rank count defaults to
/// `default_ranks` (1 for the single-worker arm, the used-worker count
/// for the multi-worker arm — the WORST case with all ranks live);
/// override via `NUC_MPI_RANKS`. `--oversubscribe` lets the ranks share
/// however few cores are available.
fn render_run_sh(default_ranks: usize) -> String {
    format!(
        "\
#!/usr/bin/env bash
# Generated by the nucleus pre-compiler (mpi-nonblocking). Rerun `nucleus build` to regenerate.
# Usage: bash run.sh [INPUT_BIN] [OUTPUT_BIN]   (rank count via NUC_MPI_RANKS, default {default_ranks})
# Build + run REQUIRE `nix develop .#mpi` (OpenMPI + rsmpi build deps).
# The buffered-send buffer is sized heuristically; override with
# NUC_MPI_BSEND_BYTES=<bytes> for an unusually large in-flight set.
set -euo pipefail

here=\"$(cd -- \"$(dirname -- \"${{BASH_SOURCE[0]}}\")\" && pwd)\"
input_bin=\"${{1:-input.bin}}\"
output_bin=\"${{2:-output.bin}}\"
ranks=\"${{NUC_MPI_RANKS:-{default_ranks}}}\"

(cd \"$here\" && cargo build --release --quiet)

NUC_INPUT_PATH=\"$input_bin\" \\
NUC_OUTPUT_PATH=\"$output_bin\" \\
mpiexec --oversubscribe -n \"$ranks\" \"$here/target/release/nuc-generated\"
"
    )
}

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

#[cfg(test)]
mod tests;
