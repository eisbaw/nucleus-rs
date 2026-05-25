//! Shared multi-worker event-walker for the pthreads-sync and
//! pthreads-async backends (TASK-0239).
//!
//! # Why this module exists
//!
//! Cycle 26 (TASK-0228 Wave B-2) implemented pthreads-async multi-worker
//! emit by COPYING ~400 LoC of pthreads-sync's walker
//! (`render_worker_events`, `render_wait_assign`, the `WaitSlice`
//! shape-dispatch (pre-TASK-0294 `leading_axis_slice` / `LeadingAxis`),
//! `collect_pre_init_sets`, `collect_xfer_pairs`, `collect_worker_slots`,
//! `collect_barriers_by_tag`), substituting `slot_<id>`
//! for `ring_<id>` at the four Push/Wait callsites. The duplication was
//! mechanically maintainable for one cycle but every subsequent edit
//! to the walker would risk silent drift between two backends whose
//! cross-backend bit-identical differential (PRD §10.1) is the headline
//! thesis-falsifiability claim.
//!
//! TASK-0239 (this module) lifts the walker into a single source of
//! truth parameterised by ONE string: the rendezvous variable prefix
//! (`"slot"` for pthreads-sync, `"ring"` for pthreads-async). Everything
//! else (Fire / Loop / Sync / Wait gather, check_frame instrumentation,
//! per-worker partition range override, per-occurrence strip-mine
//! block_tag rebinding (TASK-0181; the header + abs_subst-construction
//! half is the shared [`render_block_tag_loop_header`] helper that
//! mp-tcp-bufsync ALSO consumes — TASK-0253),
//! barrier identity via `SyncTag`, slice-paste 1D/2D-tile arithmetic
//! (TASK-0117 leading-axis path + TASK-0294 row-loop path for 2D
//! `partition=blocks2d` tiles)) is shared verbatim across both
//! backends — there is no second axis of variation worth a trait
//! abstraction.
//!
//! # What stays per-backend
//!
//! The shared walker handles only the per-worker EventList walk and the
//! Wait gather. Each backend's `Plan::emit` retains:
//!
//! - The substrate struct decl (`struct Slot<T> { Mutex+Condvar }` vs
//!   the bounded `Ring<T>` from `pthreads_async::ring_buffer::emit_ring_
//!   struct_decl`).
//! - The per-pair instance allocation (`Slot::new()` vs `Ring::new(cap)`,
//!   where cap is sidecar-derived).
//! - The `Plan` struct definition itself (the async variant carries
//!   an extra `ring_caps: BTreeMap<(DataId, SeqTag), u64>` for sizing).
//!
//! That keeps the two backends' real semantic difference (one-shot
//! rendezvous vs bounded buffered channel) visible at the `emit()`
//! entry point.
//!
//! # Design choice: direct parameter, not trait
//!
//! Option (B) from the cycle-31 plan: pass `rendezvous_prefix: &str`
//! through a small `WalkerCtx` struct. Option (A) (a `RendezvousDispatch`
//! trait) was rejected because the variation is a single string; the
//! existing `RenderCtxPub` shared-helper precedent in `lib.rs` is also
//! direct-pass. A trait would introduce dispatch ceremony for no second
//! axis.
//!
//! # SlotId == RingId == usize
//!
//! Confirmed by both backends' type alias (`type SlotId = usize` /
//! `pub(crate) type RingId = usize`). The shared map shape is
//! `BTreeMap<(DataId, SeqTag), usize>` and is reused verbatim.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use nucleus_compiler::event::{
    BlockTag, DataId, Event, IterTile, IterVar, SeqTag, SyncTag, ViolationKind, WorkerId,
};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::check_frame::{emit_count_branch, emit_log_branch, sanitize_loop_var};
use crate::render::{
    render_const_expr_pub, render_fire_args_pub, render_fire_output_assign_pub,
    render_reuse_buf_decls_pub, render_reuse_marker_comment, render_reuse_per_iter_update_pub,
    EmitError, RenderCtxPub,
};

/// Stable identifier for one rendezvous channel (slot or ring) keyed
/// by `(DataId, SeqTag)` ordered ascending. Same shape as the
/// per-backend `SlotId` / `RingId` aliases — both are `usize`, so the
/// map is shared.
pub type RendezvousId = usize;

/// Receiver-side gather shape for a Wait event's tile.
///
/// Dispatched in [`render_wait_assign`]:
///
/// - `Flat { lo, hi }` — 1D leading-axis slice-paste
///   (`name[lo..hi].copy_from_slice(&_tmp[lo..hi])`). The TASK-0117
///   path. `lo`/`hi` are pre-multiplied flat-element offsets (i.e.
///   leading-axis index × product of inner dims).
/// - `Rows { outer_lo, outer_hi, row_stride, inner_lo_off,
///   inner_hi_off }` — 2D row-loop slice-paste, one `copy_from_
///   slice` per outer-axis iteration. The TASK-0294 path. Selected
///   when the tile has rank >= 2 AND the data has dim rank >= 2.
///   Each worker under `partition=blocks2d` owns a 2D rectangle of
///   its data; the 1D leading-axis path would paste the worker's
///   whole y-band (overwriting adjacent workers' columns with
///   default-zero values), so a row-loop is required for
///   bit-identical gather. `row_stride` is the per-outer-axis-
///   element flat-element count (= product of `dims[1..]`);
///   `inner_lo_off` / `inner_hi_off` are the per-row flat-element
///   offsets of the inner-axis range (= inner-axis index × product
///   of `dims[2..]`).
///
/// Module-private — only [`render_wait_assign`] destructures the
/// variants, and it lives in this module.
enum WaitSlice {
    Flat {
        lo: usize,
        hi: usize,
    },
    Rows {
        outer_lo: usize,
        outer_hi: usize,
        row_stride: usize,
        inner_lo_off: usize,
        inner_hi_off: usize,
    },
}

