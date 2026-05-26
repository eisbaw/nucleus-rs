//! Receiver-side gather shape dispatch for `Event::Wait` (TASK-0117 1D
//! leading-axis path + TASK-0294 2D row-loop path). Consumed by the
//! shared event walker AND directly by mp-tcp-bufsync (which bypasses
//! [`super::event_walker::render_worker_events`] and calls
//! [`render_wait_assign`] from its own event walker).

use std::collections::BTreeMap;

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, IterTile, SeqTag};
use nucleus_compiler::sidecar::NameSidecar;

use crate::render::EmitError;

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
    accumulate: bool,
) -> Result<String, EmitError> {
    let slice = match pair_tiles.get(&(data, seq)) {
        Some(tile) => wait_slice(sidecar, data, tile)?,
        None => None,
    };
    match slice {
        None => {
            // Empty tile (or whole-array match) — pre-cycle-189 path
            // was whole-array assign; cycle-189 adds the accumulate
            // dispatch.
            if accumulate {
                // TASK-0343 cycle 189: overlapping-write accumulator
                // fan-in (N>=2 whole-array Waits on the same data,
                // detected by `collect_accumulate_waits` in sibling
                // collect.rs). Emit element-wise wrapping_add into
                // the pre-initialised destination instead of a bare
                // overwrite assign. Pre-cycle-189 every such Wait
                // emitted `name = rhs;` producing last-write-wins on
                // 08-histogram/distributed (4 sequential overwrites
                // yielded one worker's standalone partial).
                let ty = sidecar.data_type(data).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "accumulate Wait of data {data:?} has no ResolvedType in NameSidecar"
                    ))
                })?;
                render_accumulate_assign(name, rhs, ty)
            } else {
                Ok(format!("{name} = {rhs};"))
            }
        }
        Some(_) if accumulate => {
            // Defensive: `collect_accumulate_waits` (sibling
            // collect.rs) only classifies whole-array Waits as
            // accumulate, so reaching here with a slice tile means a
            // caller fabricated the flag against a slice-paste tile.
            // Fail LOUD so the divergence between the collector and
            // the consumer surfaces at compile time rather than
            // silently mis-combining a disjoint-write slice gather.
            Err(EmitError::ContractGap(format!(
                "render_wait_assign: accumulate=true on Wait of data {data:?} \
                 (seq {seq:?}) whose tile resolves to a slice-paste arm; \
                 `collect_accumulate_waits` only classifies whole-array tiles \
                 as accumulate. This is a contract gap between the caller and \
                 the accumulate collector (TASK-0343 cycle 189)."
            )))
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
/// Module-private — all three `render_worker_events`-using
/// backends (pthreads-sync, pthreads-async, mp-tcp-event) consume
/// this only indirectly via `render_wait_assign`. mp-tcp-bufsync
/// calls `render_wait_assign` directly without going through
/// `render_worker_events`, so it consumes this helper through the
/// same surface.
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

/// Element-wise sum-identity accumulate emit for the overlapping-write
/// fan-in arm of `render_wait_assign` (TASK-0343, cycle 189).
///
/// Emits one of:
/// - Array: `{ let _tmp = <rhs>; for _k in 0..<LEN>usize { <name>[_k] =
///   <name>[_k].wrapping_add(_tmp[_k]); } }`. `<LEN>` is the product of
///   `ty.dims` (flat element count; matches `render_array_init`'s
///   per-backend zero-init).
/// - Scalar: `<name> = <name>.wrapping_add(<rhs>);`. The scalar case
///   is structurally degenerate but kept for completeness — if a
///   future schedule fans a scalar accumulator into a host worker
///   the emit is well-defined without falling back to the array form.
///
/// # Scalar-type carve-out
///
/// Returns `EmitError::ContractGap` for floats / bool (filed as
/// TASK-0343 follow-up). Sum identity is integer-only in v2 today:
/// - Float addition collides with PRD §10.1 bit-identity (sum order
///   is not associative-stable).
/// - Bool has no canonical "sum" identity (OR vs AND vs XOR are
///   distinct algebraic operators; the user-level intent must be
///   declared explicitly when the follow-up lands).
fn render_accumulate_assign(
    name: &str,
    rhs: &str,
    ty: &ResolvedType,
) -> Result<String, EmitError> {
    let op = accumulate_op_for_scalar(&ty.scalar)?;
    if ty.dims.is_empty() {
        // Scalar accumulator. The pre-init in each backend has set
        // `<name>` to the sum identity (0 for integers; see
        // `render_array_init` / `rust_scalar_zero` in each backend's
        // `multi_worker.rs`).
        Ok(format!("{name} = {name}.{op}({rhs});"))
    } else {
        let total: usize = ty.dims.iter().copied().product();
        Ok(format!(
            "{{ let _tmp = {rhs}; \
             for _k in 0..{total}usize {{ \
             {name}[_k] = {name}[_k].{op}(_tmp[_k]); \
             }} }}"
        ))
    }
}

/// Per-scalar-type accumulate operator name. Integer-only in cycle
/// 189; float / bool carve-outs surface as `EmitError::ContractGap`
/// pointing to the TASK-0343 follow-up bucket.
fn accumulate_op_for_scalar(t: &ScalarType) -> Result<&'static str, EmitError> {
    match t {
        ScalarType::Usize
        | ScalarType::Isize
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64 => Ok("wrapping_add"),
        ScalarType::F32 | ScalarType::F64 => Err(EmitError::ContractGap(
            "render_wait_assign: accumulate fan-in on a float-scalar data symbol — \
             sum identity collides with PRD §10.1 bit-identity invariant (float \
             addition is not associative-stable across worker arrival order); \
             not supported in TASK-0343 cycle 189 (filed as TASK-0343.02: float / \
             bool follow-up — needs PRD §10.1-compatible identity declared by the \
             user via TASK-0343.01 kernel attribute)"
                .into(),
        )),
        ScalarType::Bool => Err(EmitError::ContractGap(
            "render_wait_assign: accumulate fan-in on a bool-scalar data symbol — \
             no canonical sum identity (OR vs AND vs XOR are distinct algebraic \
             operators); not supported in TASK-0343 cycle 189 (filed as TASK-0343.02: \
             float / bool follow-up — needs identity declared by the user via \
             TASK-0343.01 kernel attribute)"
                .into(),
        )),
    }
}
