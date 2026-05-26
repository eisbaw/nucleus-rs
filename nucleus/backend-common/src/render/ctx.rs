//! `RenderCtx` + `RenderCtxPub` — the rendering context structs that
//! carry the `(NameTables, NameSidecar, abs_subst, reuse_active)`
//! quadruple every renderer consumes. Split from `render.rs` for
//! file-size hygiene; no behaviour change.
//!
//! The full `RenderCtx` is constructed directly by single-worker
//! callers (pthreads-sync's `render_main_rs`) because they walk
//! `Event::Loop.block_tag` and populate `abs_subst` per-occurrence.
//! Multi-worker / cross-backend callers consume the thin
//! `RenderCtxPub` whose `_pub` shim renderers downcall to the private
//! `RenderCtx` impls via `inner()`.

use std::collections::BTreeMap;

use nucleus_compiler::event::DataId;
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use super::reuse::ReuseRewriteGroup;

/// The full rendering context. Carries the `abs_subst` map used by
/// strip-mined absolute-index rebinding (TASK-0180). Single-worker
/// callers (pthreads-sync's `render_main_rs`) construct this directly
/// because they walk `Event::Loop.block_tag` and populate the map
/// per-occurrence.
///
/// The fields are `pub` so backend code outside this crate can
/// construct an instance directly — the rebinding logic lives in the
/// backend's per-event walker, not here.
pub struct RenderCtx<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// Active absolute-index substitutions: an inner-block loop
    /// variable name -> the `(LO + tile*N + inner)` Rust expression
    /// it must expand to at every *body* use site. Empty for every
    /// non-blocked program, so non-blocked codegen is byte-identical
    /// to the pre-TASK-0124 backend (the map is consulted only by
    /// `render_int_expr`/`render_const_expr` on an `Ident`).
    pub abs_subst: BTreeMap<String, String>,
    /// Active reuse-buffer rewrite groups, keyed by `DataId`.
    ///
    /// Populated by [`super::render_reuse_buf_decls`] at the entry of an
    /// `Event::Loop` body when `sidecar.reuse_widths.get(iter_var)` is
    /// non-empty AND a matching DataRef shape was discovered in the
    /// body (TASK-0269 Stage 2 codegen). Empty for every loop without
    /// reuse-active slots — preserves byte-identicality on every
    /// pre-TASK-0269 schedule (the map is consulted only by
    /// `render_fire_arg` on an `ArgBinding::Data` with non-empty
    /// indices).
    ///
    /// Multi-axis reuse on the same DataId (a separable filter at
    /// different (data, axis) pairs) is supported in shape but the
    /// 05-stencil first landing exercises one (data, axis=1) group.
    pub reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
}

impl<'a> RenderCtx<'a> {
    /// Construct a fresh context with an empty `abs_subst` map. The
    /// caller fills `abs_subst` per-occurrence at strip-mine inner
    /// loops.
    pub fn new(names: &'a NameTables, sidecar: &'a NameSidecar) -> Self {
        RenderCtx {
            names,
            sidecar,
            abs_subst: BTreeMap::new(),
            reuse_active: BTreeMap::new(),
        }
    }
}

