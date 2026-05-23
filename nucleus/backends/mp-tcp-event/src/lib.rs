//! mp-tcp-event backend. PRD §7.1, TASK-0042.02.
//!
//! The **fourth** tier-1 backend, and the SYNC -> ASYNC upgrade of
//! mp-tcp-bufsync. The relationship mirrors pthreads-async / pthreads-
//! sync: same transport (here: TCP loopback), same project skeleton
//! shape (here: one binary per worker + run.sh), upgraded notify =
//! event (mio + epoll for inbound readiness) + supports_buffer = true
//! (bounded ring buffer per cross-worker channel).
//!
//! ## Same contract, AlgoIR-free (TASK-0124 carried)
//!
//! Identical `emit()` signature and consumed contract as the three
//! shipped tier-1 backends: `(per_worker, names, sidecar,
//! kernels_rs_path, out_dir) -> Result<EmitResult, EmitError>`. No
//! `compiler::algo` / `compiler::link` / `compiler::acfg` access
//! beyond the inert `IrExpr` / `ResolvedType` grammar the EventList
//! carries.
//!
//! ## Implementation status (TASK-0042.02 cycle 41 — Stages 1+2)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`): IMPLEMENTED.
//!   Delegates to `pthreads_sync::render_single_worker_main` so the
//!   emitted arithmetic is byte-identical to pthreads-sync's (and to
//!   pthreads-async's / mp-tcp-bufsync's) single-worker output.
//!   Project skeleton mirrors mp-tcp-bufsync's `src/bin/<name>.rs`
//!   layout because the multi-worker arm (Stage 3) will also use it
//!   — that keeps `EmitResult::worker_bins` semantically uniform
//!   across both mp-tcp-* backends. The wire.rs sidecar file is
//!   emitted byte-identical to mp_tcp_common::WIRE_RUNTIME_SRC (same
//!   single-source-of-truth precedent as mp-tcp-bufsync) so the
//!   multi-worker arm landing later does not have to re-touch the
//!   single-worker artefact.
//!
//! - **Multi-worker arm** (`used_workers.len() >= 2`): NOT YET
//!   IMPLEMENTED. Returns [`EmitError::ContractGap`] with a precise
//!   forward-link to TASK-0042.05 (Stage 3 of TASK-0042.02). The blocker is the mio
//!   reactor + per-(src,dst) ring-buffer codegen — the bulk of the
//!   work. Single-worker e2e cells in this cycle exercise the Stage
//!   1+2 surface (driver dispatch + capability-compat + delegated
//!   single-worker emit). Multi-worker e2e cells (09/pipelined,
//!   11/pipelined, 13/pipeline_parallel × mp-tcp-event) wait on
//!   Stage 3.
//!
//! ## Why split single-worker vs multi-worker into separate cycles
//!
//! Single-worker mp-tcp-event has NO cross-worker Push/Wait events
//! (the shared single-worker emitter `ContractGap`s on either; same
//! pattern inherited via delegation). The mio reactor + ring buffer
//! only fires across two or more workers. Splitting the single-worker
//! arm off as Stages 1+2 keeps it a mechanical-delegation unit and
//! quarantines the multi-cycle mio + ring-buffer headline work under
//! Stage 3 — the EXACT discipline pthreads-async followed in cycle
//! 16 (TASK-0226 single-worker) vs cycle 26 (TASK-0228 multi-worker).
//!
//! ## Forward-carried context for Stage 3 (multi-worker)
//!
//! 1. **Topology**: one TCP connection per `(host, worker)` ordered
//!    pair, same as mp-tcp-bufsync; the reactor multiplexes inbound
//!    readiness on EVERY socket via epoll (mio's `Poll` + `Events`).
//!    Outbound writes go through a per-pair `VecDeque<Vec<u8>>` ring
//!    buffer drained on writable readiness.
//! 2. **Ring buffer contract** (post-TASK-0213): one
//!    `BoundedFrameRing` per `(DataId, SeqTag, src_worker,
//!    dst_worker)` tuple. Capacity = `transfer DATA : buffer=N`.
//!    Ring STARTS EMPTY (D is the analysis-only sizing invariant,
//!    NOT a runtime pre-fill).
//! 3. **Reuse the shared walker**: the multi-worker codegen body
//!    will route through `backend_common::multi_worker_walker::
//!    render_worker_events` with `rendezvous_prefix = "chan"` (or a
//!    chosen alternative — `"ring"` would collide with pthreads-
//!    async if the emitted Rust binaries are ever side-by-side
//!    diff'd, hence the distinct prefix). Same SHARED check_frame
//!    instrumentation as the other tier-1 backends.
//! 4. **Wire codec**: copy `mp_tcp_common::WIRE_RUNTIME_SRC`
//!    verbatim, as mp-tcp-bufsync does. The wire FORMAT is the same
//!    across the two mp-tcp-* backends; only the reactor that pumps
//!    it differs.
//! 5. **mio dependency** (Stage 3): `mio = "0.8"` is the one
//!    well-known crate allowance per PRD §12 ("one well-known crate"
//!    rule). Pure epoll-readiness multiplexer, no async runtime, no
//!    tokio, no futures. The honest limitation note belongs in the
//!    Stage-3 PR: mio polling adds per-Push/Wait reactor-trip
//!    overhead (~µs) above raw TCP send/recv; mp-tcp-bufsync's
//!    blocking sync path has lower latency but cannot satisfy the
//!    async/buffered capability surface.
//!
//! ## Honest limitations
//!
//! - Multi-worker codegen is `ContractGap` (Stage 3). Until Stage 3
//!   lands, the *only* schedules this backend runs end-to-end are
//!   those whose used-worker count is 0 or 1 — which is already
//!   covered by the three shipped tier-1 backends. Single-worker
//!   mp-tcp-event is therefore a *capability-check gateway*
//!   (capability surface satisfiable) more than a new runtime
//!   transport — the real value lands when Stage 3 ships the
//!   mio + ring-buffer multi-worker arm and Stage 4 wires
//!   09/pipelined, 11/pipelined, 13/pipeline_parallel ×
//!   mp-tcp-event into e2e-matrix.toml.
//!
//! - The `transport = "tcp"` capability differs from pthreads-async's
//!   `"shared-memory"`, so a schedule whose capability surface (via
//!   docs/capabilities-toml.md) is satisfied by pthreads-async is
//!   ALSO satisfied by mp-tcp-event once Stage 3 lands. The two are
//!   independent columns of the differential matrix on the same
//!   pipelined schedules.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// Re-export the SHARED error + reverse name tables. Same precedent
// as the three shipped tier-1 backends (pthreads-sync owns
// EmitError's Display arms; per-backend prefix is owned by the
// driver dispatch site). The cross-backend differential requires
// identical NameTables input across every backend — they are built
// ONCE by the driver and fed to every backend's emit().
pub use backend_common::EmitError;
pub use compiler::NameTables;

