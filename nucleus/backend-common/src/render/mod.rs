//! Shared expression / index / kernel-call / loop-bound / type
//! renderers (TASK-0244; moved here from `pthreads-sync` where they
//! first lived as `pub(crate)` shims + `pub` wrappers, accumulating
//! via TASK-0124 / TASK-0156 / TASK-0169 / TASK-0180 / TASK-0209 /
//! TASK-0222).
//!
//! # Single source of truth
//!
//! Every tier-1 backend (pthreads-sync straight-line + multi-worker,
//! pthreads-async single + multi-worker, mp-tcp-bufsync host +
//! worker) routes its expression / index / call / bound / type
//! rendering through the functions here. The cross-backend
//! bit-identical differential (PRD §10.1) holds because there is
//! exactly ONE implementation — no second copy can drift.
//!
//! # Private vs Pub
//!
//! - The full-context [`RenderCtx`] carries the `abs_subst` map used
//!   by strip-mined absolute-index rebinding (TASK-0180). Pthreads-
//!   sync's single-worker renderer (`render_main_rs` in
//!   `pthreads-sync/lib.rs`) constructs it directly to drive per-
//!   occurrence rebinding from `Event::Loop.block_tag`.
//!
//! - The thin [`RenderCtxPub`] is for multi-worker / cross-backend
//!   callers (pthreads-sync multi-worker, pthreads-async multi-worker,
//!   mp-tcp-bufsync host + worker). It carries the SAME `abs_subst`
//!   map as `RenderCtx` so per-occurrence strip-mine rebinding works
//!   on the shared multi-worker walker too (TASK-0181). The map is
//!   empty for every non-blocked program — which is every tier-1
//!   multi-worker schedule today — so non-blocked codegen is byte-
//!   identical to the pre-TASK-0181 emission. The `_pub` variants
//!   stay thin pass-throughs.
//!
//! # Sub-module map
//!
//! - [`error`] — [`EmitError`], the codegen-time error type.
//! - [`ctx`]   — [`RenderCtx`] + [`RenderCtxPub`] context structs.
//! - [`fire`]  — Name resolution, Fire output/argument rendering,
//!   DataSlice classification (scalar vs sub-array), flat-index
//!   rendering, and the `write_file` filesystem helper.
//! - [`expr`]  — Integer / const expression and loop-bound renderers
//!   (`render_int_expr`, `render_loop_bounds`, `render_const_expr`).
//! - [`types`] — Rust type / zero-literal / init-expression renderers
//!   (`rust_scalar_type`, `rust_type_of`, `render_array_init_for`).
//! - [`reuse`] — Reuse-widths marker emit + circular-buffer codegen
//!   (TASK-0265 / TASK-0269 / TASK-0270 / TASK-0282).
//!
//! All public symbols are re-exported below so `backend_common::
//! render::{...}` import paths stay stable for every external caller.

pub mod ctx;
pub mod error;
pub mod expr;
pub mod fire;
pub mod reuse;
pub mod types;

pub use ctx::{RenderCtx, RenderCtxPub};
pub use error::EmitError;
pub use expr::{render_const_expr, render_const_expr_pub, render_int_expr, render_loop_bounds};
pub use fire::{
    data_name, render_fire_args, render_fire_args_pub, render_fire_output_assign,
    render_fire_output_assign_pub, render_flat_index, render_flat_index_pub, write_file,
};
pub use reuse::{
    render_reuse_buf_decls, render_reuse_buf_decls_pub, render_reuse_marker_comment,
    render_reuse_per_iter_update, render_reuse_per_iter_update_pub, ReuseRewriteGroup,
};
pub use types::{
    render_array_init_for, rust_scalar_type, rust_scalar_type_pub, rust_scalar_zero, rust_type_of,
};
