//! pthreads-async backend (SKELETON). PRD §7.1, TASK-0042.01.
//!
//! Tier-1 CPU backend: `std::thread` + `std::sync::Condvar` +
//! bounded ring buffer per `(DataId, SeqTag)`. Shared-memory
//! transport, single binary. **supports_async=true,
//! supports_buffer=true** — the third tier-1 backend, completing
//! the async/buffered surface that pthreads-sync and mp-tcp-bufsync
//! cannot.
//!
//! # Current status: SKELETON (TASK-0042.01 cycle 16, 2026-05-21)
//!
//! This crate is the **foundation** for the third tier-1 backend:
//!
//! - `Cargo.toml` + workspace registration: DONE.
//! - `capabilities.toml`: DONE (tier-1, shared-memory, notify=[event],
//!   async+buffer support, max_buffer=64).
//! - Public [`emit`] entry point with the pthreads-sync signature: DONE,
//!   but returns [`EmitError::ContractGap`] until ring-buffer codegen
//!   lands.
//! - Driver dispatch arm: DONE (`--backend pthreads-async` resolves to
//!   this crate).
//!
//! The actual codegen body is intentionally NOT in this cycle. Splitting
//! the foundation off lets the next implementer step directly into
//! ring-buffer + Condvar work with a fresh context budget, without
//! bikeshedding crate layout, Cargo wiring, capabilities.toml schema,
//! or driver dispatch.
//!
//! # Follow-up sub-tasks (filed in this cycle)
//!
//! - **TASK-0226**: Single-worker ring-buffer + Condvar codegen.
//!   `std::sync::Mutex<VecDeque<T>>` + two `Condvar`s per
//!   `(DataId, SeqTag)`. Ring STARTS EMPTY (D is sizing, not a fill
//!   input — see the post-TASK-0213 corrected contract in TASK-0042.01
//!   notes).
//! - **TASK-0227**: Single-worker `check_frame` codegen (Panic / Log /
//!   Count). Reuses the shared helpers `pthreads_sync::{sanitize_loop_var,
//!   collect_count_check_frames, emit_count_reporter_struct,
//!   CountCheckLoop}` per the TASK-0052.04 forward-carry on TASK-0042.01.
//! - **TASK-0228**: Multi-worker arm. Initial: reject with
//!   `EmitError::ContractGap` (mirror the pthreads-sync multi-worker
//!   `check_frame=Some` pattern). Full multi-worker pipelined emit
//!   deferred to a further sub-task.
//! - **TASK-0229**: e2e cells (examples 9 producer/consumer pipe + 11
//!   Game of Life multi-iter) + bit-identical differential vs the
//!   reference oracle. The cross-backend differential gate becomes
//!   three-way (pthreads-sync, mp-tcp-bufsync, pthreads-async) once
//!   AC#4 lands.
//!
//! # Why a skeleton-first cycle?
//!
//! The ring-buffer codegen is genuinely multi-cycle: check_frame
//! integration + multi-worker arm + e2e cells + bit-identical
//! differential gate. The IR contract carrying the per-seq pipeline
//! depth (`ACFG::pipeline_depth_for_seq`) is itself <1 month old
//! (TASK-0134), and the marking-aware boundedness pass (TASK-0213) +
//! the link-step's `D <= N` invariant are the load-bearing
//! preconditions a runtime ring-buffer relies on. Splitting the
//! foundation off keeps the diff reviewable and the gate-bisectable
//! risk small.
//!
//! # Forward-carried context the next implementer should read first
//!
//! 1. **TASK-0042.01 notes** — the parent task carries: ring-buffer
//!    pre-fill contract (STARTS EMPTY post-TASK-0213), `D` is the
//!    sizing invariant not a fill input, `Place::initial_marking = D`
//!    is the analysis encoding, EventList NOT YET carrying D (option
//!    (c) recommended: read `buffer=N` directly).
//! 2. **TASK-0124** — EventList contract (no AlgoIR/ACFG access).
//! 3. **TASK-0052.04 + 0052.05** — check_frame codegen pattern (single-
//!    + multi-worker), shared helpers from pthreads-sync.
//! 4. **TASK-0214** — same-worker carveout (no Xfer for same-worker
//!    data; ignore harmless `transfer X : buffer=N` in codegen).
//! 5. **TASK-0216** — partition + pipeline: per fan-out pair
//!    `(DataId, SeqTag)` is the right sizing unit for multi-worker
//!    pipelined rings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// Re-export the SHARED error + reverse name tables from pthreads-sync.
// Same precedent as mp-tcp-bufsync (lib.rs:81): the driver builds
// `NameTables` ONCE and feeds every backend the identical input, which
// is required by the cross-backend differential gate (PRD §8.4 — same
// algorithm + same schedule -> bit-identical output across backends).
//
// Caveat: `EmitError`'s `Display` impl prepends "pthreads-sync:" to
// every message regardless of which crate emitted the error. This is a
// known cross-backend cosmetic lie (mp-tcp-bufsync inherits the same
// issue). The driver dispatch site prepends "pthreads-async codegen
// error:" outside the inner Display, so the final user-visible text
// reads "pthreads-async codegen error: pthreads-sync: <msg>" — confusing
// but not a correctness defect. Filed as TASK-0230 for architectural
// cleanup once the third backend's codegen lands (any cosmetic shift
// would otherwise create churn against the gate test strings).
pub use pthreads_sync::{EmitError, NameTables};

