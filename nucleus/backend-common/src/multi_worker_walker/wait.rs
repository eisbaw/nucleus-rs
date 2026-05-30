//! Receiver-side gather shape dispatch for `Event::Wait` (TASK-0117 1D
//! leading-axis path + TASK-0294 2D row-loop path). Consumed by the
//! shared event walker AND directly by mp-tcp-bufsync (which bypasses
//! [`super::event_walker::render_worker_events`] and calls
//! [`render_wait_assign`] from its own event walker).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, IterTile, IterVar, SeqTag};
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
    /// N-D nested-loop slice-paste (TASK-0341.02.02.01.01). The
    /// general shape: one or more FULL leading axes (each emitted as a
    /// `for` loop accumulating a flat base offset), exactly ONE BANDED
    /// axis (the partitioned axis — a contiguous sub-range), and zero
    /// or more FULL trailing axes (absorbed into the contiguous copy
    /// span). The 16-jacobi/distributed `field[ITERS+1][H][W]` ×
    /// `partition=rows(y)` tile `[(t, 0..T FULL), (y, band), (x, 0..W
    /// FULL)]` is the load-bearing case: `t` is a full leading axis
    /// (one loop), `y` is the banded axis, `x` is a full trailing axis
    /// (folded into the per-`t` contiguous span).
    ///
    /// - `leading`: per full-leading-axis `(dim, stride)`. `dim` is
    ///   `ty.dims[k]` (the loop count); `stride` is `product(ty.dims[k+1..])`
    ///   (the flat-element step per unit of that axis). Outer-to-inner.
    /// - `band_lo_off` / `band_hi_off`: the banded-axis range
    ///   pre-multiplied by `product(ty.dims[banded+1..])` (the
    ///   banded-axis-element stride), i.e. the contiguous flat span to
    ///   copy WITHIN each leading base block.
    NestedRows {
        leading: Vec<(usize, usize)>,
        band_lo_off: usize,
        band_hi_off: usize,
    },
}

