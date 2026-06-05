//! Reuse-widths marker emit (TASK-0265 Tier 1 Stage 2 wiring) +
//! circular-buffer codegen (TASK-0269 cycle 103 + TASK-0270 cycle
//! 104). Consumed by ALL FOUR tier-1 backends as of TASK-0284
//! (cycle 107):
//! - pthreads-sync single-worker path (private [`RenderCtx`] via
//!   direct calls in `backends/pthreads-sync/src/lib.rs`);
//! - the shared `multi_worker_walker::render_worker_events_inner`
//!   ([`RenderCtxPub`] via the `_pub` shims at the bottom of this
//!   file) — consumed by pthreads-sync multi-worker, pthreads-async,
//!   and mp-tcp-event;
//! - mp-tcp-bufsync's own `Plan::render_events` walker (cycle 107
//!   lift, verified by `backends/mp-tcp-bufsync/tests/
//!   reuse_codegen_emit.rs`).
//!
//! Split from `render.rs` for file-size hygiene; no behaviour change.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{IrExpr, ResolvedConst};
use nucleus_compiler::event::{ArgBinding, DataId, DataSlice, Event, IterVar};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use super::ctx::{RenderCtx, RenderCtxPub};
use super::error::EmitError;
use super::fire::{data_name, render_flat_index};
use super::group::{
    canonicalise_outer_axis, sidecar_consts_to_resolved, try_reuse_axis_offset, ReuseRewriteGroup,
};
use super::types::{rust_scalar_type, rust_scalar_zero};

// --------------------------------------------------------------------
// Reuse-widths marker emit — TASK-0265 Tier 1 Stage 2 wiring
// --------------------------------------------------------------------

