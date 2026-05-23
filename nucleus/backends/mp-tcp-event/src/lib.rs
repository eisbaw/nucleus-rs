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
//! `nucleus_compiler::algo` / `nucleus_compiler::link` / `nucleus_compiler::acfg` access
//! beyond the inert `IrExpr` / `ResolvedType` grammar the EventList
//! carries.
//!
//! ## Implementation status (TASK-0042.05 cycle 79 — Stages 1+2+3 LANDED)
//!
//! - **Single-worker arm** (`used_workers.len() <= 1`): IMPLEMENTED
//!   (TASK-0042.02 Stages 1+2 cycle 41). Delegates to
//!   `pthreads_sync::render_single_worker_main` so the emitted
//!   arithmetic is byte-identical to pthreads-sync's (and to
//!   pthreads-async's / mp-tcp-bufsync's) single-worker output.
//!   Project skeleton uses the multi-process `src/bin/<name>.rs`
//!   layout so the `EmitResult::worker_bins` shape stays uniform
//!   with the multi-worker arm. The wire.rs sidecar file is
//!   byte-identical to `mp_tcp_common::WIRE_RUNTIME_SRC`.
//!
//! - **Multi-worker arm** (`used_workers.len() >= 2`): IMPLEMENTED
//!   (TASK-0042.05 / Stage 3 cycle 79). Emits one
//!   `src/bin/<wname>.rs` per used worker plus a shared
//!   `src/runtime.rs` (the mio reactor + `Chan<T>`; verbatim from
//!   [`RUNTIME_SRC`] — same single-source-of-truth precedent as
//!   `mp_tcp_common::WIRE_RUNTIME_SRC`). The shared event-walker
//!   [`backend_common::multi_worker_walker::render_worker_events`]
//!   drives Push/Wait/Loop/Fire with `rendezvous_prefix = "chan"`;
//!   barriers go through a per-worker `Bar<bid>` shim on the
//!   synchronous CTRL channel (`wire::barrier_cross`, same as
//!   mp-tcp-bufsync). The rendezvous-file handshake (TASK-0176)
//!   replaces the deleted close-then-rebind `__nuc_pick_port` helper.
//!
//! ## Topology + runtime substrate (cycle-79 LANDED)
//!
//! 1. **Two sockets per (host, worker) pair**: DATA (mio-managed,
//!    non-blocking) for Push/Wait; CTRL (`std::net::TcpStream`,
//!    blocking) for barriers. Mirrors mp-tcp-bufsync's DATA+CTRL
//!    split — both backends need it because producer/consumer
//!    barrier-vs-data ordering can differ on each side of a
//!    `(host,worker)` pair, and a single FIFO would corrupt frame
//!    demuxing. mp-tcp-event additionally demultiplexes DATA by
//!    `seq` (per-seq inbound queue), not by arrival order; the CTRL
//!    channel keeps its own ordered semantics.
//!
//! 2. **Per-(seq, peer) outbound queues** sized from
//!    `sidecar.transfer_buffer_for_seq[seq]` (TASK-0233 contract).
//!    `Chan<T>::push(v)` enqueues then opportunistically drains
//!    non-blockingly (so producer can move on without parking the
//!    consumer); blocks on the back-pressure point when
//!    `queue.len() >= cap`. The reactor's `pump_once` services
//!    READABLE + WRITABLE readiness via `mio::Poll`.
//!
//! 3. **Per-seq inbound queues**. The reactor reads framed messages
//!    on every peer DATA socket and routes by `seq` into
//!    `inbound[seq]`; `Chan<T>::wait()` blocks polling until the
//!    matching queue has at least one element. Two Pushes from
//!    different peers cannot collide on the same `seq` because
//!    `(DataId, SeqTag)` uniquely identifies one Push/Wait pair.
//!
//! 4. **Host-mediated barrier topology** (TASK-0175 limit): same as
//!    mp-tcp-bufsync. Every barrier must include the host worker
//!    because there is only one CTRL stream per `(host, worker)`
//!    pair; a worker-to-worker barrier needs a w↔w mesh that the
//!    star topology lacks. Fail-loud with a typed `ContractGap`
//!    forward-linking TASK-0175 — never a wrong binary.
//!
//! ## mio dependency
//!
//! The emitted Cargo.toml declares `mio = { version = "0.8",
//! default-features = false, features = ["os-poll", "net"] }` —
//! the PRD §12 "one well-known crate" allowance. Pure
//! epoll-readiness multiplexer, no async runtime, no tokio, no
//! futures.
//!
//! ## Honest limitations
//!
//! - **mio reactor-trip overhead**: every Push/Wait that crosses
//!   the reactor incurs a `poll()` round-trip when the socket would
//!   block. On loopback for steady-state pipelines (small frames,
//!   kernel accepts immediately) this is negligible; under
//!   back-pressure it adds ~µs per blocked push. mp-tcp-bufsync's
//!   blocking-sync path has lower latency on the contended case but
//!   cannot satisfy the async/buffered capability surface — these
//!   two backends are the trade-off column for TCP-transport
//!   pipelined schedules.
//!
//! - **Worker-to-worker channel**: cycle-79 still leaves
//!   `mp-tcp-event` constrained to the host-mediated star topology
//!   for both DATA and CTRL. Schedules with worker-to-worker Push
//!   pairs OR host-excluding barriers fail-loud with a
//!   `ContractGap` naming TASK-0175. AC#2 (09/pipelined) and AC#4
//!   (13/pipeline_parallel) of TASK-0042.05 are therefore blocked
//!   on TASK-0175, not a regression of this cycle — same blocker
//!   mp-tcp-bufsync has on the same w↔w surface.
//!
//! - **Determinism boundary**: the wire frame ORDER per pair is
//!   schedule-determined (the projection emits Pushes in event-list
//!   order, and the reactor enqueues + drains in BTreeMap-sorted
//!   `(seq, peer)` order). Cross-pair interleaving on a single peer
//!   socket is determined by ascending `seq` for any one drain
//!   sweep — deterministic by construction; the loopback kernel
//!   does not reorder TCP bytes. Verified bit-identical against
//!   reference.bin on 3 cells cycle 79 (02-split-add/split,
//!   11-game-of-life/pipelined, 13-cnn-inference/batch_parallel).

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
pub use nucleus_compiler::NameTables;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