/// Thin context for multi-worker / cross-backend callers. Carries
/// the SAME `abs_subst` map as the private [`RenderCtx`] so the
/// per-occurrence absolute-index rebinding (TASK-0180) reaches the
/// `_pub` render helpers too. Pre-TASK-0181 this struct held only
/// `(names, sidecar)` because the multi-worker walker hard-rejected
/// any `Event::Loop.block_tag.is_some()` (the TASK-0181 fail-loud
/// guard) — so the map was guaranteed empty. TASK-0181 replaces that
/// guard with the actual rebinding logic on the shared
/// [`multi_worker_walker`](crate::multi_worker_walker), which means
/// the `_pub` helpers MUST consult `abs_subst` or the substitution
/// would silently stop at the loop header and never reach Fire arg /
/// const-expr / output-assign sites (the exact accumulator
/// double-count failure mode TASK-0180 closed for the single-worker
/// path).
///
/// Default-constructed empty via [`Self::new`]; the shared
/// `render_block_tag_loop_header` helper
/// ([`crate::multi_worker_walker::render_block_tag_loop_header`])
/// extends a child copy per strip-mined inner-loop occurrence via
/// [`Self::with_abs_subst`], and every multi-worker walker
/// (pthreads-sync, pthreads-async, mp-tcp-bufsync) consumes the
/// returned child (TASK-0253 — was duplicated per-backend pre-TASK-
/// 0253 / cycle 73).
pub struct RenderCtxPub<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// See [`RenderCtx::abs_subst`]. Empty for every non-blocked
    /// multi-worker program (which is every tier-1 schedule today, so
    /// non-blocked codegen renders byte-identically to the pre-TASK-
    /// 0181 emission; cross-backend differential pinned in the e2e
    /// matrix).
    pub abs_subst: BTreeMap<String, String>,
    /// See [`RenderCtx::reuse_active`]. Populated by the shared
    /// `multi_worker_walker::render_worker_events_inner` at both
    /// Event::Loop arms (cycle-104 TASK-0270 landing): each loop's
    /// pre-header sets the map from `render_reuse_buf_decls_pub`'s
    /// returned groups, the body recursion inherits it via
    /// [`Self::with_abs_subst_and_reuse_active`] /
    /// [`Self::with_reuse_active`]. Empty for any iv without a
    /// matching `sidecar.reuse_widths` entry (every non-reuse
    /// schedule emits byte-identically to pre-cycle-103). The field
    /// carries the same shape as `RenderCtx::reuse_active` so the
    /// cross-context `inner()` conversion is a literal copy.
    ///
    /// **mp-tcp-bufsync** has its own per-event walker (it does NOT
    /// route through the shared `multi_worker_walker`). TASK-0284
    /// (cycle 107) lifted the reuse-codegen call sites onto that
    /// walker too — its `RenderCtxPub::reuse_active` is populated
    /// from `render_reuse_buf_decls_pub` at body entry, identical in
    /// shape to the shared-walker path.
    pub reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
}

impl<'a> RenderCtxPub<'a> {
    /// Fresh context, empty `abs_subst`. Existing call sites that
    /// pre-date TASK-0181 keep working — they were already passing an
    /// implicit empty map.
    pub fn new(names: &'a NameTables, sidecar: &'a NameSidecar) -> Self {
        RenderCtxPub {
            names,
            sidecar,
            abs_subst: BTreeMap::new(),
            reuse_active: BTreeMap::new(),
        }
    }

    /// Build a child context sharing `(names, sidecar)` with this one
    /// but carrying the supplied `abs_subst`. Used by the shared
    /// multi-worker walker to introduce a per-occurrence strip-mine
    /// rebinding inside one `Event::Loop` body without mutating the
    /// parent context (mirrors the `RenderCtx { abs_subst: child, .. }`
    /// pattern in the single-worker path).
    pub fn with_abs_subst(&self, abs_subst: BTreeMap<String, String>) -> RenderCtxPub<'a> {
        RenderCtxPub {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst,
            reuse_active: self.reuse_active.clone(),
        }
    }

    /// Build a child context sharing `(names, sidecar)` with this one
    /// but carrying the supplied `reuse_active` map. Used by the shared
    /// multi-worker walker to seed a per-occurrence reuse-buffer rewrite
    /// group into the body recursion (TASK-0270 multi-worker codegen
    /// landing). Parent's `abs_subst` is preserved verbatim — the
    /// regular (non-strip-mine) loop arm does not introduce an
    /// abs_subst rebinding (only the strip-mine arm does, and it uses
    /// [`Self::with_abs_subst_and_reuse_active`] which carries both at
    /// once).
    pub fn with_reuse_active(
        &self,
        reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
    ) -> RenderCtxPub<'a> {
        RenderCtxPub {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst: self.abs_subst.clone(),
            reuse_active,
        }
    }

    /// Build a child context carrying BOTH a new `abs_subst` map AND a
    /// new `reuse_active` map. Used by the shared multi-worker walker's
    /// strip-mine arm where one `Event::Loop` simultaneously introduces
    /// the per-occurrence absolute-index rebinding (TASK-0181) AND a
    /// reuse-buffer rewrite group (TASK-0270 multi-worker codegen).
    /// Chaining `with_abs_subst().with_reuse_active()` would also work
    /// but doubles the BTreeMap clone — this builder does both in one
    /// pass.
    pub fn with_abs_subst_and_reuse_active(
        &self,
        abs_subst: BTreeMap<String, String>,
        reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
    ) -> RenderCtxPub<'a> {
        RenderCtxPub {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst,
            reuse_active,
        }
    }

    /// Internal lowering to the private `RenderCtx` the underlying
    /// helpers consume. Clones the `abs_subst` map (cheap — the map
    /// holds at most one entry per active strip-mine nesting depth,
    /// which is bounded by source loop nesting).
    pub(super) fn inner(&self) -> RenderCtx<'_> {
        RenderCtx {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst: self.abs_subst.clone(),
            reuse_active: self.reuse_active.clone(),
        }
    }
}