/// Emit a Rust comment line at an `Event::Loop` body's entry naming
/// every (data, axis, ReuseSlot) the sidecar carries for this iv —
/// the FIRST consumer of `NameSidecar::reuse_widths` (Stage 1 ⇒
/// Stage 2 handoff, TASK-0265).
///
/// # Stage 2 status (Tier 1 wiring; cycle 87)
///
/// This is the LOOKUP scaffolding step. The walker reads
/// `sidecar.reuse_widths.get(iter_var)`, iterates `(DataId, axis,
/// ReuseSlot)` triples in deterministic order (`BTreeMap` keys are
/// `u64`-newtype / `u64`), and writes ONE comment line per slot
/// naming `data=<symbol> axis=<n> length=<L> min_offset=<M>`. The
/// `reuse_widths_pending` marker substring is grep-able by the e2e
/// test crate to assert the consumer ran (AC#4 of TASK-0265 — the
/// "emitted code contains 'circular' / 'delay_line' / similar
/// marker" half).
///
/// Tier 2 / Tier 3 of TASK-0265 (per-backend circular-buffer emit)
/// is forward-carried — the actual delay-line `Vec<T>` declaration,
/// initial-fill prologue, per-iteration rotate, and `grid[iv + b]`
/// → `buf[(iv + b - min_offset) % length]` index rewrite live on
/// each backend's `Plan::emit` and are filed as TASK-0269 (pthreads-sync,
/// .01) + TASK-0270 (multi_worker_walker, .02 — covers pthreads-async +
/// mp-tcp-bufsync + mp-tcp-event). Driver promotion strict /
/// partition-policy-aware: TASK-0271 (.04). IvScopeError unification
/// with halo_inference: TASK-0272 (.05).
///
/// # Determinism
///
/// `BTreeMap` iteration on every level. The comment lines emit in
/// (DataId, axis) order — identical inputs produce identical outputs.
/// Empty-input path is a true no-op: when `reuse_widths.get(iter_var)`
/// is `None` (no reuse on this iv) NOTHING is written, preserving
/// byte-identicality with the pre-TASK-0265 emit for every shipped
/// schedule that does not carry `reuse`.
///
/// # Determinism in name lookup
///
/// Data symbol name comes from the `NameTables` reverse map keyed by
/// `DataId`. A missing entry falls back to `d<id>` (defensive — the
/// invariant is that `name_data` covers every DataId in the
/// sidecar, and an absent name in a non-empty reuse map would be a
/// projection-layer bug; we emit the id-form so the marker still
/// fires and downstream tests can see it without an emit hard-fail).
pub fn render_reuse_marker_comment(
    out: &mut String,
    indent: usize,
    iter_var: IterVar,
    iter_var_name: &str,
    sidecar: &NameSidecar,
    names: &NameTables,
) {
    use std::fmt::Write as _;
    let Some(per_data) = sidecar.reuse_widths.get(&iter_var) else {
        return;
    };
    if per_data.is_empty() {
        return;
    }
    let pad = "    ".repeat(indent);
    for (data_id, per_axis) in per_data {
        let data_name = names
            .data
            .get(data_id)
            .cloned()
            .unwrap_or_else(|| format!("d{}", data_id.0));
        for (axis, slot) in per_axis {
            // Marker substring `reuse_widths_pending` is load-bearing
            // for AC#4 of TASK-0265 — the e2e marker-detection test
            // greps for it. Do NOT rename without updating the test.
            //
            // TASK-0269 (cycle 103) + TASK-0270 (cycle 104) + TASK-0284
            // (cycle 107) + TASK-0282 (cycle 110): the marker comment
            // now precedes the real circular-buffer codegen on ALL FOUR
            // tier-1 backends:
            //   - pthreads-sync single-worker path (private `RenderCtx`
            //     via direct calls in `backends/pthreads-sync/src/lib.rs`).
            //   - The shared `multi_worker_walker::render_worker_events_inner`
            //     (consumed by pthreads-sync multi-worker, pthreads-async,
            //     and mp-tcp-event via the `_pub` shims).
            //   - mp-tcp-bufsync's own `Plan::render_events` (TASK-0284
            //     cycle 107 lifted the reuse codegen calls onto its
            //     per-event walker too — `_g0` buffer naming verified by
            //     `backends/mp-tcp-bufsync/tests/reuse_codegen_emit.rs`).
            // The marker substring is preserved as a regression canary
            // above the buffer decl — the
            // `__reuse_buf_<data>_a<axis>_g<group_idx>` +
            // `rem_euclid(L_i64)` strings below are the second-layer
            // codegen canary (TASK-0282 cycle 110 made `_g<group_idx>`
            // uniform — single-group cases carry `_g0`; multi-outer-coord
            // shapes like 05-stencil/reuse carry `_g0/_g1/_g2`), pinned
            // per-arm by the tests in
            // `nucleus/backend-common/tests/multi_worker_reuse_marker.rs`
            // (regular + strip-mine) and the per-backend reuse tests.
            let _ = writeln!(
                out,
                "{pad}// reuse_widths_pending: iv={iter_var_name} data={data_name} axis={axis} length={length} min_offset={min_offset} (Stage 2 active; circular-buffer codegen below — multi-outer-coord rewrite landed cycles 103/104/107/110 on all 4 tier-1 backends)",
                length = slot.length,
                min_offset = slot.min_offset,
            );
        }
    }
}

// --------------------------------------------------------------------
// Reuse circular-buffer codegen — TASK-0269 (cycle 103) +
// TASK-0270 (cycle 104) + TASK-0284 (cycle 107, mp-tcp-bufsync lift).
// Consumed by ALL FOUR tier-1 backends: the pthreads-sync single-
// worker path (private RenderCtx via direct calls), the shared
// `multi_worker_walker::render_worker_events_inner` (RenderCtxPub
// via the `_pub` shims below; pthreads-sync MW + pthreads-async +
// mp-tcp-event), and mp-tcp-bufsync's own `Plan::render_events`
// walker (cycle 107). Module-level docstring at the top of this file
// carries the full consumer map.
// --------------------------------------------------------------------