// AlgoIR-free: the only `compiler::*` surface this skeleton uses is the
// inert per-worker EventList carrier (`compiler::event::{Event, WorkerId}`)
// + the NameSidecar (`compiler::sidecar`). When TASK-0226 lands the
// codegen body, it will additionally import the inert expression grammar
// (`compiler::algo::{IrExpr, IrBinOp, ResolvedType, ScalarType}`) the
// EventList already carries — exactly the surface pthreads-sync +
// mp-tcp-bufsync consume.
use compiler::event::{Event, WorkerId};
use compiler::sidecar::NameSidecar;

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

/// Paths to the files [`emit`] wrote. Same shape as
/// `pthreads_sync::EmitResult` because pthreads-async also produces a
/// single Cargo project + single binary (shared-memory transport) —
/// only the per-loop codegen body differs (ring buffer + Condvar
/// instead of sequential barrier). Mirrors the field set so the driver
/// can pattern-match the dispatch arm against either single-binary
/// tier-1 backend identically.
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
/// **SKELETON**: returns [`EmitError::ContractGap`] with a precise
/// forward-link to **TASK-0226** (ring-buffer + Condvar codegen). The
/// wire-shape matches `pthreads_sync::emit` and `mp_tcp_bufsync::emit`
/// — the next implementer fills in the body without changing the
/// signature.
///
/// Wire contract (AC#1, TASK-0124): consumes the per-worker [`Event`]
/// lists + the [`NameTables`] (reverse `name_*`) + the [`NameSidecar`].
/// **No `&ACFG` / `&LinkedIR` access**, exactly like pthreads-sync and
/// mp-tcp-bufsync.
pub fn emit(
    _per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    _names: &NameTables,
    _sidecar: &NameSidecar,
    _kernels_rs_path: &Path,
    _out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // FAIL-LOUD: there is no silent fallback. The capability-compat
    // check already accepted this backend (capabilities.toml is real);
    // the codegen body is not yet implemented and that is HONEST.
    // CLAUDE.md: no workarounds — the next implementer rips this
    // early-return out as the first line of TASK-0226.
    Err(EmitError::ContractGap(
        "pthreads-async backend (skeleton, TASK-0042.01 cycle 16): \
         ring-buffer + Condvar codegen not yet implemented. \
         The capabilities.toml + Cargo wiring + driver dispatch arm \
         are real; the codegen body is the subject of TASK-0226. \
         Use `--backend pthreads-sync` or `--backend mp-tcp-bufsync` \
         for runnable output until then."
            .to_string(),
    ))
}
