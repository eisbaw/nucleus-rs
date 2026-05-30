//! Integer / const expression and loop-bound renderers. Split from
//! `render.rs` for file-size hygiene; no behaviour change.
//!
//! `render_int_expr` consults `ctx.abs_subst` (strip-mined absolute-
//! index rebinding, TASK-0180) and `ctx.sidecar.consts` (declared
//! `const` resolution); `render_const_expr` does the same for loop
//! bounds with `_i64` literal spelling. `bin_op_str` is the small
//! shared `IrBinOp -> &str` mapper both renderers share.

use nucleus_compiler::algo::{IrBinOp, IrExpr};
use nucleus_compiler::event::IterVar;

use super::ctx::{RenderCtx, RenderCtxPub};
use super::error::EmitError;

/// Render an integer-valued index/scalar expression as Rust.
///
/// Identifier resolution priority (highest first):
///   1. `ctx.abs_subst` — an active strip-mined absolute-index
///      rebinding (`inner_var` → `(LO + tile*N + inner)`). The map
///      is empty for every non-blocked program.
///   2. `ctx.sidecar.consts` — a declared `const` in the source
///      algorithm (e.g. `const N : usize = 32;` referenced as `N`
///      inside an `IndexExpr` such as `grid[t][(i+N-1) % N]`).
///      Grammar §1 line 91-93 explicitly allows consts inside an
///      IndexExpr; PRD §6.2.1 forbids iter-var shadowing a const, so
///      the two namespaces never collide.
///   3. Bare ident — an iteration variable in scope. Emitted as the
///      verbatim Rust identifier; Rust's type-checker is responsible
///      for confirming it is in scope (mirrors the prior backend).
///
/// The const-resolution path (step 2) is the fix for the IndexExpr-
/// const bug discovered while landing example 11-game-of-life
/// (cycle 35 / TASK-0042.04). Prior to that, an IndexExpr like
/// `(t + ITERS) % (ITERS + 1)` rendered as `((t + ITERS) % (ITERS +
/// 1))` — emitting bare `ITERS` as a Rust identifier, which is not
/// in scope in the generated host source (the codegen does not
/// declare a Rust `const ITERS` mirroring the Nuc const). Loop
/// BOUNDS already resolve consts via `render_const_expr` (step 2 in
/// that function), so `for t : 0 .. ITERS+1` worked; the asymmetry
/// was the bug. Examples 01..09/13 only used loop variables in
/// `IndexExpr`, so the bug was inert in the existing matrix.
pub fn render_int_expr(e: &IrExpr, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}")),
        IrExpr::Ident(n) => {
            if let Some(repl) = ctx.abs_subst.get(n) {
                Ok(repl.clone())
            } else if let Some(c) = ctx.sidecar.consts.get(n) {
                Ok(format!("{}", c.value))
            } else {
                Ok(n.clone())
            }
        }
        IrExpr::Neg(inner) => Ok(format!("-({})", render_int_expr(inner, ctx)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_int_expr(l, ctx)?;
            let rs = render_int_expr(r, ctx)?;
            Ok(format!("({ls} {} {rs})", bin_op_str(op)))
        }
        // TASK-0341.03.01: a DATA-DEPENDENT (gather) index — `x[col[k]]`,
        // where the index of `x` is itself a runtime data read `col[k]`.
        // The inner ref `col[k]` lowers to this `IrExpr::DataRef`; we
        // render it as a flat row-major load `col[(<flat>) as usize]`
        // (an `i32`), which the surrounding subscript then casts to
        // `usize` in turn. The inner read MUST be FULL-RANK (every axis
        // of the index array indexed to a single scalar slot) — a
        // partial-rank gather index would be a sub-array, not an integer
        // index value, so it is rejected fail-loud. Kernel calls in index
        // position remain unsupported.
        IrExpr::DataRef(iref) => render_gather_index_load(iref, ctx),
        IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "kernel call inside an integer index expression".to_string(),
        )),
    }
}

