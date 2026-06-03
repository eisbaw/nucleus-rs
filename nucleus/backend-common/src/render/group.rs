//! Reuse-rewrite group descriptor + the leaf affine/canonicalisation
//! helpers it is built from. Split out of `reuse.rs` (TASK-0340.02) to
//! break the two sibling-module dependency cycles `ctx <-> reuse` and
//! `fire <-> reuse`: `ctx` needs [`ReuseRewriteGroup`] and `fire` needs
//! [`sidecar_consts_to_resolved`] + [`try_reuse_axis_offset`], so when
//! those lived in `reuse.rs` (which itself imports `ctx::RenderCtx` and
//! `fire::data_name` / `fire::render_flat_index`) both directions of
//! the import graph closed into mutual cycles.
//!
//! This module is a TRUE LEAF: it depends only on `nucleus_compiler`
//! IR / sidecar types + `std::BTreeMap`. It references nothing else in
//! `render/` (no `RenderCtx`, no `fire`, no `error`, no `types`), so it
//! sorts strictly below every other `render/` submodule in the
//! topological order. No behaviour change — the moved code is verbatim.

use std::collections::BTreeMap;

use nucleus_compiler::affine_decompose;
use nucleus_compiler::algo::{IrBinOp, IrExpr, ResolvedConst};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::NameSidecar;

/// One reuse rewrite group: the circular buffer that backs every
/// matching DataRef read of a single `(DataId, axis)` slot whose
/// OUTER axes match the canonical `outer_axes` pattern verbatim.
///
/// Populated by `render_reuse_buf_decls` (in the `reuse` sibling
/// module) when a body's matching DataRefs are discovered; consumed by
/// `render_fire_args` (in the `fire` sibling module) via
/// `try_rewrite_reuse_arg` at every Fire-arg read site to rewrite a
/// matching DataRef into a
/// `buf[(iv + b - min_offset).rem_euclid(L) as usize]` lookup.
///
/// TASK-0282 generalisation: there is ONE group per UNIQUE
/// `(data_id, axis, outer_axes_tuple)`. In 05-stencil/reuse the
/// `blur3(img_in[y-1][...], img_in[y][...], img_in[y+1][...])` call
/// produces three groups on `(img_in, axis=1)` — one each for outer
/// axes `[y-1]`, `[y]`, `[y+1]` — disambiguated by
/// [`Self::group_idx`] in source-discovery order. All 9 of the
/// `img_in` reads in the for-x body rewrite to the matching buffer.
#[derive(Debug, Clone)]
pub struct ReuseRewriteGroup {
    /// The reuse-axis index inside the DataRef (`axis` key from the
    /// sidecar's `reuse_widths` triple-nested map).
    pub axis: u64,
    /// Disambiguates groups that share `(data_id, axis)` but differ
    /// on [`Self::outer_axes`]. Assigned in source-discovery order
    /// during the body walk (TASK-0282). With one group per
    /// `(data_id, axis)` (the pre-TASK-0282 narrowing) every group
    /// carries `group_idx = 0`; with N unique outer-axes tuples on a
    /// given `(data_id, axis)` the groups carry `0..N` in walk order.
    pub group_idx: u64,
    /// The slot shape inferred by Stage 1 (`length`, `min_offset`).
    pub slot: ReuseSlot,
    /// The canonical OUTER-axis index expressions (all axes except
    /// `axis`) — a matching DataRef must have these EXACT outer-axis
    /// expressions to be rewritten. Stored in source-order, so axis
    /// indexing is preserved (axis 0 first, then axis 1, etc., with
    /// the reuse axis at position `axis` omitted).
    pub outer_axes: Vec<IrExpr>,
    /// Emitted Rust identifier for the per-`(data, axis, group_idx)`
    /// buffer. Form: `__reuse_buf_<data_name>_a<axis>_g<group_idx>`.
    /// The `_g<group_idx>` suffix is uniform — single-group cases
    /// carry `_g0` rather than a bare `_a<axis>` (deterministic across
    /// the narrow first-cut shape and the multi-outer-coord shape).
    pub buf_ident: String,
    /// The iter-var name (e.g. `"x"`) — used by `render_fire_arg` to
    /// detect the affine `iv + b` shape on the reuse-axis index.
    pub iv_name: String,
}

/// Convert the sidecar's `ConstValue`
/// table to the [`ResolvedConst`] table shape
/// `affine_decompose` consumes.
/// They carry the same `(ty, value)` data; `ResolvedConst` additionally
/// names itself. Trivial O(n) materialise on a small per-program table.
///
/// `pub(super)` so `try_rewrite_reuse_arg` (in the `fire` sibling
/// module) can call it at the Fire-arg rewrite boundary.
pub(super) fn sidecar_consts_to_resolved(sidecar: &NameSidecar) -> BTreeMap<String, ResolvedConst> {
    sidecar
        .consts
        .iter()
        .map(|(name, cv)| {
            (
                name.clone(),
                ResolvedConst {
                    name: name.clone(),
                    ty: cv.ty.clone(),
                    value: cv.value,
                },
            )
        })
        .collect()
}

