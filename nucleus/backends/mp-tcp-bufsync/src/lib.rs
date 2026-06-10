//! mp-tcp-bufsync backend. PRD §7.1, TASK-0036/0037/0038.
//!
//! The **second** tier-1 backend, and the keystone that makes the
//! project's core thesis falsifiable: the same `(algorithm,
//! schedule)` must produce a **bit-identical** `output.bin` on two
//! independent backends. pthreads-sync (shared-memory threads) is
//! backend #1; this is `mp-tcp-bufsync` — workers are **OS
//! processes**, transport is `std::net::TcpStream` over loopback,
//! sync = blocking recv, buffered.
//!
//! ## Same contract, AlgoIR-free (TASK-0124 carried)
//!
//! Identical `emit()` signature and consumed contract as
//! pthreads-sync: `(per_worker: &BTreeMap<WorkerId,Vec<Event>>,
//! names: &NameTables, sidecar: &NameSidecar, kernels_rs_path,
//! out_dir) -> Result<EmitResult, EmitError>`. It imports neither
//! `nucleus_compiler::algo` (beyond the inert `IrExpr`/type grammar the
//! EventList carries) nor `nucleus_compiler::link`/`acfg`.
//!
//! ## No renderer drift (TASK-0124 flagged risk, addressed)
//!
//! Expression / index / kernel-call / loop-bound / single-worker
//! rendering is **not re-implemented** here — it calls the shared
//! `backend_common` renderers (`render::render_fire_args`,
//! `render::render_fire_output_assign`,
//! `single_worker_main::render_single_worker_main_with_kernels_attr`,
//! …). There is exactly ONE implementation; this crate only adds the
//! multi-PROCESS transport. That is why a single-worker example is
//! byte-identical to pthreads-sync's single process, and why the
//! multi-worker arithmetic cannot silently diverge.
//!
//! ## Topology (deterministic; no sleeps-as-sync — AC#3)
//!
//! Exactly one TCP connection per `(host, worker)` ordered pair.
//! `host` is the server: it binds a listener per non-host worker on
//! `127.0.0.1:0` (kernel-assigned ephemeral port) and ATOMICALLY
//! writes the port to `$NUC_RENDEZVOUS_DIR/<worker>.port` (via
//! tmp-file + POSIX rename). Each non-host worker is the client; it
//! polls-with-retry on that rendezvous file (600 × 10 ms = 6 s,
//! symmetric with the connect-retry bound), reads the port, then
//! `connect`s with a bounded retry loop — a refused connect only
//! means the listener is not up yet (a liveness wait, not a data
//! sync; the eventual outcome is deterministic). The worker that
//! *uses* the port also *allocates* it: there is NO close-then-rebind
//! window where some other process could grab the port (TASK-0176;
//! replaces the prior `__nuc_pick_port` helper that bound+closed
//! before the host re-bound). All Push / Wait / Sync between host
//! and a worker travel over that one stream in the schedule's
//! deterministic event order, so the `SeqTag` on each framed message
//! is a fail-loud cross-check, not routing.
//!
//! Barriers are host-mediated: every barrier in the tier-1 set is
//! `{host, w0}` (2-party); the general N-party barrier is a star
//! through host. Barrier identity is the contract-carried
//! [`nucleus_compiler::event::SyncTag`] on `Event::Sync` (TASK-0172): host
//! and every worker key the wire `barrier_cross` token by that tag,
//! which is the same value for every participant by construction —
//! so partial / non-uniform barriers (participant sets that differ
//! between barriers) lower correctly, not just uniform ones. The old
//! per-worker pre-order-index recovery and its non-uniform
//! [`EmitError::ContractGap`] are removed (TASK-0172). A barrier whose
//! participant set excludes `host` cannot be lowered AT THE BACKEND
//! directly on the one-stream-per-pair topology — that is a genuine
//! transport limitation. The original combined TASK-0175 worker-to-
//! worker filing was split into a DATA arm and a CTRL arm cycles
//! 148/149: the DATA arm was lifted in two phases — top-level w↔w
//! `Push`/`Wait` via host-relay cycle 148 (TASK-0327) and
//! in-`Repeat`-body w↔w `Push`/`Wait` via the
//! `apply_host_data_relay_inject` ACFG pass cycles 163-164b
//! (TASK-0329.01.02). The CTRL arm — host-mediated barrier mediation
//! for host-excluding barriers — was lifted cycle 160 (TASK-0329,
//! marked Done) via the `apply_host_mediation_inject` compiler pass
//! at `nucleus_compiler::passes::host_mediation_inject`, dispatched
//! from the driver before backend emit. The pass adds `host` as a
//! participant to every `Sync` whose participant set excludes it,
//! turning each host-excluding barrier into an N+1-party star through
//! host that the existing `wire::barrier_cross` shim handles natively.
//! The backend's [`EmitError::ContractGap`] rejection at
//! `Plan::build` is now defense-in-depth — it should never fire for
//! ACFGs that came through the driver's pipeline; it still bites loud
//! if an upstream change ever removes the mediation pass. For a
//! uniform-barrier program the tags are `0,1,2,…` in pre-order, so
//! generated code stays byte-identical.
//!
//! ## Inherited caveats (identical to pthreads-sync; fail-loud)
//!
//! - `block_transform` defers absolute-index rebinding to codegen;
//!   the divisible case is handled via the shared single-worker
//!   renderer, non-divisible is TASK-0173. No required mp-tcp cell
//!   exercises a blocked *multi*-worker schedule.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::project_skeleton::multi_binary;
use backend_common::project_skeleton::CargoDependencies;
use backend_common::single_worker_main::render_single_worker_main_with_kernels_attr;
pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

mod plan;

use plan::Plan;