/// Render a data-dependent (gather) index load `col[(<flat>) as usize]`
/// for an inner [`IrExpr::DataRef`] appearing in index position
/// (TASK-0341.03.01). The result is an `i32`-valued Rust expression
/// (the loaded index value); the caller wraps it in the outer
/// subscript and its own `as usize` cast.
///
/// Requires a FULL-RANK index (every dim of the inner array indexed to
/// a scalar slot): a partial-rank inner ref is a sub-array, not a
/// single integer index, and is rejected with [`EmitError`] rather
/// than silently emitting a wrong (sub-array-start) offset.
fn render_gather_index_load(
    iref: &nucleus_compiler::algo::IndexedRef,
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    if iref.indices.is_empty() {
        return Err(EmitError::UnsupportedFeature(format!(
            "whole-array reference `{}` used as an integer index (a gather \
             index must be a fully-indexed scalar load like `col[k]`)",
            iref.name
        )));
    }
    // Resolve the inner array's DataId by inverting NameTables.data
    // (DataId -> name); data symbols are few, a linear scan is fine.
    let did = ctx
        .names
        .data
        .iter()
        .find(|(_, n)| n.as_str() == iref.name)
        .map(|(d, _)| *d)
        .ok_or_else(|| {
            EmitError::ContractGap(format!(
                "gather index array `{}` has no DataId in NameTables",
                iref.name
            ))
        })?;
    let ty = ctx.sidecar.data_type(did).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "gather index array `{}` ({did:?}) has no ResolvedType in the NameSidecar",
            iref.name
        ))
    })?;
    if iref.indices.len() != ty.dims.len() {
        return Err(EmitError::UnsupportedFeature(format!(
            "gather index array `{}` indexed with {} expression(s) but has rank {} \
             (a gather index must be a FULL-RANK scalar load; sidecar dims={:?})",
            iref.name,
            iref.indices.len(),
            ty.dims.len(),
            ty.dims
        )));
    }
    // Reuse the shared row-major flattener (it renders each inner index
    // via `render_int_expr`, so a nested gather `x[a[b[k]]]` terminates
    // by structural recursion). Yields `(<flat>) as usize`.
    let slice = nucleus_compiler::event::DataSlice {
        data: did,
        indices: iref.indices.clone(),
    };
    let flat = super::fire::render_flat_index(&slice, ctx)?;
    Ok(format!("{}[{flat}]", iref.name))
}

/// The `(lo, hi)` source strings for an `Event::Loop`.
///
/// If the iter var has a `sidecar.loop_bounds` entry it is a SOURCE
/// `for` loop — render the *unevaluated* bound through
/// `render_const_expr` + `sidecar.consts` so `for y : 1 .. H-1`
/// (H=16) becomes `1_i64 .. (16_i64 - 1_i64)`, byte-identical to the
/// old AlgoIR-walking backend.
///
/// If there is NO `loop_bounds` entry the loop is a
/// `block_transform`-synthesised tile loop with no source form — use
/// the concrete `Event::Loop.range`, rendered `{n}_i64` (matching the
/// old backend, which rendered the ACFG `Repeat.range` literals for
/// synthesised loops the same way).
pub fn render_loop_bounds(
    iter_var: IterVar,
    range: &std::ops::Range<i64>,
    ctx: &RenderCtx<'_>,
) -> Result<(String, String), EmitError> {
    match ctx.sidecar.loop_bounds.get(&iter_var) {
        Some(b) => {
            let lo = render_const_expr(&b.lo, ctx)?;
            let hi = render_const_expr(&b.hi, ctx)?;
            Ok((lo, hi))
        }
        None => {
            // Synthesised tile loop: concrete folded range, same
            // spelling as an integer literal const bound.
            Ok((format!("{}_i64", range.start), format!("{}_i64", range.end)))
        }
    }
}

/// Render a *constant* loop-bound expression, resolving const idents
/// to their value via the SIDECAR's const table (NOT `algo.consts`,
/// AC#2). Mirrors the old `render_const_expr` spelling exactly:
/// `IntLit(v)` → `{v}_i64`, a const ident → `{value}_i64`, an
/// outer-loop iter var → bare name, `BinOp` → `({l} op {r})`.
pub fn render_const_expr(e: &IrExpr, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}_i64")),
        IrExpr::Ident(n) => {
            if let Some(c) = ctx.sidecar.consts.get(n) {
                Ok(format!("{}_i64", c.value))
            } else if let Some(repl) = ctx.abs_subst.get(n) {
                // A rebound strip-mined outer iter var referenced in
                // an inner loop bound: use its absolute expression.
                // (Empty for every non-blocked program -> the old
                // bare-name behaviour, byte-identical.)
                Ok(repl.clone())
            } else {
                // An outer loop's iter var: render as-is, rely on
                // Rust to type-check (mirrors the old backend).
                Ok(n.clone())
            }
        }
        IrExpr::Neg(inner) => Ok(format!("-({})", render_const_expr(inner, ctx)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_const_expr(l, ctx)?;
            let rs = render_const_expr(r, ctx)?;
            Ok(format!("({ls} {} {rs})", bin_op_str(op)))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside a const expression (loop bound)".to_string(),
        )),
    }
}

fn bin_op_str(op: &IrBinOp) -> &'static str {
    match op {
        IrBinOp::Add => "+",
        IrBinOp::Sub => "-",
        IrBinOp::Mul => "*",
        IrBinOp::Div => "/",
        IrBinOp::Mod => "%",
    }
}

/// `_pub` shim consumed by multi-worker callers. Downcalls to
/// [`render_const_expr`] via `ctx.inner()`.
pub fn render_const_expr_pub(e: &IrExpr, ctx: &RenderCtxPub<'_>) -> Result<String, EmitError> {
    render_const_expr(e, &ctx.inner())
}