/// Render the receiver-side assignment statement for one Wait event.
/// Returns one statement (no trailing newline).
///
/// Three shapes, dispatched by `wait_slice`:
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
#[allow(clippy::too_many_arguments)]
pub fn render_wait_assign(
    sidecar: &NameSidecar,
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    name: &str,
    data: DataId,
    seq: SeqTag,
    rhs: &str,
    accumulate: bool,
    let_at_wait: &BTreeSet<DataId>,
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
            } else if let_at_wait.contains(&data) {
                // TASK-0349 cycle 220: whole-array recv on a data
                // symbol whose ONLY Waits are whole-array (and which
                // is not accumulate-fan-in and not indexed-Fire-
                // written). The per-backend pre-init pass omits the
                // `let mut <name>: Vec<..> = vec![0; N];` line for
                // these data; the emit MUST declare-and-assign in
                // one statement so the variable comes into scope at
                // the recv site. Type inference picks up the Vec<T>
                // from the rendezvous slot's `.wait()` return type.
                //
                // SCOPE HAZARD (TASK-0356 cycle 222, characterized;
                // guard LANDED as TASK-0364, OPTION B): this `let
                // {name}` comes into scope at the WAIT'S lexical
                // position. When the Wait sits inside an `Event::Loop`
                // body, the `let` lands inside the emitted `for { }`
                // block; a consumer of `{name}` at the ENCLOSING outer
                // scope would then read it out of scope (non-compiling
                // Rust). This is NOT producible today —
                // `transfer_inject` (`inject_in_sequence`) co-locates
                // every cross-worker Wait in the SAME sequence as its
                // consuming Operation, so an outer-scope consumer gets
                // its Wait at the outer scope, never in a nested loop.
                // This emit is therefore unchanged by TASK-0364; the
                // guard is the sibling
                // `super::collect::check_let_at_wait_scope_safety`,
                // called once at `super::event_walker::
                // render_worker_events` entry, which fails LOUD with
                // `EmitError::ContractGap` BEFORE this emit runs if a
                // future pass ever constructs the at-risk scope. The
                // boundary is pinned by
                // `tests/wait_let_at_wait_loop_scope.rs`.
                Ok(format!("let {name} = {rhs};"))
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
        Some(WaitSlice::NestedRows {
            leading,
            band_lo_off,
            band_hi_off,
        }) => {
            // N-D nested-loop slice-paste (TASK-0341.02.02.01.01). Emit
            // one `for _aK in 0..dim` loop per full leading axis,
            // accumulating a flat `_base` offset, then a single
            // contiguous `copy_from_slice` over the banded span within
            // each base block. The `_aK` / `_base` locals are
            // underscore-prefixed AND inside a `{ ... }` block (Rust
            // block-shadowing makes them collision-safe wherever this
            // Wait is placed). 16-jacobi/distributed: `leading = [(5,
            // 64)]` (the t axis), `band_lo_off..band_hi_off` = the
            // per-row y-band flat span (e.g. `8..24` for band 1..3 on a
            // [.][8][8] data symbol).
            let mut s = String::from("{ let _tmp = ");
            s.push_str(rhs);
            s.push_str("; ");
            // `_base` starts at 0 and each loop level adds `_aK * stride`.
            // We open the loops, accumulating the base via a fresh
            // shadowed binding at each level so the innermost `_base`
            // sees the full accumulation.
            s.push_str("let _base = 0usize; ");
            let mut closers = 0usize;
            for (lvl, (dim, stride)) in leading.iter().enumerate() {
                s.push_str(&format!(
                    "for _a{lvl} in 0usize..{dim}usize {{ let _base = _base + _a{lvl} * {stride}usize; "
                ));
                closers += 1;
            }
            s.push_str(&format!(
                "{name}[_base + {band_lo_off}usize.._base + {band_hi_off}usize]\
                 .copy_from_slice(\
                 &_tmp[_base + {band_lo_off}usize.._base + {band_hi_off}usize]); "
            ));
            for _ in 0..closers {
                s.push('}');
                s.push(' ');
            }
            s.push('}');
            Ok(s)
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
    // N-D nested-loop dispatch (TASK-0341.02.02.01.01, cycle 213): a
    // tile/data shape with rank >= 3 (or a rank-2 tile against rank
    // >= 3 data) is routed to [`nd_banded_slice`], which validates
    // each positional axis range against its dim and emits the
    // [`WaitSlice::NestedRows`] full-leading + one-banded + full-
    // trailing slice-paste. A whole-array rank-N tile collapses to
    // `Ok(None)` there (every axis full). 16-jacobi/distributed's
    // `field[ITERS+1][H][W]` × `partition=rows(y)` write-band tile
    // `[(t, 0..T), (y, band), (x, 0..W)]` is the load-bearing case (t
    // full leading, y banded, x full trailing). Shapes the N-D path
    // does NOT yet support (multiple banded axes at rank >= 3) still
    // fail LOUD inside `nd_banded_slice` rather than emit a wrong
    // gather.
    //
    // Pre-cycle-213 this region was a hard `ContractGap` reject for
    // every rank >= 3 shape (TASK-0294 cycle-115 architect P2.1). The
    // 16-jacobi/distributed blocker required THREE co-landed fixes:
    // the placement hoist (TASK-0341.02.02.01.02), the cumulative-
    // array COPY-not-accumulate classifier (TASK-0341.02.02.01.03),
    // and this N-D dispatch — see those tasks for the full chain.
    if tile.bounds.len() >= 3 || (tile.bounds.len() >= 2 && ty.dims.len() > 2) {
        return nd_banded_slice(data, &ty.dims, &tile.bounds);
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

/// N-D nested-loop slice-paste dispatch (TASK-0341.02.02.01.01, cycle
/// 213). The tile carries positional `(iv, range)` bounds with the
/// convention `bounds[i] ↔ dims[i]` (row-major nest order, the same
/// convention `wait_slice`'s 1D/2D arms use). Each axis is classified
/// full (range covers `0..dims[i]`) or banded (a strict sub-range).
///
/// Supported shape: ANY number of FULL leading axes + exactly ONE
/// BANDED axis + ANY number of FULL trailing axes. This is the
/// `partition=rows(y)` write-band shape on a `[t][y][x]` (or deeper)
/// cumulative array — one partitioned spatial axis banded, the
/// iteration axis (`t`) and the inner spatial axis (`x`) full.
///
/// - All axes full ⇒ `Ok(None)` (whole-array assign, byte-identical
///   with the rank <= 2 degenerate-full collapse).
/// - Exactly one banded axis ⇒ `Ok(Some(NestedRows { .. }))`. The full
///   leading axes become nested `for` loops accumulating a flat base;
///   the banded axis contributes the contiguous copy span (its range ×
///   the product of all trailing dims); full trailing axes are folded
///   into that span. The 16-jacobi case `[(t,0..5),(y,1..3),(x,0..8)]`
///   on dims `[5,8,8]` ⇒ `leading=[(5,64)]`, `band_lo_off=1*8=8`,
///   `band_hi_off=3*8=24`.
/// - Two or more banded axes ⇒ `Err(ContractGap)` (no shipped schedule
///   constructs a multi-banded rank >= 3 tile; the 2D two-banded case
///   is the rank-2 `Rows` arm; fail LOUD rather than emit a
///   multi-banded gather this path does not yet support).
/// - Tile rank != data dim rank ⇒ `Err(ContractGap)` (the positional
///   convention requires one bound per dim; a partial-rank tile would
///   silently mis-map axes).
/// - Any axis range out of bounds / empty ⇒ `Err(ContractGap)`.
fn nd_banded_slice(
    data: DataId,
    dims: &[usize],
    bounds: &[(IterVar, std::ops::Range<i64>)],
) -> Result<Option<WaitSlice>, EmitError> {
    if bounds.len() != dims.len() {
        return Err(EmitError::ContractGap(format!(
            "Wait of data {data:?}: N-D banded slice needs one tile bound per data \
             dim (tile rank {}, data dim rank {}); the positional axis-mapping \
             convention `bounds[i] <-> dims[i]` cannot resolve a partial-rank tile \
             (TASK-0341.02.02.01.01)",
            bounds.len(),
            dims.len(),
        )));
    }
    // Classify each axis full vs banded; validate ranges.
    let mut banded_axis: Option<usize> = None;
    for (i, (_iv, range)) in bounds.iter().enumerate() {
        let dim = dims[i] as i64;
        if range.start < 0 || range.end > dim || range.start >= range.end {
            return Err(EmitError::ContractGap(format!(
                "Wait of data {data:?}: tile axis {i} range {range:?} out of bounds \
                 for data dims {dims:?} (dim {dim}) (TASK-0341.02.02.01.01)"
            )));
        }
        let full = range.start == 0 && range.end == dim;
        if !full {
            if banded_axis.is_some() {
                return Err(EmitError::ContractGap(format!(
                    "Wait of data {data:?}: tile has >=2 banded axes (axis {} and axis \
                     {i}); the N-D nested-loop slice-paste supports exactly one banded \
                     axis (full-leading + one-banded + full-trailing). No shipped \
                     schedule constructs a multi-banded rank >= 3 tile \
                     (TASK-0341.02.02.01.01)",
                    banded_axis.unwrap(),
                )));
            }
            banded_axis = Some(i);
        }
    }
    // All axes full ⇒ whole-array.
    let Some(b) = banded_axis else {
        return Ok(None);
    };
    // Stride of the banded axis = product of all trailing dims.
    let band_stride: usize = dims[b + 1..].iter().product();
    let band_lo_off = (bounds[b].1.start as usize).saturating_mul(band_stride);
    let band_hi_off = (bounds[b].1.end as usize).saturating_mul(band_stride);
    // Full leading axes (those BEFORE the banded axis) become loops.
    // Each carries (dim, stride) where stride = product of dims after it.
    let mut leading: Vec<(usize, usize)> = Vec::with_capacity(b);
    for (i, d) in dims.iter().enumerate().take(b) {
        let stride: usize = dims[i + 1..].iter().product();
        leading.push((*d, stride));
    }
    Ok(Some(WaitSlice::NestedRows {
        leading,
        band_lo_off,
        band_hi_off,
    }))
}

/// Module-private classifier wrapping `wait_slice` for sibling
/// collect-pass consumers (TASK-0349 cycle 220
/// `collect_let_at_wait_data` in `super::collect`). Returns
/// `Ok(true)` when the (data, tile) pair resolves to whole-array
/// recv (the `None` arm of `wait_slice`); `Ok(false)` when it
/// resolves to slice-paste; `Err` on shape-error or sidecar-lookup
/// invariant violations (wait_slice's Err arms span both classes —
/// rank > 2, out-of-bounds range, AND `NameSidecar::data_type`
/// returning `None`).
///
/// Visibility narrowed to `pub(super)` cycle 220b per architect P2.2
/// — no consumer outside `multi_worker_walker` needs to call this
/// directly; the only call site is `super::collect::collect_let_at_wait_inner`.
pub(super) fn is_whole_array_recv(
    sidecar: &NameSidecar,
    data: DataId,
    tile: &IterTile,
) -> Result<bool, EmitError> {
    Ok(wait_slice(sidecar, data, tile)?.is_none())
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
fn render_accumulate_assign(name: &str, rhs: &str, ty: &ResolvedType) -> Result<String, EmitError> {
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