/// Try to decompose an affine `iv + b` index for the reuse-axis
/// rewrite. Returns `Some(b)` iff
/// [`nucleus_compiler::affine_decompose`] accepts the expression with
/// coefficient 1 (the only coefficient Stage 1
/// [`nucleus_compiler::apply_reuse_inference`] records — any other has
/// already been rejected at inference time).
///
/// Pure-integer expressions (no iv mention) return `None` — they're
/// out-of-pattern for a reuse-axis index (no rewrite needed).
///
/// TASK-0283 cycle 105: the cycle-103 first landing inlined a tiny
/// re-impl of the iv+const shapes because `affine_decompose` was
/// `pub(crate)`. That created a cross-pass divergence risk: if the
/// affine grammar widened in Stage 1 (e.g. constant Mod folding for
/// example 11 game-of-life), the codegen re-impl here would silently
/// reject reads that Stage 1 accepted — marker fires, rewrite skips,
/// silent codegen mismatch. Lifting both stages onto the same function
/// makes the divergence structurally impossible.
///
/// `consts` is the algorithm's const table (passed pre-converted from
/// `NameSidecar::consts` via [`sidecar_consts_to_resolved`] at the
/// per-discover-or-render boundary); it lets the affine decomposition
/// fold const-named offsets like `iv + OFFSET` when `const OFFSET = 1`
/// was declared.
///
/// `pub(super)` so `try_rewrite_reuse_arg` (in the `fire` sibling
/// module) can call it at the Fire-arg rewrite boundary.
pub(super) fn try_reuse_axis_offset(
    e: &IrExpr,
    iv_name: &str,
    consts: &BTreeMap<String, ResolvedConst>,
) -> Option<i64> {
    let (coeff, offset) = affine_decompose(e, iv_name, consts)?;
    if coeff == 1 {
        Some(offset)
    } else {
        None
    }
}

/// Canonicalise a single outer-axis index expression by folding
/// additive and multiplicative identity-element subtrees (`e + 0`,
/// `0 + e`, `e - 0`, `e * 1`, `1 * e` → `e`). Used by
/// `walk_arg_for_reuse` (in the `reuse` sibling module) to normalise
/// the dedupe key on [`ReuseRewriteGroup::outer_axes`] (TASK-0286 P2.1
/// hardening of TASK-0282).
///
/// `pub(super)` so the `reuse` sibling module's discovery walker can
/// call it across the (TASK-0340.02) sibling-module boundary.
///
/// Risk model: `IrExpr` `PartialEq` is structural equality on the AST.
/// Two semantically-equal outer-axes that arrive at the dedupe site in
/// different ASTs (e.g. `y` from one upstream pass, `Add(y, IntLit(0))`
/// from another) would compare unequal and produce two redundant
/// circular buffers. The AC#4 `<= 3` verbatim-read grep on the for-x
/// body does NOT detect this — it bounds verbatim reads, not buffer
/// count. Today every shipped fixture emits already-canonical outer
/// axes, so this is pre-emptive defence; the unit test pins the
/// invariant so a future upstream non-canonical emit fails this
/// regression instead of silently bloating the emitted code.
///
/// Bottom-up walk: child subtrees are canonicalised first, then the
/// parent node's identity-element fold is applied to the rewritten
/// children. Idempotent (applying twice yields the same expression).
/// Out of scope: constant folding of pure-int subtrees (`1 + 1 → 2`),
/// associativity reassociation (`(y + 1) + 1 → y + 2`), or
/// `Add(e, IntLit(neg)) → Sub(e, IntLit(-neg))`. Those would require a
/// full canonicalisation pass and are deferred until a real shipped
/// fixture triggers a divergence.
pub(super) fn canonicalise_outer_axis(e: &IrExpr) -> IrExpr {
    match e {
        IrExpr::IntLit(_) | IrExpr::Ident(_) => e.clone(),
        IrExpr::Neg(inner) => IrExpr::Neg(Box::new(canonicalise_outer_axis(inner))),
        IrExpr::BinOp(op, l, r) => {
            let lc = canonicalise_outer_axis(l);
            let rc = canonicalise_outer_axis(r);
            match (op, &lc, &rc) {
                (IrBinOp::Add, _, IrExpr::IntLit(0)) => lc,
                (IrBinOp::Add, IrExpr::IntLit(0), _) => rc,
                (IrBinOp::Sub, _, IrExpr::IntLit(0)) => lc,
                (IrBinOp::Mul, _, IrExpr::IntLit(1)) => lc,
                (IrBinOp::Mul, IrExpr::IntLit(1), _) => rc,
                _ => IrExpr::BinOp(*op, Box::new(lc), Box::new(rc)),
            }
        }
        // `IrExpr::DataRef` / `IrExpr::Call` are documented as legal
        // only at the top level of a kernel arg (see ir.rs:180-186),
        // not nested inside another DataSlice index expression. They
        // therefore should not appear inside an outer-axis subtree.
        // Pass through unchanged (rebuilding rather than cloning would
        // pull in the `IndexedRef` import for zero observable gain on
        // any legal input).
        IrExpr::DataRef(_) | IrExpr::Call { .. } => e.clone(),
        // A relational comparison is bool-valued and cannot appear in an
        // outer-axis index subtree (lowering rejects a comparison in index
        // position). Pass through unchanged — same posture as the
        // DataRef / Call arm above (TASK-0341.02.01.02 / S2).
        IrExpr::Compare(..) => e.clone(),
    }
}