// Single-worker emit delegate. The shared single-worker renderer
// lives in pthreads-sync (the pthreads-sync-specific straight-line
// main.rs emitter); mp-tcp-event, pthreads-async, and mp-tcp-bufsync
// all DELEGATE to it for the 0/1-used-worker arm. This is THE one
// semantic inter-backend arrow — Cargo.toml + run.sh now come from
// `backend_common::project_skeleton`, but the straight-line emitter
// is genuinely pthreads-sync-owned.
use pthreads_sync::render_single_worker_main;

// Multi-worker codegen (TASK-0042.05). The mio reactor + per-(seq,
// peer) outbound queue + per-seq inbound queue substrate lives in
// `runtime_src.rs` as a SINGLE SOURCE OF TRUTH (same precedent as
// `mp_tcp_common::WIRE_RUNTIME_SRC` — one file the host crate's
// `cargo test` build compiles AND the generated project includes
// verbatim as `src/runtime.rs`; see the `#[cfg(test)] mod
// runtime_src` below). The Plan + per-worker emitter live in
// `multi_worker.rs`.
mod multi_worker;
use multi_worker::Plan;

/// The mp-tcp-event runtime: mio reactor + per-(seq, peer) outbound
/// queue + per-seq inbound queue. Emitted verbatim into every
/// generated multi-process project as `src/runtime.rs`. Same single-
/// source-of-truth pattern as `mp_tcp_common::WIRE_RUNTIME_SRC`.
pub const RUNTIME_SRC: &str = include_str!("runtime_src.rs");