/// Walker-time bundle of every fact the per-worker event walk needs.
///
/// Mirrors the per-backend `Plan` field set, but only the references
/// the walker actually reads — no ownership transfer, no copying. The
/// `rendezvous_prefix` field is THE ONE knob that distinguishes the
/// two backends (`"slot"` vs `"ring"`); every other field is shared
/// verbatim.
pub struct WalkerCtx<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// `"slot"` for pthreads-sync, `"ring"` for pthreads-async. Used
    /// in the four emit-string substitutions (`{prefix}_{id}.push(...)`
    /// and `{prefix}_{id}.wait()`).
    pub rendezvous_prefix: &'a str,
    /// Cross-worker Push/Wait pair -> rendezvous index. Both backends
    /// key by `(DataId, SeqTag)` and assign indices ascending.
    pub rendezvous_ids: &'a BTreeMap<(DataId, SeqTag), RendezvousId>,
    /// Per-pair iteration tile from the originating XferPlaceholder
    /// (TASK-0117). Drives the receiver-side leading-axis slice-paste
    /// in `render_wait_assign` (1D leading-axis path TASK-0117 + 2D
    /// row-loop path TASK-0294).
    pub pair_tiles: &'a BTreeMap<(DataId, SeqTag), IterTile>,
}

impl WalkerCtx<'_> {
    /// Render context for the shared expression renderers
    /// (`render_fire_args_pub`, `render_const_expr_pub`, etc.).
    fn render_ctx(&self) -> RenderCtxPub<'_> {
        RenderCtxPub::new(self.names, self.sidecar)
    }

    /// Worker name from the reverse NameTables, falling back to
    /// `w<id>` if the table is missing the entry (defensive — should
    /// never happen for an in-Plan WorkerId).
    fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    /// Data name lookup; fails LOUD via [`EmitError::ContractGap`]
    /// when a DataId in the event stream has no name in the tables.
    fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }
}

/// Emit the strip-mined inner-block loop HEADER and build the
/// per-occurrence absolute-index rebinding context, shared between
/// every backend's multi-worker `Event::Loop` walker (TASK-0253).
///
/// # What this helper owns
///
/// 1. Lookup of `lo_src` from `sidecar.loop_bounds[iter_var]` (single
///    source of truth — the LO is keyed off the (reused) IterVar, not
///    duplicated into the tag).
/// 2. Construction of the rebound absolute-index expression:
///    - full nest (`is_partial == false`): `LO + tile*N + inner`,
///      where `tile` is the immediately-enclosing tile-loop's iter var
///      (its name resolved from `names.iter_var`); missing one is a
///      malformed EventList and returns [`EmitError::ContractGap`].
///    - trailing partial (`is_partial == true`): `LO + num_full*N +
///      inner` (constant base — the partial's own `0..1` tile loop
///      would always contribute 0).
/// 3. Construction of the child [`RenderCtxPub`] carrying the extended
///    `abs_subst` so every body use site (Fire arg, indexed assign,
///    inner-loop bound) sees the rebound expression — NOT just the
///    loop header (the load-bearing TASK-0181 review-gate finding).
/// 4. Emit of the loop header `for {var} in ({start}_i64)..({end}_i64)
///    {{` (concrete folded range; NOT the source-form bound and NOT a
///    partition slice — the strip-mined inner iterates the tile).
///
/// # What the CALLER owns
///
/// - Recursion into the loop body using the returned child context
///   (each backend has its own per-event walker — the walker recurses
///   through `bar_<tag>` / `{prefix}_<id>` Push/Wait, mp-tcp-bufsync
///   recurses through `sock_<peer>` Push/Wait — extracting that axis
///   was deemed not worth the trait abstraction, see the walker's
///   design doc above).
/// - Emit of the closing `}` after the body.
///
/// Cycles 73 + 75: TASK-0181 landed the rebinding logic by COPYING it
/// into both the walker and mp-tcp-bufsync; TASK-0253 (this helper)
/// consolidates that duplication into ONE place ACROSS THE MULTI-WORKER
/// BACKENDS (walker is consumed by pthreads-sync + pthreads-async;
/// mp-tcp-bufsync delegates here directly). A separate sibling copy of
/// the same arithmetic survives on the pthreads-sync SINGLE-worker
/// render path (`pthreads-sync/src/lib.rs` `Event::Loop` arm of
/// `render_events_in`), which uses the backend-private `RenderCtx` (vs
/// this helper's `RenderCtxPub`); unifying those two flavours requires
/// a separate `RenderCtx` ↔ `RenderCtxPub` refactor and is filed
/// downstream of TASK-0253 — do not claim "exactly one place" in the
/// codebase as a whole. The two MULTI-worker callers now produce
/// byte-identical emitted bytes for the strip-mined header + rebound
/// child context BY CONSTRUCTION — no future drift possible between
/// THEM.
///
/// Seven params is over clippy's `too_many_arguments` threshold; the
/// alternative (a `BlockTagHeaderCtx` bundle struct) would be synthetic
/// ceremony for a one-call-site-per-backend stateless emit, and the
/// same local allow is used on the sibling `render_worker_events_inner`.
/// `names` and `sidecar` are sourced from `ctx.names` / `ctx.sidecar`
/// (RenderCtxPub holds the same borrows; passing them separately would
/// duplicate the bundle — review-MAJOR-2 cycle 75). Local allow.
#[allow(clippy::too_many_arguments)]
pub fn render_block_tag_loop_header<'a>(
    out: &mut String,
    indent: usize,
    iter_var: IterVar,
    range: &std::ops::Range<i64>,
    tag: &BlockTag,
    enclosing: Option<IterVar>,
    ctx: &RenderCtxPub<'a>,
) -> Result<RenderCtxPub<'a>, EmitError> {
    let pad = "    ".repeat(indent);
    let var = ctx.names.iter_var.get(&iter_var).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "iter var {iter_var:?} in Event::Loop has no name in NameTables"
        ))
    })?;
    let (abs, _strip_lo_expr) = compute_block_tag_abs_exprs(iter_var, tag, enclosing, ctx)?;
    let mut child_subst = ctx.abs_subst.clone();
    child_subst.insert(var.clone(), abs);
    let child = ctx.with_abs_subst(child_subst);
    // Loop header uses the concrete folded range
    // (`{start}_i64..{end}_i64`) — NOT the source-form bound (would
    // re-introduce the full range) and NOT the partition slice (the
    // strip-mined inner loop iterates over the tile, not the worker's
    // partition slice).
    writeln!(
        out,
        "{pad}for {var} in ({}_i64)..({}_i64) {{",
        range.start, range.end
    )
    .ok();
    Ok(child)
}

