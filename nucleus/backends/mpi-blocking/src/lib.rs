//! `mpi-blocking` backend — the FIRST tier-2 backend (PRD §7.2 / §11
//! M7, TASK-0045).
//!
//! Workers map to MPI ranks; transport is MPI; sync = blocking recv.
//! Codegen is **SPMD**: ONE Rust binary that dispatches on
//! `MPI_Comm_rank` (PRD §7.2), launched via `mpiexec -n N`. Output is
//! hosted Rust + the rsmpi (`mpi` crate) binding.
//!
//! # Why SPMD does NOT reuse the multi-binary `tcp_plan` substrate
//!
//! The sync-TCP backends (`mp-tcp-bufsync`, `mp-tcp-poll`) emit N
//! SEPARATE `src/bin/worker_<id>.rs` binaries wired over TCP loopback,
//! parameterised over `backend_common::tcp_plan::WirePrimitives`. MPI
//! is structurally different: ONE executable (`mpiexec -n N <same-elf>`)
//! whose body branches on `world.rank()`. So this backend does NOT route
//! through `tcp_plan`/`WirePrimitives`; it emits a single rank-dispatched
//! `main`. The compute ARITHMETIC is still the shared single-worker
//! renderer (no drift), but the process model is MPI-specific.
//!
//! # Tier-2 ship bar = COMPILE (PRD §7.4), runtime best-effort
//!
//! The generated project depends on the `mpi` crate and builds + runs
//! ONLY under `nix develop .#mpi` (OpenMPI + libclang/bindgen). It is
//! therefore EXCLUDED from the tier-1 bit-identical e2e RUNTIME matrix
//! (that matrix runs in the DEFAULT shell, which has no MPI). Its
//! acceptance is the dedicated `just check-mpi` recipe (compile under
//! `.#mpi` + a best-effort localhost `mpiexec -n N` run) — the same
//! compile-only-with-best-effort-runtime precedent as `embedded-pattern`
//! / `renode`.
//!
//! # Status: single-worker + multi-worker SPMD arms
//!
//! [`emit`] handles the 0/1-used-worker case (single-worker arm,
//! TASK-0045) AND the ≥2-used-worker case (multi-worker SPMD arm,
//! TASK-0045.01). The multi-worker arm emits one rank-dispatched binary
//! (`match world.rank()`) whose `Push`/`Wait` lower to blocking MPI
//! `Send`/`Recv` (tag = rendezvous id) and whose `Sync` lowers to a
//! whole-world `MPI_Barrier`. See [`multi_worker`] for the lowering and
//! its scope limits.
//!
//! # Honest limitations (AC#6)
//!
//! - **No real-cluster CI.** Acceptance is localhost `mpiexec -n N` only
//!   (PRD §10.2). Multi-node correctness is unproven.
//! - **Point-to-point + barrier only** — collective recognition
//!   (`MPI_Allreduce`/`MPI_Scatter`) is deliberately deferred (PRD §7.2;
//!   AC#5). Correct, just suboptimal.
//! - **Standard-mode blocking send.** `Push` lowers to `MPI_Send`
//!   (standard mode): small messages are buffered eagerly (non-blocking
//!   in practice) but a message above the MPI eager limit may block until
//!   its matching `Recv` is posted. The schedule ordering targets the
//!   fully-async pthreads `Slot` model, so an adversarial large-message
//!   ordering could deadlock. Buffered / non-blocking send is the M8
//!   non-blocking arm (TASK-0046). `just check-mpi` wraps each run in a
//!   `timeout` so a deadlock fails LOUD rather than hanging.
//! - **Non-whole-world barriers: Comm_split.** A barrier whose participant
//!   set is a strict subset of the used workers (host-excluding /
//!   non-uniform) lowers to `MPI_Comm_split` + a sub-communicator barrier
//!   (TASK-0045.02), emitted once per distinct subset OUTSIDE the rank
//!   dispatch (collective over `COMM_WORLD`, so every rank reaches every
//!   split in identical order). The lowering lives in the shared
//!   `mpi_plan` substrate, so both MPI backends inherit it; see
//!   [`multi_worker`].

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::project_skeleton::single_binary;
pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;
use pthreads_sync::render_single_worker_main_with_signature;

mod multi_worker;

/// The rsmpi dependency line interpolated into the generated project's
/// `[dependencies]`. Pinned to the same `0.8` series the M7 foundation
/// smoke (`tests/mpi/rsmpi-smoke`) de-risked against OpenMPI 5.0.10.
const MPI_DEP: &str = "mpi = \"0.8\"\n";