/// Host-side compile-check of `runtime_src.rs`. The file is otherwise
/// only `include_str!`'d as a string (above); without this `mod`
/// declaration the host crate's `cargo test` would never compile the
/// reactor at all, and a typo / mio API break / `HEADER_LEN` drift
/// against `mp_tcp_common::wire_runtime` would surface only on the
/// first e2e cell that runs a generated project. Architect-review
/// finding F1 of TASK-0042.05: the previous "build-time check"
/// comment was a doc-lie (no such check existed); this declaration
/// makes it true. `mio` is a dev-dependency so this compiles under
/// `cargo test --workspace` but not under a downstream consumer of
/// `mp-tcp-event` that doesn't want the mio dep transitively.
#[cfg(test)]
mod runtime_src;

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
    /// (TASK-0042.05 cycle 79, LANDED): one entry per used worker.
    /// The handshake uses the rendezvous-file pattern landed by
    /// TASK-0176 in mp-tcp-bufsync (host binds 127.0.0.1:0 itself +
    /// writes the port to `$NUC_RENDEZVOUS_DIR/<w>.port`; non-host
    /// worker polls + reads + connects) — never reintroduce the
    /// close-then-rebind `__nuc_pick_port` helper.
    pub worker_bins: Vec<PathBuf>,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `src/wire.rs` (the copied protocol v0).
    /// Single-worker emit still writes this file so the multi-
    /// worker arm landing later does not have to re-touch the
    /// single-worker artefact; the single-worker binary does not
    /// reference it.
    pub wire_rs: PathBuf,
    /// Path to the emitted `src/runtime.rs` (the mio reactor + ring
    /// buffer + chan substrate). Only present on the multi-worker
    /// emit path (TASK-0042.05); single-worker emit skips this file
    /// because the delegated single-worker renderer has no
    /// cross-worker Push/Wait.
    pub runtime_rs: Option<PathBuf>,
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
///   so the result shape is uniform with the multi-worker arm.
/// - `used_workers >= 2` -> MULTI-WORKER (TASK-0042.05 / Stage 3
///   landed cycle 79). Emits one `src/bin/<wname>.rs` per used
///   worker + shared `src/runtime.rs` (mio reactor + `Chan<T>`,
///   verbatim from [`RUNTIME_SRC`]) + `src/wire.rs` + run.sh. Push
///   sites lower to `chan_<rid>.push(name.clone())`, Wait sites to
///   `chan_<rid>.wait()`, barriers via per-worker `Bar<bid>` shims
///   over the synchronous CTRL channel. Returns
///   [`EmitError::ContractGap`] only when the schedule needs a
///   worker-to-worker channel (host-excluding barrier OR
///   worker-to-worker Push) — same transport limit mp-tcp-bufsync
///   has (filed as TASK-0175).
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
            runtime_rs: None,
            run_sh,
        });
    }

    // ---- Multi-worker arm (TASK-0042.05). ----
    //
    // Emit the runtime substrate (mio reactor + Chan<T>) verbatim
    // from `RUNTIME_SRC`, then one `src/bin/<wname>.rs` per used
    // worker via the per-worker renderer in `multi_worker.rs`. The
    // shared event walker drives Push/Wait/Loop/Fire; Sync goes
    // through a barrier shim emitted per worker (CTRL-channel
    // wire::barrier_cross, same as mp-tcp-bufsync).
    let runtime_rs_path = src_dir.join("runtime.rs");
    write_file(&runtime_rs_path, RUNTIME_SRC)?;

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

    write_file(&cargo_toml, &multi_worker::render_cargo_toml(&bin_names))?;
    write_file(&run_sh, &multi_worker::render_run_sh(&plan)?)?;
    mark_executable(&run_sh);

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        worker_bins,
        kernels_rs,
        wire_rs,
        runtime_rs: Some(runtime_rs_path),
        run_sh,
    })
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