/// Compute the strip-mined-loop rebound absolute-index expression `abs`
/// AND its `iv=0` counterpart `strip_lo_expr` (the absolute coordinate
/// of the strip-mined loop's FIRST iteration). Both expressions are
/// built from the same structural components — STRUCTURAL not textual,
/// so a tile-name that overlaps the inner var name (e.g. iv="x" and
/// tile_name="x__tile" from `block_transform`'s canonical
/// `format!("{var}__tile")`) does NOT corrupt the rebound expression
/// (cycle 103 architect P1.1 review-gate finding on the single-worker
/// path; mirrored here for TASK-0270 because the multi-worker walker's
/// strip-mine arm now ALSO needs both expressions for the reuse
/// prologue's `lo` argument).
///
/// Pure computation — no writes to `out`, no side effects on the
/// parent context. The caller is responsible for inserting `abs` into
/// the child `RenderCtxPub::abs_subst`, writing the loop header, and
/// passing `strip_lo_expr` to [`crate::render::render_reuse_buf_decls_pub`]
/// when the iv carries reuse.
///
/// # Returned shape
///
/// - `abs`: the rebound absolute-index expression at body sites
///   (`(LO + tile*N + inner)` for full-nest; `(LO + num_full*N +
///   inner)` for trailing-partial).
/// - `strip_lo_expr`: the `iv=0` counterpart used as the reuse
///   prologue's lo argument (substitutes `inner` with `0_i64` in the
///   `abs` template).
///
/// # Errors
///
/// - [`EmitError::ContractGap`] when a full-nest `BlockTag` (`is_partial
///   == false`) has no `enclosing` tile loop — `block_transform`
///   contracts to always emit the tile, so a missing one is a malformed
///   EventList.
/// - [`EmitError::ContractGap`] when the enclosing tile var has no name
///   in `NameTables`.
/// - [`EmitError::ContractGap`] when `iter_var` has no name in
///   `NameTables`.
pub fn compute_block_tag_abs_exprs(
    iter_var: IterVar,
    tag: &BlockTag,
    enclosing: Option<IterVar>,
    ctx: &RenderCtxPub<'_>,
) -> Result<(String, String), EmitError> {
    let var = ctx.names.iter_var.get(&iter_var).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "iter var {iter_var:?} in Event::Loop has no name in NameTables"
        ))
    })?;
    let lo_src = ctx
        .sidecar
        .loop_bounds
        .get(&iter_var)
        .map(|b| render_const_expr_pub(&b.lo, ctx))
        .transpose()?
        .unwrap_or_else(|| "0_i64".to_string());
    let n = tag.block_n;
    let (abs, strip_lo_expr) = if tag.is_partial {
        // Constant base: the partial tile's own tile loop is `0..1`,
        // so a `tile*N` term is always 0. Build BOTH `abs` and
        // `strip_lo_expr` structurally from the same components — the
        // strip_lo_expr is `abs` evaluated at iv=0 (substitute `var`
        // with `0_i64`), which we construct directly, not via textual
        // `abs.replace(var, "0_i64")` (that defect is filed in
        // memory `feedback-textual-replace-codegen-unsafe`: when `var`
        // is a substring of any other token, the replace corrupts the
        // sibling — cycle 103 architect P1.1).
        (
            format!("({lo_src} + ({}_i64 * {n}_i64) + {var})", tag.num_full),
            format!("({lo_src} + ({}_i64 * {n}_i64) + 0_i64)", tag.num_full),
        )
    } else {
        // Variable base from the enclosing tile loop. A tagged full
        // nest ALWAYS has an enclosing tile loop (block_transform
        // emits `tile -> seq -> inner`); missing one is a malformed
        // EventList — fail loud with context (typed error, not panic).
        let tile_iv = enclosing.ok_or_else(|| {
            EmitError::ContractGap(format!(
                "strip-mined full-tile inner loop {iter_var:?} \
                 (block_tag is_partial=false) has no enclosing tile \
                 loop — block_transform always wraps it; malformed \
                 EventList"
            ))
        })?;
        let tile_name = ctx.names.iter_var.get(&tile_iv).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "tile iter var {tile_iv:?} has no name in NameTables"
            ))
        })?;
        (
            format!("({lo_src} + ({tile_name} * {n}_i64) + {var})"),
            format!("({lo_src} + ({tile_name} * {n}_i64) + 0_i64)"),
        )
    };
    Ok((abs, strip_lo_expr))
}

/// Walk one worker's EventList, emitting Rust statements into `out`.
///
/// This is the SHARED walker — both pthreads-sync's `Plan` and
/// pthreads-async's `Plan` call through it. The substitution surface
/// is exactly `ctx.rendezvous_prefix` (the variable-name prefix on
/// `{prefix}_<id>.push(...)` / `{prefix}_<id>.wait()`).
///
/// # Strip-mine rebinding (TASK-0181)
///
/// A `block_tag.is_some()` `Event::Loop` is per-occurrence absolute-
/// index rebound exactly as the single-worker path does (TASK-0180).
/// The rebinding map is threaded through `RenderCtxPub.abs_subst` so
/// it reaches every Fire arg / index / const-expr render site, not
/// just the loop header (the subtle TASK-0181 review-gate finding:
/// substituting only at the header would leave Fire body uses un-
/// rebound, re-introducing the accumulator double-count TASK-0180
/// closed). No tier-1 multi-worker schedule blocks today, so this
/// path is structurally unreachable from the e2e matrix; the rebinding
/// is exercised by unit tests in `backend-common/tests/`.
///
/// # Per-worker partition override (TASK-0212)
///
/// When the partition pass recorded a per-worker slice for THIS
/// worker on this iter var (`sidecar.partition_worker_ranges`), the
/// loop renders the concrete literal range. Otherwise the source-form
/// symbolic / literal precedence from `sidecar.loop_bounds` applies.
/// The strip-mine path above renders the concrete folded range
/// instead and skips the partition-slice check (the strip-mined inner
/// loop iterates `0..N`, not the partitioned source slice).
///
/// # check_frame defense
///
/// `check_frame.is_some() && block_tag.is_some()` is an
/// `inject_check_frames`-layer invariant violation (frames attach
/// only to outer source loops). The strip-mine path above returns
/// early before reaching this branch, so the defense is structurally
/// unreachable today, but catches a future projection-layer regression.
pub fn render_worker_events(
    ctx: &WalkerCtx<'_>,
    worker: WorkerId,
    events: &[Event],
    out: &mut String,
    indent: usize,
    prefix: &str,
) -> Result<(), EmitError> {
    let render_ctx = ctx.render_ctx();
    render_worker_events_inner(ctx, worker, events, out, indent, prefix, &render_ctx, None)
}

