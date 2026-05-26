//! Strip-mined inner-block loop header + per-occurrence absolute-index
//! rebinding helpers (TASK-0253 + TASK-0181). Consumed by both
//! [`super::event_walker::render_worker_events_inner`] (the shared
//! pthreads-sync / pthreads-async / mp-tcp-event walker) AND by
//! mp-tcp-bufsync's strip-mine arm directly via the public surface
//! re-exported in [`super`]'s mod.rs.

use std::fmt::Write as _;

use nucleus_compiler::event::{BlockTag, IterVar};

use crate::render::{render_const_expr_pub, EmitError, RenderCtxPub};

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