use compiler::event::{Event, WorkerId};
use compiler::sidecar::NameSidecar;

// Single-worker emit delegate. The shared single-worker renderer
// lives in pthreads-sync (the pthreads-sync-specific straight-line
// main.rs emitter); mp-tcp-event, pthreads-async, and mp-tcp-bufsync
// all DELEGATE to it for the 0/1-used-worker arm. This is THE one
// semantic inter-backend arrow — Cargo.toml + run.sh now come from
// `backend_common::project_skeleton`, but the straight-line emitter
// is genuinely pthreads-sync-owned.
use pthreads_sync::render_single_worker_main;

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

/// Paths to the files [`emit`] wrote.
///
/// Shape mirrors `mp_tcp_bufsync::EmitResult` (multi-binary layout
/// under `src/bin/`) because the multi-worker arm (Stage 3) will also
/// emit one binary per worker. Single-worker emit (this cycle) yields
/// `worker_bins.len() == 1` — the `src/bin/nuc-generated.rs` artefact
/// — keeping the result shape uniform across single- and multi-
/// worker paths so the driver dispatch arm + downstream e2e harness
/// can pattern-match identically across both.
///
/// Defined locally (not a re-export of `mp_tcp_bufsync::EmitResult`)
/// for the same reason pthreads-async defines its own: the *backend
/// identity* is part of the result — a caller inspecting `EmitResult`
/// should not be lied to about which crate produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Per-worker `src/bin/<worker>.rs` paths. Single-worker emit:
    /// exactly one entry (`nuc-generated.rs`). Multi-worker emit
    /// (Stage 3, not yet landed — TASK-0042.05): one entry per used
    /// worker. The handshake will use the rendezvous-file pattern
    /// landed by TASK-0176 in mp-tcp-bufsync (host binds 127.0.0.1:0
    /// itself + writes the port to `$NUC_RENDEZVOUS_DIR/<w>.port`;
    /// non-host worker polls + reads + connects) — do NOT reintroduce
    /// the close-then-rebind `__nuc_pick_port` helper.
    pub worker_bins: Vec<PathBuf>,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `src/wire.rs` (the copied protocol v0).
    /// Single-worker emit still writes this file so the multi-
    /// worker arm landing later does not have to re-touch the
    /// single-worker artefact; the single-worker binary does not
    /// reference it.
    pub wire_rs: PathBuf,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