/// Paths to the files [`emit`] wrote.
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
/// EventList. Same signature/contract as `pthreads_sync::emit`.
///
/// 0/1 used workers → a single binary whose body is the *shared*
/// single-worker renderer (byte-identical arithmetic to
/// pthreads-sync) plus a no-op run.sh. ≥2 used workers → one binary
/// per used worker, TCP-wired, plus a run.sh that launches them.
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

    // Shared modules every worker binary `#[path]`-includes.
    //
    // `wire.rs` is emitted PRISTINE: it is a byte-for-byte copy of
    // `mp_tcp_common::WIRE_RUNTIME_SRC` with no test-only branch on this
    // (or any) codegen path. The cross-backend negative falsifier
    // (`NUC_XBACKEND_NEGATIVE`) used to corrupt this source inline here;
    // it was relocated entirely harness-side to nucleus-e2e
    // (`maybe_corrupt_wire_for_xbackend`, post-`nucleus build`,
    // pre-compile) in TASK-0183 — parallel to TASK-0157/0187 for the
    // `NUC_NONDET_TEST` seam. Production codegen now carries zero
    // self-corruption / no `NUC_XBACKEND_NEGATIVE` read.
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&wire_rs, mp_tcp_common::WIRE_RUNTIME_SRC)?;

    if used_workers.len() <= 1 {
        // Single process: reuse the SHARED single-worker renderer so
        // the emitted arithmetic is byte-identical to pthreads-sync.
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
            &multi_binary::render_cargo_toml(
                &[String::from("nuc-generated")],
                CargoDependencies::none(),
            ),
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

    // ---- Multi-process path. ----
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
    // No port-picker binary: each host worker binds 127.0.0.1:0
    // itself and rendezvouses the OS-assigned port through a file
    // (see Plan::render_worker_program / Plan::render_run_sh).
    // TASK-0176 deleted the previous `__nuc_pick_port` helper because
    // its bind-print-exit shape opened a close-then-rebind TOCTOU
    // window between the picker exiting and the host re-binding.

    write_file(
        &cargo_toml,
        &multi_binary::render_cargo_toml(&bin_names, CargoDependencies::none()),
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

// --------------------------------------------------------------------
// Cargo.toml / run.sh renderers
// --------------------------------------------------------------------
//
// TASK-0257 (cycle 112) lifted the inline `render_cargo_toml` +
// `render_run_sh_single` + the bulk of `Plan::render_run_sh` into
// `backend_common::project_skeleton::multi_binary`. The remaining
// surface here is the per-backend SO_BUF commentary block consumed
// by the shared multi-process emitter (see `SO_BUF_COMMENT_BUFSYNC`
// below) and the data-shape glue around `Plan::max_payload_bytes` /
// `Plan::non_host_workers` that the shared emitter needs as
// parameters.

/// Per-backend SO_BUF commentary block interpolated by the shared
/// [`multi_binary::render_run_sh_multi`] before the
/// `export NUC_SO_BUF=...` line. mp-tcp-bufsync is sync (`buffer=1`)
/// so the requirement is one message in flight — the shared call
/// passes the largest cross-worker payload (sum of element bytes,
/// 64 KiB floor) and this comment explains the sync sizing.
pub(crate) const SO_BUF_COMMENT_BUFSYNC: &str =
    "# Socket buffer requirement from the schedule's per-channel\n\
     # buffer needs (largest single transfer payload, sync=1 msg).\n";

/// `#[path]` attribute block that redirects the shared single-worker
/// renderer's `mod kernels;` at the copied sibling `../kernels.rs`,
/// because mp-tcp-bufsync emits the binary under `src/bin/` rather
/// than as a sibling of `src/kernels.rs`. Passed to
/// `backend_common::single_worker_main::render_single_worker_main_with_kernels_attr`
/// as a typed parameter (TASK-0177) — replaces the prior `replacen`
/// token-match against the renderer's literal `mod kernels;` spelling.
const KERNELS_MOD_ATTR_FOR_SRC_BIN: &str = "#[path = \"../kernels.rs\"]\n#[allow(dead_code)]\n";

// --------------------------------------------------------------------
// File helpers
// --------------------------------------------------------------------

// NUC_XBACKEND_NEGATIVE seam (TASK-0178) — RELOCATED OUT of this crate.
//
// The cross-backend negative falsifier used to corrupt the emitted
// `wire.rs` here, inline on the production codegen path
// (`maybe_corrupt_wire`, deleted in TASK-0183). It was a safe seam
// (gated / deterministic / loud / anchor-guarded) but production
// codegen carrying a self-corruption branch is not a clean seam.
//
// The e2e harness is the SOLE consumer of `NUC_XBACKEND_NEGATIVE`
// (only `just xbackend-check-negative` sets it), so — exactly as
// TASK-0157/0187 did for the sibling `NUC_NONDET_TEST` seam — the
// whole branch now lives harness-side: nucleus-e2e applies the
// `wire.rs` rewrite as a post-`nucleus build`, pre-compile
// post-process of the emitted mp-tcp tree
// (`maybe_corrupt_wire_for_xbackend` in `e2e/src/main.rs`). This crate
// now does ZERO `std::env::var("NUC_XBACKEND_NEGATIVE")` and has no
// corruption branch on any codegen path; `emit()` writes `wire.rs`
// byte-identical to `mp_tcp_common::WIRE_RUNTIME_SRC`. The exact-`"1"`
// gate, loud banner, and anchor-drift hard-failure all moved verbatim
// to the harness with behaviour preserved.

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

// `io` is referenced via `EmitError` (re-exported from pthreads-sync)
// for the `KernelsReadFailed`/`WriteFailed` variants; the alias keeps
// the use explicit for readers.
#[allow(unused_imports)]
use io as _io;