// `ReuseRewriteGroup`, `sidecar_consts_to_resolved`,
// `try_reuse_axis_offset`, and `canonicalise_outer_axis` MOVED to the
// leaf module `group.rs` (TASK-0340.02) to break the `ctx <-> reuse`
// and `fire <-> reuse` sibling-module import cycles. They are
// re-imported below (`use super::group::{...}`) so the discovery
// walkers here keep calling them unchanged.

/// Walk an `Event` tree looking for EVERY unique
/// `(data_id, axis, outer_axes_tuple)` matching a reuse slot.
/// "Matching" means: the `ArgBinding::Data`'s `DataSlice` has enough
/// axes, the reuse-axis index decodes via [`try_reuse_axis_offset`],
/// and the outer-axes tuple is novel for this `(data_id, axis)` so
/// far (deduped against already-discovered groups).
///
/// The walk descends through nested `Event::Loop` bodies and
/// `Event::Fire` arg bindings, including `ArgBinding::Nested`'s
/// `Vec<ArgBinding>`. Unlike the pre-TASK-0282 first-cut, it does
/// NOT stop after the first match per axis — the body is walked in
/// full so every outer-coord variation gets its own buffer (e.g.
/// the 3-row 05-stencil/reuse fixture produces 3 groups on
/// `(img_in, axis=1)`, one each for outer axes `[y-1]`, `[y]`, and
/// `[y+1]`).
///
/// Group indexing is in source-discovery order: the first novel
/// `(axis, outer_axes)` tuple under a given `(data_id, axis)` is
/// `group_idx = 0`, the second is `1`, etc. The body walk is
/// deterministic (source-order Vec/BTreeMap iteration), so the
/// resulting `group_idx` assignment is reproducible.
///
/// Returns `BTreeMap<DataId, Vec<ReuseRewriteGroup>>` ordered by
/// `DataId` (BTreeMap), and within each `Vec` by source-discovery
/// order (which sorts by axis ascending — `per_axis` is a BTreeMap —
/// then by outer-axes discovery order within an axis).
///
/// Fail-loud on a missing `name_data` entry for any reuse-active
/// `DataId`: the buffer identifier
/// `__reuse_buf_<data_name>_a<axis>_g<group_idx>` requires a real
/// symbol name and an absent one is a projection-layer gap, NOT a
/// silent fallback to `d<id>` (TASK-0269 cycle-103 review architect
/// P2.3 — siblings like [`super::fire::data_name`] in this module are
/// already fail-loud; the reuse-discovery path now matches).
fn discover_reuse_groups(
    body: &[Event],
    iv_name: &str,
    per_data: &BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>,
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<BTreeMap<DataId, Vec<ReuseRewriteGroup>>, EmitError> {
    // TASK-0283: materialise the consts table ONCE for the whole walk
    // (instead of converting per Fire-arg) — the affine-decomposition
    // function `affine_decompose` consumes `BTreeMap<String, ResolvedConst>`
    // while the sidecar carries `BTreeMap<String, ConstValue>`. Same
    // value semantics, slightly different shapes; this conversion
    // is the boundary.
    let consts_resolved = sidecar_consts_to_resolved(sidecar);
    let mut out: BTreeMap<DataId, Vec<ReuseRewriteGroup>> = BTreeMap::new();
    for (data_id, per_axis) in per_data {
        // Eagerly resolve the data name once per (data, ...) pair —
        // fail-loud if absent, matching `data_name`'s discipline. A
        // missing entry would still be caught later by `data_name` in
        // `render_reuse_buf_decls`, but only AFTER the silent `d<id>`
        // fallback shaped the `buf_ident` carried in the group. Hoist
        // the lookup so the group is built with the real name or not
        // at all.
        let data_name_resolved = names.data.get(data_id).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!(
                "reuse-active data id {data_id:?} has no name in NameTables \
                 (TASK-0269 — buffer ident requires a real symbol name)"
            ))
        })?;
        // TASK-0282: collect EVERY unique (axis, outer_axes) tuple per
        // (data_id) — no early-out. The walk visits the body in source
        // order; each novel outer_axes tuple under a given axis becomes
        // a new group with group_idx = (count of existing groups for
        // that axis at discovery time).
        let mut found: Vec<ReuseRewriteGroup> = Vec::new();
        for ev in body {
            walk_event_for_reuse(
                ev,
                *data_id,
                &data_name_resolved,
                iv_name,
                per_axis,
                &mut found,
                &consts_resolved,
            );
        }
        if !found.is_empty() {
            out.insert(*data_id, found);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk_event_for_reuse(
    ev: &Event,
    data_id: DataId,
    data_name: &str,
    iv_name: &str,
    per_axis: &BTreeMap<u64, ReuseSlot>,
    found: &mut Vec<ReuseRewriteGroup>,
    consts: &BTreeMap<String, ResolvedConst>,
) {
    match ev {
        Event::Fire { bindings, .. } => {
            for arg in &bindings.inputs {
                walk_arg_for_reuse(arg, data_id, data_name, iv_name, per_axis, found, consts);
            }
        }
        Event::Loop { body, .. } => {
            for child in body {
                walk_event_for_reuse(child, data_id, data_name, iv_name, per_axis, found, consts);
            }
        }
        // Sync / Push / Wait / Alloc / Free carry no Fire-arg DataRefs.
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_arg_for_reuse(
    arg: &ArgBinding,
    data_id: DataId,
    data_name: &str,
    iv_name: &str,
    per_axis: &BTreeMap<u64, ReuseSlot>,
    found: &mut Vec<ReuseRewriteGroup>,
    consts: &BTreeMap<String, ResolvedConst>,
) {
    match arg {
        ArgBinding::Data(s) => {
            if s.data != data_id || s.indices.is_empty() {
                return;
            }
            for (axis, slot) in per_axis {
                let ax_idx = *axis as usize;
                if ax_idx >= s.indices.len() {
                    continue;
                }
                // The reuse-axis index must decode as `iv + b`. If not,
                // this DataRef is out-of-pattern (a non-iv index on the
                // reuse axis); skip it. Stage 1 would have rejected the
                // body if NO DataRef matched, so we're guaranteed at
                // least one match per axis (else `per_axis` wouldn't
                // contain `axis`).
                if try_reuse_axis_offset(&s.indices[ax_idx], iv_name, consts).is_none() {
                    continue;
                }
                // Build the OUTER axes (all axes except the reuse one),
                // source-order preserved. TASK-0286 canonicalises each
                // outer-axis IrExpr before the dedupe key is taken —
                // additive/multiplicative identity folds (`y + 0`,
                // `y - 0`, `y * 1`) become the bare operand so two
                // semantically-equal-but-structurally-distinct outer
                // axes from upstream passes coalesce into one group.
                let outer_axes: Vec<IrExpr> = s
                    .indices
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| {
                        if i == ax_idx {
                            None
                        } else {
                            Some(canonicalise_outer_axis(e))
                        }
                    })
                    .collect();
                // TASK-0282 dedupe: skip if a group with this exact
                // `(axis, outer_axes)` already exists. `IrExpr` carries
                // `PartialEq` (structural equality on the AST), so the
                // dedupe is direct. Both stored and search-key sides
                // pass through `canonicalise_outer_axis` (TASK-0286),
                // so semantically-equal outer axes from upstream
                // canonical-form drift cannot over-emit a redundant
                // buffer.
                if found
                    .iter()
                    .any(|g| g.axis == *axis && g.outer_axes == outer_axes)
                {
                    continue;
                }
                // TASK-0282 group_idx: in source-discovery order within
                // an axis. New axis -> 0; second outer-coord variant on
                // the same axis -> 1; etc. The `_g<group_idx>` suffix
                // is uniform — single-group cases carry `_g0` rather
                // than a bare `_a<axis>`.
                let group_idx = found.iter().filter(|g| g.axis == *axis).count() as u64;
                let buf_ident = format!("__reuse_buf_{}_a{}_g{}", data_name, axis, group_idx);
                found.push(ReuseRewriteGroup {
                    axis: *axis,
                    group_idx,
                    slot: *slot,
                    outer_axes,
                    buf_ident,
                    iv_name: iv_name.to_string(),
                });
            }
        }
        ArgBinding::Nested { args, .. } => {
            for inner in args {
                walk_arg_for_reuse(inner, data_id, data_name, iv_name, per_axis, found, consts);
            }
        }
        ArgBinding::Scalar(_) => {}
    }
}

/// Emit `Vec<T>` circular-buffer declarations + initial-fill prologue
/// for every reuse group active on `iter_var` in `body`. Returns the
/// `reuse_active` map the caller seeds into the child `RenderCtx` it
/// recurses into the body with.
///
/// The prologue unrolls the fills for offsets `b in min_offset ..
/// min_offset+length-1` (i.e. every offset EXCEPT the most-distant
/// `max_offset = min_offset + length - 1`). The per-iter update for
/// `max_offset` is the responsibility of [`render_reuse_per_iter_update`]
/// at body entry.
///
/// `lo_expr_rs` is the Rust expression for the loop's `lo` bound
/// (e.g. `"1_i64"` for `for x : 1..W-1`). It substitutes the iv in
/// the source-axis index at prologue time — the reuse-axis position
/// of the source array reads becomes `lo + b` for each prologue
/// offset.
///
/// `body` is the loop's body — walked once to discover the canonical
/// outer-axes pattern per `(data_id, axis)`.
///
/// Empty path is a true no-op (returns empty map, writes nothing).
/// Byte-identicality with the pre-TASK-0269 emit holds for every
/// schedule without an active reuse slot.
pub fn render_reuse_buf_decls(
    out: &mut String,
    indent: usize,
    iter_var: IterVar,
    iter_var_name: &str,
    lo_expr_rs: &str,
    body: &[Event],
    ctx: &RenderCtx<'_>,
) -> Result<BTreeMap<DataId, Vec<ReuseRewriteGroup>>, EmitError> {
    use std::fmt::Write as _;
    let Some(per_data) = ctx.sidecar.reuse_widths.get(&iter_var) else {
        return Ok(BTreeMap::new());
    };
    if per_data.is_empty() {
        return Ok(BTreeMap::new());
    }
    let groups = discover_reuse_groups(body, iter_var_name, per_data, ctx.names, ctx.sidecar)?;
    let pad = "    ".repeat(indent);
    for (data_id, gs) in &groups {
        let data_name = data_name(*data_id, ctx)?;
        let ty = ctx.sidecar.data_type(*data_id).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "reuse buffer for data `{data_name}` ({:?}) has no \
                 ResolvedType in the NameSidecar (TASK-0269)",
                data_id
            ))
        })?;
        let scalar_ty = rust_scalar_type(&ty.scalar);
        // Zero (NOT combine-identity, TASK-0343.01.02): this is a
        // circular-buffer reuse window, never an accumulator fan-in —
        // every slot is fully overwritten by the prologue/loop before
        // read, so the init value is dead. Stays zero deliberately.
        let zero = rust_scalar_zero(&ty.scalar);
        for g in gs {
            // 1. Buffer decl.
            let _ = writeln!(
                out,
                "{pad}let mut {buf}: Vec<{scalar_ty}> = vec![{zero}; {length}usize];",
                buf = g.buf_ident,
                length = g.slot.length,
            );
            // 2. Prologue: fill every offset EXCEPT the most-distant
            //    one (which is filled per-iter inside the body). The
            //    `lo + b` source-axis index substitutes the iv for the
            //    prologue's evaluation.
            //
            //    The source array's flat index is computed by the same
            //    `render_flat_index`-style sum the body uses; we build
            //    a synthetic `DataSlice` with the prologue's
            //    iv-substituted index and pass it through
            //    `render_flat_index`.
            let max_offset = g.slot.min_offset + (g.slot.length as i64) - 1;
            for b in g.slot.min_offset..max_offset {
                let prologue_slice = prologue_slice_for_offset(g, *data_id, b, lo_expr_rs);
                let src_flat = render_flat_index(&prologue_slice, ctx)?;
                // The buffer slot index MUST match the body's read
                // formula `buf[(iv + b - min_offset).rem_euclid(L)]`
                // evaluated at iv == lo (the first body iteration).
                // Hence: `buf[((lo + b) - min_offset).rem_euclid(L)]`.
                // We emit this as a runtime expression to avoid having
                // to const-fold `lo` at codegen time (it may carry
                // `H-1`-style symbolic-const subtrees from
                // `render_const_expr`).
                let length = g.slot.length;
                let _ = writeln!(
                    out,
                    "{pad}{buf}[(((({lo_expr_rs}) + ({b}_i64) - ({min_offset}_i64)).rem_euclid({length}_i64)) as usize)] = {data_name}[{src_flat}];",
                    buf = g.buf_ident,
                    min_offset = g.slot.min_offset,
                );
            }
        }
    }
    Ok(groups)
}