/// `enclosing` is the iter-var of the immediately-enclosing
/// `Event::Loop` (the tile loop, when the child is a strip-mined
/// inner-block loop with `block_tag.is_partial == false`). `None` at
/// top level. Mirrors the single-worker `render_events_in` /
/// `render_event` parameter (TASK-0180).
///
/// Eight params is one over clippy's `too_many_arguments` threshold;
/// the alternative (a single bundle struct) would be a synthetic
/// container with no semantic content — the parameters are the
/// genuine inputs to one stateless event-walk step. Local allow.
#[allow(clippy::too_many_arguments)]
fn render_worker_events_inner(
    ctx: &WalkerCtx<'_>,
    worker: WorkerId,
    events: &[Event],
    out: &mut String,
    indent: usize,
    prefix: &str,
    render_ctx: &RenderCtxPub<'_>,
    enclosing: Option<IterVar>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    let rendezvous_prefix = ctx.rendezvous_prefix;
    for e in events {
        match e {
            Event::Fire {
                kernel, bindings, ..
            } => {
                let callee = ctx.names.kernel.get(kernel).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "kernel id {kernel:?} in a Fire has no name in NameTables"
                    ))
                })?;
                let args = render_fire_args_pub(*kernel, &bindings.inputs, render_ctx)?;
                match &bindings.output {
                    None => {
                        writeln!(out, "{pad}kernels::{callee}({args});").ok();
                    }
                    Some(o) if o.indices.is_empty() => {
                        let name = ctx.data_name(o.data)?;
                        writeln!(out, "{pad}let mut {name} = kernels::{callee}({args});").ok();
                    }
                    Some(o) => {
                        // TASK-0209 shared scalar-vs-sub-array
                        // classifier — both backends route through
                        // `render_fire_output_assign_pub` so the Fire-
                        // output sites cannot drift.
                        let rhs = format!("kernels::{callee}({args})");
                        let stmt = render_fire_output_assign_pub(o, &rhs, render_ctx)?;
                        writeln!(out, "{pad}{stmt}").ok();
                    }
                }
            }
            Event::Loop {
                iter_var,
                range,
                body,
                block_tag,
                check_frame,
            } => {
                let var = ctx.names.iter_var.get(iter_var).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                    ))
                })?;

                // Per-occurrence absolute-index rebinding (TASK-0181;
                // mirrors TASK-0180 on the single-worker path) AND
                // reuse circular-buffer codegen (TASK-0270, mirrors
                // TASK-0269 strip-mine arm on the single-worker path).
                //
                // Pre-TASK-0270 this arm delegated wholesale to
                // [`render_block_tag_loop_header`] (which writes the
                // for-header and returns the rebound child). With the
                // reuse-buffer landing the order matters: the buffer
                // decl + initial-fill prologue MUST live OUTSIDE the
                // for-header so the buffer persists across iterations.
                // We therefore split the helper into the pure
                // [`compute_block_tag_abs_exprs`] (returns abs +
                // strip_lo_expr) + an inline header write, so the
                // walker can interpose `render_reuse_buf_decls_pub`
                // between them.
                //
                // `strip_lo_expr` is the `iv=0` counterpart of `abs`
                // built STRUCTURALLY (NOT via textual
                // `abs.replace(var, "0_i64")` — that defect is filed
                // in memory `feedback-textual-replace-codegen-unsafe`,
                // cycle 103 architect P1.1 on the single-worker path,
                // mirrored here).
                if let Some(tag) = block_tag {
                    let (abs, strip_lo_expr) =
                        compute_block_tag_abs_exprs(*iter_var, tag, enclosing, render_ctx)?;
                    // TASK-0270 strip-mine arm reuse-buffer decl +
                    // prologue. Emitted at the OUTER pad above the
                    // for-header so the buffer persists across the
                    // inner loop's iterations. The prologue's
                    // reuse-axis "lo" uses the rebound ABSOLUTE
                    // expression at iv=0 (`LO + tile*N + 0`) because
                    // the strip-mined loop's lexical range is
                    // `0..inner_len`, not `LO..HI`. NO-OP when the iv
                    // carries no reuse (every shipped non-blocked
                    // schedule).
                    let reuse_groups = render_reuse_buf_decls_pub(
                        out,
                        indent,
                        *iter_var,
                        var,
                        &strip_lo_expr,
                        body,
                        render_ctx,
                    )?;
                    // Build the child context carrying BOTH the
                    // strip-mine abs rebinding AND the reuse-active
                    // groups. Parent's reuse_active is preserved (so
                    // nested reuse loops compose); new groups OVERRIDE
                    // on data_id collision.
                    let mut child_subst = render_ctx.abs_subst.clone();
                    child_subst.insert(var.clone(), abs.clone());
                    let mut child_reuse = render_ctx.reuse_active.clone();
                    for (data_id, gs) in reuse_groups.clone() {
                        child_reuse.insert(data_id, gs);
                    }
                    let child = render_ctx
                        .with_abs_subst_and_reuse_active(child_subst, child_reuse);
                    // Header line: concrete folded range
                    // (`{start}_i64..{end}_i64`) — NOT the source-form
                    // bound (would re-introduce the full range) and
                    // NOT the partition slice (the strip-mined inner
                    // loop iterates over the tile, not the worker's
                    // partition slice). Inlined here because the buf
                    // decls above had to land BEFORE the for-header;
                    // the regular `render_block_tag_loop_header`
                    // wrapper is unchanged and continues to be used by
                    // mp-tcp-bufsync's strip-mine arm (which does not
                    // emit reuse codegen on this cycle).
                    writeln!(
                        out,
                        "{pad}for {var} in ({}_i64)..({}_i64) {{",
                        range.start, range.end
                    )
                    .ok();
                    // TASK-0265 Tier 1 marker (preserved): a
                    // strip-mined inner loop CAN carry reuse (cf.
                    // 05-stencil/distributed `loop x : block=64,
                    // vectorize=8, reuse;`). Emit the marker at body
                    // entry of the inner-block loop, same as the
                    // non-strip-mine path below. The marker substring
                    // `reuse_widths_pending` is preserved as a
                    // regression canary above the buffer decls (which
                    // landed above the for-header) — the
                    // `__reuse_buf_<data>_a<axis>_g<group_idx>` +
                    // `rem_euclid(L_i64)` strings are the second-layer
                    // codegen canary (TASK-0282: `_g<group_idx>` is
                    // uniform; single-group cases carry `_g0`).
                    render_reuse_marker_comment(
                        out,
                        indent + 1,
                        *iter_var,
                        var,
                        ctx.sidecar,
                        ctx.names,
                    );
                    // TASK-0270 per-iter update: load the most-distant
                    // element into the buffer slot before any Fire arg
                    // reads it. Iv expression here is the rebound
                    // ABSOLUTE expression (so the source-array index
                    // reflects the strip-mined coordinate), NOT the
                    // bare `var`.
                    render_reuse_per_iter_update_pub(
                        out,
                        indent + 1,
                        &reuse_groups,
                        &abs,
                        &child,
                    )?;
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        indent + 1,
                        prefix,
                        &child,
                        Some(*iter_var),
                    )?;
                    writeln!(out, "{pad}}}").ok();
                    continue;
                }

                // Defense-in-depth invariant
                // (`inject_check_frames` is contracted to populate
                // check_frame only on outer source loops; block_tag ==
                // None). The strip-mine path above returns early, so
                // reaching here with both set is a projection-layer
                // bug.
                if check_frame.is_some() && block_tag.is_some() {
                    return Err(EmitError::ContractGap(format!(
                        "Event::Loop for iter var `{var}` carries BOTH a check_frame \
                         and a block_tag — `inject_check_frames` is contracted to \
                         populate check_frame only on outer source loops; this is a \
                         projection-layer bug (TASK-0052.05 multi-worker invariant)."
                    )));
                }
                // Per-worker partition override (TASK-0212): if the
                // partition pass recorded a slice for THIS worker on
                // this iter var, render the concrete literal range.
                // The symbolic `loop_bounds` entry names the SOURCE
                // range, not the partitioned slice. A worker not
                // listed in the per-iter-var map (e.g. host, which
                // doesn't participate in partition=workers) falls
                // through to the source-form symbolic / literal
                // precedence exactly as before TASK-0212.
                let partition_slice = ctx
                    .sidecar
                    .partition_worker_ranges
                    .get(iter_var)
                    .and_then(|m| m.get(&worker));
                let (lo, hi) = match partition_slice {
                    Some(r) => (format!("{}_i64", r.start), format!("{}_i64", r.end)),
                    None => match ctx.sidecar.loop_bounds.get(iter_var) {
                        Some(b) => (
                            render_const_expr_pub(&b.lo, render_ctx)?,
                            render_const_expr_pub(&b.hi, render_ctx)?,
                        ),
                        None => (format!("{}_i64", range.start), format!("{}_i64", range.end)),
                    },
                };
                // TASK-0270 regular (non-strip-mine) arm: emit reuse-
                // buffer decl + initial-fill prologue at OUTER pad
                // BEFORE the for-header (the buffer must persist
                // across iterations). The prologue's reuse-axis "lo"
                // is the PER-WORKER lo computed above — when
                // `partition_worker_ranges` recorded a slice, the
                // prologue uses that worker's first-iteration absolute
                // coordinate, which is the correct source-array index
                // for the prologue fill. NO-OP when the iv carries no
                // reuse (every shipped multi-worker schedule pre-
                // TASK-0270).
                let reuse_groups = render_reuse_buf_decls_pub(
                    out,
                    indent,
                    *iter_var,
                    var,
                    &lo,
                    body,
                    render_ctx,
                )?;
                writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                let body_indent = indent + 1;
                let body_pad = "    ".repeat(body_indent);
                // TASK-0265 Tier 1 marker (preserved): regular (non-
                // strip-mined) loop — marker comment at body entry.
                // The substring `reuse_widths_pending` is the first-
                // layer canary (grep-able for AC#4 of TASK-0265). The
                // `__reuse_buf_<data>_a<axis>_g<group_idx>` +
                // `rem_euclid(L_i64)` strings the buf_decls above emit
                // are the second-layer codegen canary (TASK-0282:
                // `_g<group_idx>` is uniform; single-group cases carry
                // `_g0`). NO-OP when the iv carries no reuse.
                render_reuse_marker_comment(
                    out,
                    body_indent,
                    *iter_var,
                    var,
                    ctx.sidecar,
                    ctx.names,
                );
                // Build the body's child RenderCtxPub carrying the
                // newly-active reuse groups. Parent's abs_subst is
                // preserved (non-strip-mine arm does NOT introduce a
                // rebinding — only the strip-mine arm above does);
                // new reuse groups OVERRIDE on data_id collision (a
                // hypothetical inner-loop reuse on the SAME data; not
                // exercised by 05-stencil/distributed but BTreeMap
                // semantics are well-defined). Per-iter-update +
                // recursion both consume this child.
                let mut child_reuse = render_ctx.reuse_active.clone();
                for (data_id, gs) in reuse_groups.clone() {
                    child_reuse.insert(data_id, gs);
                }
                let body_ctx = render_ctx.with_reuse_active(child_reuse);
                // TASK-0270 per-iter update: load the most-distant
                // element into the buffer slot before any Fire arg
                // reads it. Iv expression here is the bare var (no
                // abs_subst rebinding on this non-strip-mine path).
                render_reuse_per_iter_update_pub(
                    out,
                    body_indent,
                    &reuse_groups,
                    var,
                    &body_ctx,
                )?;
                if let Some(frame) = check_frame {
                    // TASK-0221 defensive — `var` (NameTables) and
                    // `frame.loop_var` (CheckFrame) must name the same
                    // user-source loop variable. Dev-only assert
                    // catches future projection divergence.
                    debug_assert_eq!(
                        var.as_str(),
                        frame.loop_var.as_str(),
                        "CheckFrame.loop_var diverged from NameTables.iter_var \
                         (projection-layer bug; TASK-0221)"
                    );
                    writeln!(
                        out,
                        "{body_pad}let _check_start = std::time::Instant::now();"
                    )
                    .ok();
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        body_indent,
                        prefix,
                        &body_ctx,
                        Some(*iter_var),
                    )?;
                    writeln!(
                        out,
                        "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                    )
                    .ok();
                    match frame.on_violation {
                        ViolationKind::Panic => {
                            writeln!(
                                out,
                                "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                                ns = frame.latency_max_ns,
                                lv = frame.loop_var,
                            )
                            .ok();
                        }
                        ViolationKind::Log => {
                            // TASK-0222: shared template — see emit_log_branch.
                            emit_log_branch(out, &body_pad, &frame.loop_var, frame.latency_max_ns);
                        }
                        ViolationKind::Count => {
                            // TASK-0222: shared template — see emit_count_branch.
                            let id = sanitize_loop_var(&frame.loop_var);
                            emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
                        }
                    }
                } else {
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        indent + 1,
                        prefix,
                        &body_ctx,
                        Some(*iter_var),
                    )?;
                }
                writeln!(out, "{pad}}}").ok();
            }
            Event::Sync { sync, .. } => {
                // Barrier identity is the contract-carried SyncTag
                // (TASK-0172): every participant of this barrier
                // carries the same tag, so all participants .wait()
                // on the same `bar_<tag>` with no pre-order-index
                // recovery.
                let bid = sync.0;
                writeln!(out, "{pad}{prefix}bar_{bid}.wait();").ok();
            }
            Event::Push { data, dst, seq, .. } => {
                let rid = ctx.rendezvous_ids.get(&(*data, *seq)).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "Push of data {data:?} (seq {seq:?}) has no rendezvous id \
                         (not collected as cross-worker)"
                    ))
                })?;
                let name = ctx.data_name(*data)?;
                let to = ctx.worker_name(*dst);
                writeln!(
                    out,
                    "{pad}{prefix}{rendezvous_prefix}_{rid}.push({name}.clone()); // send `{name}` to {to}",
                )
                .ok();
            }
            Event::Wait { data, src, seq, .. } => {
                let rid = ctx.rendezvous_ids.get(&(*data, *seq)).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "Wait of data {data:?} (seq {seq:?}) has no rendezvous id \
                         (not collected as cross-worker)"
                    ))
                })?;
                let name = ctx.data_name(*data)?;
                let from = ctx.worker_name(*src);
                // TASK-0117 host-side gather: see render_wait_assign.
                let assign = render_wait_assign(
                    ctx.sidecar,
                    ctx.pair_tiles,
                    &name,
                    *data,
                    *seq,
                    &format!("{prefix}{rendezvous_prefix}_{rid}.wait()"),
                )?;
                writeln!(out, "{pad}{assign} // recv `{name}` from {from}",).ok();
            }
            Event::Alloc { .. } | Event::Free { .. } => {
                // RAII Vec storage; no explicit reservation.
            }
        }
    }
    Ok(())
}