/// Emit a runnable Cargo project from the per-worker EventList.
///
/// Wire contract (AC#1, TASK-0124): consumes the per-worker [`Event`]
/// lists + the [`NameTables`] + the [`NameSidecar`]. **No `&ACFG` /
/// `&LinkedIR` access**, exactly like the three shipped tier-1
/// backends.
///
/// Dispatch:
///
/// - `used_workers <= 1` -> SINGLE-WORKER. Delegates to the SHARED
///   `pthreads_sync::render_single_worker_main` so the emitted
///   arithmetic is byte-identical to pthreads-sync's. Project skeleton
///   uses the mp-tcp-bufsync-style `src/bin/nuc-generated.rs` layout
///   (so the result shape is uniform with the Stage-3 multi-worker
///   arm).
/// - `used_workers >= 2` -> MULTI-WORKER. Returns
///   [`EmitError::ContractGap`] pointing at TASK-0042.05 (Stage 3 of TASK-0042.02)
///   (the mio reactor + per-(src,dst) ring-buffer codegen).
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

    // Shared modules every worker binary `#[path]`-includes. The
    // wire.rs file is emitted byte-identical to
    // `mp_tcp_common::WIRE_RUNTIME_SRC` (single source of truth shared
    // with mp-tcp-bufsync's tests). Single-worker emit does NOT
    // reference wire.rs in its body, but writing it is harmless and
    // keeps the artefact uniform across single- and multi-worker
    // paths.
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&wire_rs, mp_tcp_common::WIRE_RUNTIME_SRC)?;

    if used_workers.len() <= 1 {
        // Single process: reuse the SHARED single-worker renderer so
        // the emitted arithmetic is byte-identical to pthreads-sync.
        // The body's `mod kernels;` is rewritten to a `#[path]` form
        // pointing at the sibling `src/kernels.rs` (the `src/bin/`
        // layout needs the path redirect; pthreads-async and
        // pthreads-sync use `src/main.rs` and do not).
        let events = used_workers
            .first()
            .and_then(|w| per_worker.get(w))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let body = render_single_worker_main(events, names, sidecar)?;
        let bin_path = bin_dir.join("nuc-generated.rs");
        write_file(&bin_path, &wrap_single_worker(&body))?;
        write_file(
            &cargo_toml,
            &render_cargo_toml(&[String::from("nuc-generated")]),
        )?;
        write_file(&run_sh, &render_run_sh_single())?;
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

    // Multi-worker arm (Stage 3, not yet landed). The capability-
    // compat check has already verified the schedule's surface IS
    // satisfiable by this backend (capabilities.toml declares
    // supports_async + supports_buffer + notify=event); the runtime
    // gap is in CODEGEN only — fail LOUD with a precise forward-link
    // so a user targeting this backend with a multi-worker schedule
    // sees a typed error, not a panic or a wrong binary.
    Err(EmitError::ContractGap(format!(
        "mp-tcp-event multi-worker codegen is not yet implemented \
         (used_workers = {n}). The single-worker arm (used_workers <= 1) \
         is functional and delegates to the shared single-worker \
         renderer; the multi-worker arm (mio reactor + per-(src,dst) \
         ring buffer over TCP loopback) is tracked under TASK-0042.05 \
         (TASK-0042.05 (Stage 3 of TASK-0042.02)). Use pthreads-async for now if your \
         schedule needs the async/buffered capability surface on \
         >=2 workers.",
        n = used_workers.len(),
    )))
}