/// Emit the per-iteration most-distant-element load for every active
/// reuse group. Called at body entry, AFTER the loop header and
/// BEFORE recursing into the body, so the slot is current when any
/// Fire arg reads it.
///
/// `iv_expr_rs` is the Rust expression for the iv (typically just the
/// iv variable name, or the rebound absolute expression under a
/// strip-mined inner loop).
pub fn render_reuse_per_iter_update(
    out: &mut String,
    indent: usize,
    groups: &BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
    iv_expr_rs: &str,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    use std::fmt::Write as _;
    if groups.is_empty() {
        return Ok(());
    }
    let pad = "    ".repeat(indent);
    for (data_id, gs) in groups {
        let data_name = data_name(*data_id, ctx)?;
        for g in gs {
            let max_offset = g.slot.min_offset + (g.slot.length as i64) - 1;
            // Source flat index uses the LIVE iv expression
            // (`iv + max_offset`) on the reuse axis.
            let live_slice = live_slice_for_offset(g, *data_id, max_offset, iv_expr_rs);
            let src_flat = render_flat_index(&live_slice, ctx)?;
            // The buffer slot rotates with the iv. We could fold the
            // `as i64` + `rem_euclid` away if iv is known non-negative,
            // but keeping the rem_euclid form makes the rewrite
            // uniform between the prologue (where the slot index is a
            // const u64 literal) and the per-iter update + rewrite
            // sites (where the slot index depends on the live iv).
            let length = g.slot.length;
            let _ = writeln!(
                out,
                "{pad}{buf}[((({iv_expr_rs}) + ({max_offset}_i64) - ({min_offset}_i64)).rem_euclid({length}_i64)) as usize] = {data_name}[{src_flat}];",
                buf = g.buf_ident,
                min_offset = g.slot.min_offset,
            );
        }
    }
    Ok(())
}