/// Render the receiver-side assignment statement for one Wait event.
/// Returns one statement (no trailing newline).
///
/// Three shapes, dispatched by [`wait_slice`]:
/// - **Whole-array assign** (`name = <rhs>;`) — the pre-TASK-0117
///   single-pair behaviour. Selected when the pair's tile is empty
///   (no enclosing iteration nest, e.g. a top-level load_input ⇒ host
///   transfer), OR when every consulted axis of the tile covers the
///   data's full range on the corresponding dim (i.e. the producer
///   sent the whole array on this pair).
/// - **1D slice-paste** (`{ let _tmp = <rhs>; name[lo..hi]
///   .copy_from_slice(&_tmp[lo..hi]); }`) — TASK-0117 leading-axis
///   gather. Selected when the tile has a single bound (or only one
///   bound is consultable against the data's dim rank).
/// - **2D row-loop slice-paste** (`{ let _tmp = <rhs>; for _y in
///   outer_lo..outer_hi { let _r = _y * row_stride; name[_r +
///   inner_lo_off.._r + inner_hi_off].copy_from_slice(&_tmp[_r +
///   inner_lo_off.._r + inner_hi_off]); } }`) — TASK-0294
///   `partition=blocks2d` gather. Selected when the tile has rank >=
///   2 AND the data has dim rank >= 2; each outer-axis iteration
///   copies one row's inner-axis sub-range. The 1D leading-axis
///   path would paste each worker's whole y-band (overwriting
///   adjacent workers' columns with default-zero values).
pub fn render_wait_assign(
    sidecar: &NameSidecar,
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    name: &str,
    data: DataId,
    seq: SeqTag,
    rhs: &str,
) -> Result<String, EmitError> {
    let slice = match pair_tiles.get(&(data, seq)) {
        Some(tile) => wait_slice(sidecar, data, tile)?,
        None => None,
    };
    match slice {
        None => {
            // Empty tile (or whole-array match) — whole-array assign.
            Ok(format!("{name} = {rhs};"))
        }
        Some(WaitSlice::Flat { lo, hi }) => {
            // 1D leading-axis slice-paste — TASK-0117.
            Ok(format!(
                "{{ let _tmp = {rhs}; \
                 {name}[{lo}usize..{hi}usize].copy_from_slice(\
                 &_tmp[{lo}usize..{hi}usize]); }}"
            ))
        }
        Some(WaitSlice::Rows {
            outer_lo,
            outer_hi,
            row_stride,
            inner_lo_off,
            inner_hi_off,
        }) => {
            // 2D row-loop slice-paste — TASK-0294. The `_y`/`_r`
            // local names are underscore-prefixed AND introduced
            // inside a `{ ... }` block — Rust block-shadowing makes
            // them safe regardless of where this Wait is placed
            // (host main() body, worker pre-compute halo-strip
            // landing site, or a future multi-pass time-step Repeat
            // body). The cycle-115 placement happens to keep
            // halo-strip Waits at the root Sequence (TASK-0290), so
            // collision was structurally impossible; this argument
            // stays sound when that placement moves under TASK-0294
            // multi-pass follow-ups.
            Ok(format!(
                "{{ let _tmp = {rhs}; \
                 for _y in {outer_lo}usize..{outer_hi}usize {{ \
                 let _r = _y * {row_stride}usize; \
                 {name}[_r + {inner_lo_off}usize.._r + {inner_hi_off}usize]\
                 .copy_from_slice(\
                 &_tmp[_r + {inner_lo_off}usize.._r + {inner_hi_off}usize]); \
                 }} }}"
            ))
        }
    }
}