// --------------------------------------------------------------------
// Cargo.toml / run.sh / single-worker wrap renderers
// --------------------------------------------------------------------
//
// These mirror mp-tcp-bufsync's identical helpers byte-for-byte (the
// `src/bin/` project skeleton is independent of the runtime semantics
// — sync-blocking vs mio-reactor differ in the WORKER BODY, not in
// the Cargo manifest or the single-process run.sh). When Stage 3
// lands we MAY consider lifting `render_cargo_toml(bin_names: &[..])`
// + `render_run_sh_single()` into
// `backend_common::project_skeleton::multi_binary` (a sibling module
// to `single_binary`); that would close the duplication with
// mp-tcp-bufsync. Not done now to keep Stages 1+2 a minimal-delta
// landing.

fn render_cargo_toml(bin_names: &[String]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from(
        "# Generated by the mp-tcp-event backend. Do not edit; rerun \
         `nucleus build` to regenerate.\n\
         [package]\n\
         name        = \"nuc-generated\"\n\
         version     = \"0.0.0\"\n\
         edition     = \"2021\"\n\
         publish     = false\n\
         \n\
         [workspace]\n\
         # Empty: standalone, not part of any parent workspace.\n\
         \n",
    );
    for b in bin_names {
        // Each worker is its own binary target. `src/bin/<name>.rs`
        // is the conventional auto-discovered location but we declare
        // it explicitly so the manifest is the single source of truth
        // for which binaries exist (deterministic, greppable).
        writeln!(s, "[[bin]]").ok();
        writeln!(s, "name = \"{b}\"").ok();
        writeln!(s, "path = \"src/bin/{b}.rs\"").ok();
        writeln!(s).ok();
    }
    s.push_str("[profile.release]\npanic = \"abort\"\n");
    s
}

/// Single-process run.sh: build + run the one binary. Mirrors the
/// pthreads-sync / mp-tcp-bufsync single-process run.sh contract
/// (INPUT_BIN OUTPUT_BIN positional).
fn render_run_sh_single() -> String {
    String::from(
        "#!/usr/bin/env bash\n\
         # Generated by the mp-tcp-event backend (single-process: \
         0/1 used workers). Rerun `nucleus build` to regenerate.\n\
         # Usage: bash run.sh INPUT_BIN OUTPUT_BIN\n\
         set -euo pipefail\n\
         \n\
         here=\"$(cd -- \"$(dirname -- \"${BASH_SOURCE[0]}\")\" && pwd)\"\n\
         input_bin=\"${1:-input.bin}\"\n\
         output_bin=\"${2:-output.bin}\"\n\
         \n\
         (cd \"$here\" && cargo build --release --quiet)\n\
         \n\
         NUC_INPUT_PATH=\"$input_bin\" \\\n\
         NUC_OUTPUT_PATH=\"$output_bin\" \\\n\
         \"$here/target/release/nuc-generated\"\n",
    )
}

/// Wrap a shared single-worker `main.rs` body. The shared renderer
/// emits `mod kernels;` + `fn main() {...}`; for the
/// `src/bin/<name>.rs` layout we redirect `mod kernels` at the copied
/// `../kernels.rs` via `#[path]`. Same one-line substitution
/// mp-tcp-bufsync uses; no other change to the body.
fn wrap_single_worker(shared_body: &str) -> String {
    shared_body.replacen(
        "mod kernels;",
        "#[path = \"../kernels.rs\"]\n#[allow(dead_code)]\nmod kernels;",
        1,
    )
}

// --------------------------------------------------------------------
// File helpers
// --------------------------------------------------------------------

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