/// Build the synthetic DataSlice the prologue uses to fetch one slot
/// from the source array. Reuse axis is replaced with the literal
/// `(<lo_expr_rs>) + (b)`; outer axes are the canonical
/// [`ReuseRewriteGroup::outer_axes`] pattern verbatim.
///
/// `render_flat_index` consumes the result — its render path treats
/// our synthetic IrExpr nodes the same as natural ones (no
/// observational difference).
fn prologue_slice_for_offset(
    group: &ReuseRewriteGroup,
    data_id: DataId,
    b: i64,
    lo_expr_rs: &str,
) -> DataSlice {
    // Build the prologue's reuse-axis index expression as
    // `Ident("(lo) + (b)")` — render_int_expr emits Ident verbatim
    // (the `abs_subst` table is empty for this synthetic path), so
    // the printed text is exactly the Rust expression we want. This
    // is the same precedent `abs_subst`'s rebound expressions use:
    // pre-rendered Rust strings smuggled through an Ident node.
    let mut indices: Vec<IrExpr> = Vec::with_capacity(group.outer_axes.len() + 1);
    let mut outer_iter = group.outer_axes.iter();
    let ax_idx = group.axis as usize;
    // Splice the reuse-axis index back at its original position.
    for i in 0..(group.outer_axes.len() + 1) {
        if i == ax_idx {
            indices.push(IrExpr::Ident(format!("({lo_expr_rs}) + ({b}_i64)")));
        } else {
            indices.push(
                outer_iter
                    .next()
                    .expect("outer_axes length matches")
                    .clone(),
            );
        }
    }
    DataSlice {
        data: data_id,
        indices,
    }
}