/// Compute the receiver-side gather shape for a Wait's tile.
///
/// Returns:
/// - `Ok(None)` when the tile is empty OR every consulted axis
///   covers the corresponding dim's full source range — the
///   whole-array path.
/// - `Ok(Some(WaitSlice::Flat { ... }))` for the 1D leading-axis
///   slice-paste (TASK-0117).
/// - `Ok(Some(WaitSlice::Rows { ... }))` for the 2D row-loop
///   slice-paste (TASK-0294), fired iff the tile has rank >= 2 AND
///   the data has dim rank >= 2.
/// - `Err` on a shape mismatch — a tile axis range exceeding the
///   corresponding dim length, an empty range, or a negative start.
///   These are compiler-pass invariant violations worth failing
///   loud rather than silently emitting an out-of-bounds slice.
///
/// Module-private — both backends consume this only indirectly via
/// `render_wait_assign`.
///
/// # AXIS-MAPPING ASSUMPTION (discharged TASK-0302; consult upstream guarantee)
///
/// Assumes `tile.bounds[i].iter_var` maps to data dim `i` (the
/// row-major / nest-order convention). The convention is now
/// upstream-enforced by
/// `transfer_inject::compute_partition_bounds_with_dim_prefix`
/// (TASK-0302, cycle 121): it consults the per-data, per-dim iv
/// indexing map and emits bounds in *data-dim* order, dropping any
/// data symbol whose partition-covered dims do not form a
/// contiguous prefix from dim 0 to whole-array (empty bounds). This
/// generalises TASK-0301's per-symbol iv-membership filter to the
/// per-dim shape — necessary for the 07-matmul `b[k][j]` ×
/// `partition=blocks2d(i,j)` case where j is in b's union but only
/// at dim 1 (not a prefix); pre-TASK-0302 the per-symbol filter
/// would have emitted `[(j, j_band)]` for b and silently mis-sliced
/// b's k dim.
///
/// Lineage:
///   - TASK-0117 cycle 1: HONEST-PARTIAL ASSUMPTION (1D leading axis;
///     `_iv` never consulted — only the numerical range validated).
///   - TASK-0294: generalised to the second axis (`tile.bounds[1]
///     .iter_var ↔ ty.dims[1]`).
///   - TASK-0301: 1D per-symbol-union filter (07-matmul/distributed
///     × `partition=workers(i)`).
///   - TASK-0302: per-dim contiguous-prefix filter (07-matmul/
///     distributed-2d × `partition=blocks2d(i, j)`). Upstream-enforced
///     for every shipped partition shape: partition-derived bounds
///     (`compute_partition_bounds_with_dim_prefix`) AND halo-strip
///     bounds (`inject_halo_strip_xfers`, written as `[(outer_iv,
///     ...), (inner_iv, ...)]` assuming the data is `[outer][inner]`)
///     emit in data-dim order on every cell currently in the e2e
///     matrix. The assumption is no longer a silent risk for any
///     shipped schedule.
///
/// Open shapes (not currently in the e2e matrix):
///   - A halo-bearing data symbol indexed `[k][j]` while the
///     partition pair is `(outer=i, inner=j)`. `inject_halo_strip_xfers`
///     would write `[(i, ...), (j, ...)]` and `wait_slice` would
///     slice dim 0 (=k) by `i_band`. Same axis-mapping concern
///     resurfaces; the halo-strip site does not yet consult
///     `data_dim_iv_map`.
///   - An inner-axis-leading partition (e.g. `partition=blocks2d(j, i)`
///     where the OUTER iv lands at data dim 1 instead of 0) or a
///     non-row-major data layout — the dim-prefix logic assumes
///     dim 0 comes first.
fn wait_slice(
    sidecar: &NameSidecar,
    data: DataId,
    tile: &IterTile,
) -> Result<Option<WaitSlice>, EmitError> {
    // Empty tile -> no per-axis slicing.
    let Some((_iv, leading_range)) = tile.bounds.first() else {
        return Ok(None);
    };
    let ty = sidecar.data_type(data).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "Wait of data {data:?} has no ResolvedType in NameSidecar"
        ))
    })?;
    // Scalar data: no slice axes — whole-value transfer.
    if ty.dims.is_empty() {
        return Ok(None);
    }
    let leading_dim = ty.dims[0] as i64;
    if leading_range.start < 0
        || leading_range.end > leading_dim
        || leading_range.start >= leading_range.end
    {
        return Err(EmitError::ContractGap(format!(
            "Wait of data {data:?}: tile leading-axis range {:?} out of \
             bounds for data dims {:?} (leading-dim {})",
            leading_range, ty.dims, leading_dim
        )));
    }
    let leading_full = leading_range.start == 0 && leading_range.end == leading_dim;

    // 2D row-loop path (TASK-0294): fires iff tile has 2+ axes AND
    // the data has 2+ dims. The inner axis is `tile.bounds[1]`,
    // assumed to map to `ty.dims[1]` — same axis-ordering convention
    // the 1D path applies to `tile.bounds[0]` ↔ ty.dims[0].
    //
    // Rank-3+ guard (TASK-0294 cycle-115 architect P2.1): a tile or
    // data shape with rank >= 3 would slip silently into the 2D arm,
    // consulting only the first two axes — the SAME HONEST-PARTIAL
    // class the cycle-115 fix removed for 2-axis data. No shipped
    // schedule constructs such a (tile, data) shape today (13-cnn-
    // inference has rank-4 data but only rank-1 tiles via
    // partition=workers, which hits the 1D arm below). Fail LOUD so
    // a future schedule that does construct one is flagged at
    // compile time rather than emitting an out-of-bounds gather.
    if tile.bounds.len() > 2 || (tile.bounds.len() >= 2 && ty.dims.len() > 2) {
        return Err(EmitError::ContractGap(format!(
            "Wait of data {data:?}: tile rank {} and data dim rank {} \
             exceed the 2D row-loop slice-paste's supported shape (rank \
             <= 2 on both). No shipped schedule constructs this today; \
             see TASK-0294 cycle-115 architect P2.1 — extend `wait_slice` \
             to N-D nested-loop dispatch or file a follow-up before \
             shipping a schedule that does",
            tile.bounds.len(),
            ty.dims.len(),
        )));
    }
    if tile.bounds.len() >= 2 && ty.dims.len() >= 2 {
        let inner_range = &tile.bounds[1].1;
        let inner_dim = ty.dims[1] as i64;
        if inner_range.start < 0
            || inner_range.end > inner_dim
            || inner_range.start >= inner_range.end
        {
            return Err(EmitError::ContractGap(format!(
                "Wait of data {data:?}: tile inner-axis range {:?} out of \
                 bounds for data dims {:?} (inner-dim {})",
                inner_range, ty.dims, inner_dim
            )));
        }
        let inner_full = inner_range.start == 0 && inner_range.end == inner_dim;
        // Degenerate: both axes cover their full source. Whole-array
        // assign for emit identity with pre-TASK-0294 single-pair.
        if leading_full && inner_full {
            return Ok(None);
        }
        let inner_stride: usize = ty.dims[2..].iter().product();
        let row_stride: usize = ty.dims[1..].iter().product();
        return Ok(Some(WaitSlice::Rows {
            outer_lo: leading_range.start as usize,
            outer_hi: leading_range.end as usize,
            row_stride,
            inner_lo_off: (inner_range.start as usize).saturating_mul(inner_stride),
            inner_hi_off: (inner_range.end as usize).saturating_mul(inner_stride),
        }));
    }

    // 1D leading-axis path (TASK-0117). Degenerate full-range tile
    // → whole-array assign for pre-TASK-0117 single-pair identity.
    if leading_full {
        return Ok(None);
    }
    let stride: usize = ty.dims[1..].iter().product();
    Ok(Some(WaitSlice::Flat {
        lo: (leading_range.start as usize).saturating_mul(stride),
        hi: (leading_range.end as usize).saturating_mul(stride),
    }))
}

