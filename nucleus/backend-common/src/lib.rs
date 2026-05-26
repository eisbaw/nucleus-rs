//! Shared codegen primitives consumed by every tier-1 backend
//! (pthreads-sync, pthreads-async, mp-tcp-bufsync). TASK-0244.
//!
//! # Why this crate exists
//!
//! Through cycles 17/22/24/31 the shared codegen surface grew inside
//! `pthreads-sync` — it was historically the first backend, so the
//! shared code lived there and the other backends imported it via
//! `pthreads_sync::*` paths. This made pthreads-async and
//! mp-tcp-bufsync look like dependents of pthreads-sync, which is
//! NOT semantically true: all three are siblings, each emits a
//! different runtime substrate (Slot vs Ring vs TCP), and none is the
//! parent of the others. The arrow was leaking the implementation
//! detail "pthreads-sync was extracted first" as if it were structure.
//!
//! TASK-0244 (cycle 37) moves the shared code into THIS crate. Every
//! backend now depends on `backend-common`; none depends on another
//! backend except where the dependency is genuinely semantic —
//! pthreads-async's single-worker arm DELEGATES to
//! `pthreads_sync::render_single_worker_main` to guarantee byte-
//! identical single-worker emit, and that delegation is the one
//! remaining inter-backend arrow.
//!
//! # Module map
//!
//! - [`check_frame`] — `check loop V : on_violation={panic,log,count}`
//!   emit templates and the `CountCheckLoop` materialised view.
//! - [`render`] — `RenderCtx` + the expression / index / kernel-call /
//!   loop-bound / type renderers shared across single- and multi-
//!   worker emit on every backend. Also houses [`EmitError`] (the
//!   codegen-time error type re-exported by every backend's public
//!   surface).
//! - [`multi_worker_walker`] — the per-worker `Event` walker shared
//!   between pthreads-sync's `multi_worker::Plan::emit`, pthreads-
//!   async's `multi_worker::Plan::emit`, and (future) any backend
//!   whose multi-worker substrate is rendezvous-channel-shaped.
//! - [`project_skeleton`] — Cargo.toml + run.sh string templates for
//!   the single-binary tier-1 backends (pthreads-sync + pthreads-async).
//!   TASK-0246 (cycle 38): finishes the cycle-37 extraction by lifting
//!   the last two non-semantic shared strings out of pthreads-sync.
//!   mp-tcp-bufsync's multi-process variant stays in mp-tcp-bufsync
//!   (different signature, different shape).
//! - [`host_election`] — the canonical host-election rule shared by
//!   every tier-1 backend's `multi_worker::Plan::build` AND by the
//!   compiler-level passes in `nucleus-driver` that mediate against
//!   the backend-elected host (cycles 160 / 162 / 163). TASK-0336
//!   cycle 164 lift; retires the
//!   `feedback-driver-must-mirror-backend-election-exactly`
//!   recurrence surface on the canonical path.

pub mod check_frame;
pub mod host_election;
pub mod multi_worker_walker;
pub mod project_skeleton;
pub mod render;

// Convenience top-level re-exports. Backends typically import the
// specific items they need from `backend_common::{check_frame, render,
// multi_worker_walker}`, but the most-frequently used surface is also
// reachable from the crate root for shorter call sites.
pub use check_frame::{
    collect_count_check_frames, emit_count_branch, emit_count_guard_local,
    emit_count_reporter_struct, emit_count_static, emit_log_branch, sanitize_loop_var,
    CountCheckLoop,
};
pub use host_election::{elect_host_from_name_workers, elect_host_from_worker_names, HOST_NAME};
pub use project_skeleton::single_binary::{render_cargo_toml, render_run_sh};
pub use render::{
    data_name, render_array_init_for, render_const_expr, render_const_expr_pub, render_fire_args,
    render_fire_args_pub, render_fire_output_assign, render_fire_output_assign_pub,
    render_flat_index, render_flat_index_pub, render_int_expr, render_loop_bounds,
    rust_scalar_type, rust_scalar_type_pub, rust_scalar_zero, rust_type_of, write_file, EmitError,
    RenderCtx, RenderCtxPub,
};