/// Same as [`prologue_slice_for_offset`] but for the live per-iter
/// update — the reuse-axis index becomes
/// `(<iv_expr_rs>) + (offset)`.
fn live_slice_for_offset(
    group: &ReuseRewriteGroup,
    data_id: DataId,
    offset: i64,
    iv_expr_rs: &str,
) -> DataSlice {
    let ax_idx = group.axis as usize;
    let mut indices: Vec<IrExpr> = Vec::with_capacity(group.outer_axes.len() + 1);
    let mut outer_iter = group.outer_axes.iter();
    for i in 0..(group.outer_axes.len() + 1) {
        if i == ax_idx {
            indices.push(IrExpr::Ident(format!("({iv_expr_rs}) + ({offset}_i64)")));
        } else {
            indices.push(
                outer_iter
                    .next()
                    .expect("outer_axes length matches")
                    .clone(),
            );
        }
    }
    DataSlice {
        data: data_id,
        indices,
    }
}

// --------------------------------------------------------------------
// `_pub` wrappers — thin shims for multi-worker callers
// --------------------------------------------------------------------

/// Public shim for [`render_reuse_buf_decls`] consumed by the shared
/// multi-worker walker (TASK-0270). Delegates to the private impl via
/// `ctx.inner()`, mirroring the rest of the `_pub` shim layer (see
/// [`super::fire::render_fire_args_pub`],
/// [`super::fire::render_flat_index_pub`], etc.).
///
/// Returns the per-`DataId` [`ReuseRewriteGroup`] vector the caller
/// seeds into the child [`RenderCtxPub`] it recurses into the body
/// with (via [`RenderCtxPub::with_reuse_active`] or
/// [`RenderCtxPub::with_abs_subst_and_reuse_active`]). The vector
/// holds one entry per UNIQUE `(axis, outer_axes_tuple)` discovered
/// in the body (TASK-0282 multi-outer-coord generalisation).
pub fn render_reuse_buf_decls_pub(
    out: &mut String,
    indent: usize,
    iter_var: IterVar,
    iter_var_name: &str,
    lo_expr_rs: &str,
    body: &[Event],
    ctx: &RenderCtxPub<'_>,
) -> Result<BTreeMap<DataId, Vec<ReuseRewriteGroup>>, EmitError> {
    render_reuse_buf_decls(
        out,
        indent,
        iter_var,
        iter_var_name,
        lo_expr_rs,
        body,
        &ctx.inner(),
    )
}

/// Public shim for [`render_reuse_per_iter_update`] consumed by the
/// shared multi-worker walker (TASK-0270). Delegates to the private
/// impl via `ctx.inner()`. Called at body entry, AFTER the loop header
/// and BEFORE recursing into the body, so the buffer slot is current
/// when any Fire arg reads it.
pub fn render_reuse_per_iter_update_pub(
    out: &mut String,
    indent: usize,
    groups: &BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
    iv_expr_rs: &str,
    ctx: &RenderCtxPub<'_>,
) -> Result<(), EmitError> {
    render_reuse_per_iter_update(out, indent, groups, iv_expr_rs, &ctx.inner())
}