// --------------------------------------------------------------------
// Event-walk helpers (recurse into Event::Loop bodies)
// --------------------------------------------------------------------

/// Collect every `(DataId, SeqTag)` pair appearing on a Push or Wait
/// event in `events` (descending into Loop bodies). The map's value is
/// the pair's tile, copied from the first event sighting; the same
/// `seq` is carried on both endpoints by the XferPlaceholder
/// construction (TASK-0018) so first-sighting is well-defined.
pub fn collect_xfer_pairs(events: &[Event], out: &mut BTreeMap<(DataId, SeqTag), IterTile>) {
    for e in events {
        match e {
            Event::Push {
                data, seq, tile, ..
            }
            | Event::Wait {
                data, seq, tile, ..
            } => {
                out.entry((*data, *seq)).or_insert_with(|| tile.clone());
            }
            Event::Loop { body, .. } => collect_xfer_pairs(body, out),
            _ => {}
        }
    }
}

/// Build a `(DataId, SeqTag) -> IterTile` map by folding
/// [`collect_xfer_pairs`] across every worker's projected events.
///
/// Single source of truth for the construction shape that all four
/// tier-1 backends (pthreads-sync, pthreads-async, mp-tcp-bufsync,
/// mp-tcp-event) had been duplicating inline (TASK-0300, cycle 130
/// hardening from TASK-0296 cycle-116 architect P1.2).
///
/// First-sighting on a given `(DataId, SeqTag)` wins; later sightings
/// are dropped. Both endpoints carry the same `IterTile` by the
/// XferPlaceholder construction (TASK-0018), so under valid input the
/// dropped sightings agree with the kept one and the choice is
/// observationally a no-op.
///
/// Determinism of "first" under hypothetical drift (the cycle-130 pin
/// test `first_sighting_wins_on_conflicting_tiles`): callers pass
/// `per_worker.values()` where `per_worker: BTreeMap<WorkerId,
/// Vec<Event>>`. `BTreeMap::values()` iterates in key-ascending order,
/// so "first sighting" = the lowest-`WorkerId` worker whose event list
/// names that `(DataId, SeqTag)`. The helper's output is keyed only on
/// `(DataId, SeqTag)`, so worker iteration order cannot leak into the
/// output's KEY ordering — only into which tile wins on a conflict.
pub fn collect_pair_tiles<'a, I, T>(events_per_worker: I) -> BTreeMap<(DataId, SeqTag), IterTile>
where
    I: IntoIterator<Item = &'a T>,
    T: AsRef<[Event]> + 'a + ?Sized,
{
    let mut out: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    for evs in events_per_worker {
        collect_xfer_pairs(evs.as_ref(), &mut out);
    }
    out
}