/// The compute-entry signature the SPMD wrapper calls. The shared
/// renderer's default `fn main` is module-private, so the compute body
/// is emitted as this `pub fn` inside `src/compute.rs` and the real
/// `fn main` in `src/main.rs` calls `compute::nuc_compute()` after
/// MPI_Init. Must agree with the `compute::nuc_compute()` call in
/// `MPI_SPMD_MAIN`.
const COMPUTE_FN_SIGNATURE: &str = "pub fn nuc_compute()";

/// `#[path]` redirect so the compute module's `mod kernels;` resolves to
/// the sibling `src/kernels.rs`. The compute body is emitted at
/// `src/compute.rs`, so its `mod kernels;` would otherwise look for
/// `src/compute/kernels.rs`; the explicit path (relative to
/// `src/compute.rs`'s own directory, `src/`) points it at the copied
/// `src/kernels.rs`. Typed-attribute injection — NOT a textual replace
/// (memory: feedback-textual-replace-codegen-unsafe).
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
    /// compute body the wrapper calls). `None` for the multi-worker arm
    /// (TASK-0045.01): the rank-dispatched program lives entirely in
    /// `main.rs` — there is no separate single-worker compute body.
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
/// one rank-dispatched binary (`match world.rank()`) whose `Push`/`Wait`
/// lower to blocking MPI `Send`/`Recv` (tag = rendezvous id) and whose
/// `Sync` lowers to a whole-world `MPI_Barrier` (TASK-0045.01; see
/// [`multi_worker`] for the lowering and its scope limits).
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

    // ---- Multi-worker SPMD arm (TASK-0045.01). ----
    // ≥2 used workers => one rank-dispatched binary (`match world.rank()`)
    // whose Push/Wait lower to blocking MPI Send/Recv (tag = rendezvous
    // id) and whose Sync lowers to a whole-world `MPI_Barrier`. The whole
    // program lives in `main.rs`; there is no separate `compute.rs`.
    if used_workers.len() > 1 {
        let main_body = multi_worker::render_main_rs_multi(per_worker, names, sidecar)?;
        let ranks = used_workers.len();
        write_file(&kernels_rs, &kernels_src)?;
        write_file(&main_rs, &main_body)?;
        write_file(&cargo_toml, &single_binary::render_cargo_toml(Some(MPI_DEP)))?;
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
    // The compute body: the SHARED single-worker renderer, byte-identical
    // arithmetic to pthreads-sync. It emits a complete module (header +
    // `#[path=kernels.rs] mod kernels;` + `fn main() { <straight-line> }`);
    // the SPMD wrapper below calls its `main` as `compute::main()`.
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
    write_file(&cargo_toml, &single_binary::render_cargo_toml(Some(MPI_DEP)))?;
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

/// The SPMD `src/main.rs` wrapper. CONSTANT (no per-schedule variation
/// in the single-worker arm): MPI_Init via rsmpi `initialize()` (the
/// `Universe` guard runs MPI_Finalize on drop), then the shared compute
/// body runs on `rank == 0` only. With `mpiexec -n 1` rank 0 is the only
/// rank; with `-n N` the extra ranks are idle (a single-worker schedule
/// has no work for them — multi-rank dispatch is TASK-0045.01).
const MPI_SPMD_MAIN: &str = "\
//! Generated by the nucleus pre-compiler (mpi-blocking backend, SPMD).
//! Do not edit; rerun `nucleus build` to regenerate.
//!
//! SPMD: one binary, every MPI rank runs `main`; the compute body runs
//! on rank 0. MPI_Init / MPI_Finalize wrap execution (rsmpi
//! `initialize()` + `Universe` drop). Launch with `mpiexec -n N` (PRD
//! \u{00A7}7.2). Single-worker schedule => all work is rank 0's.

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
/// for the multi-worker arm — the WORST case with all ranks live, per
/// TASK-0045.01 AC#4); override via `NUC_MPI_RANKS`. `--oversubscribe`
/// lets the ranks share however few cores are available.
fn render_run_sh(default_ranks: usize) -> String {
    format!(
        "\
#!/usr/bin/env bash
# Generated by the nucleus pre-compiler (mpi-blocking). Rerun `nucleus build` to regenerate.
# Usage: bash run.sh [INPUT_BIN] [OUTPUT_BIN]   (rank count via NUC_MPI_RANKS, default {default_ranks})
# Build + run REQUIRE `nix develop .#mpi` (OpenMPI + rsmpi build deps).
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