/// Per-worker visit of Push/Wait events to collect the worker's
/// rendezvous-id touch set. Descends into `Event::Loop` bodies.
///
/// Replaces the per-backend `collect_worker_slots` /
/// `collect_worker_rings` — both walked identically, only the value
/// type alias differed (`SlotId = RingId = usize`).
pub fn collect_worker_rendezvous(
    events: &[Event],
    ids: &BTreeMap<(DataId, SeqTag), RendezvousId>,
    out: &mut BTreeSet<RendezvousId>,
) {
    for e in events {
        match e {
            Event::Push { data, seq, .. } | Event::Wait { data, seq, .. } => {
                if let Some(s) = ids.get(&(*data, *seq)) {
                    out.insert(*s);
                }
            }
            Event::Loop { body, .. } => collect_worker_rendezvous(body, ids, out),
            _ => {}
        }
    }
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier, so there is
/// nothing to validate / reject here any more).
pub fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants, sync, ..
            } => f(*sync, participants),
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}

/// Visit every `Event::Wait` / `Event::Fire` output to build the
/// three sets needed for the pre-init computation:
///
/// - `waited`: cross-worker inputs the worker WAITs on (these will
///   be overwritten by the .wait() and need to exist as locals).
/// - `whole`: data the worker writes via a whole-array Fire output
///   (let-bound at the Fire site; no pre-init needed).
/// - `indexed`: data the worker writes via an indexed Fire output
///   (must be pre-initialised so the indexed assign has something to
///   write into).
///
/// A worker's pre-init set is `waited UNION (indexed - whole)`.
pub fn collect_pre_init_sets(
    events: &[Event],
    waited: &mut BTreeSet<DataId>,
    whole: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                waited.insert(*data);
            }
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => collect_pre_init_sets(body, waited, whole, indexed),
            _ => {}
        }
    }
}
