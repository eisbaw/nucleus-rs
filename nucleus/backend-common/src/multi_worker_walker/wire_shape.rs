//! `WireShape` — the SINGLE chokepoint for the cross-worker payload
//! shape of one `(DataId, SeqTag)` transfer edge (TASK-0455.07).
//!
//! # Why this exists
//!
//! The whole-array wire assumption used to be duplicated across five
//! structurally-independent SENDER emit sites plus one RECEIVER
//! chokepoint:
//!
//! - shared walker Push: `super::event_walker` emitted `…push({name}.clone())`;
//! - sync-TCP plan Push: `crate::tcp_plan::events` encoded the WHOLE
//!   symbol via `crate::tcp_plan::encode::encode_expr(&name, ty)`;
//! - MPI buffered-send sizing: `crate::mpi_plan::plan::Plan::bsend_bytes`
//!   budgeted each channel's WHOLE-array byte footprint;
//! - embedded Push: `backends/embedded-pattern/src/render` sized the
//!   payload with `size_of_val(&{name})` (the whole `[T; N]`);
//! - receiver `_tmp` basis: `super::wait::render_wait_assign` indexed
//!   `_tmp` with DESTINATION offsets, i.e. assumed a full-shaped wire
//!   payload.
//!
//! Five senders + one receiver each deciding the shape locally is the
//! silent-sibling divergence hazard (memory `feedback-silent-sibling-
//! defect`): when TASK-0453.22 flips the transmitted shape from
//! whole-array to the inferred gather/scatter region, it would have to
//! patch all six independently and any missed sibling would ship a
//! sender/receiver shape mismatch (a wire-protocol corruption, not a
//! compile error).
//!
//! [`WireShape`] folds the decision into ONE derivation
//! ([`WireShape::derive`]) computed once per `(DataId, SeqTag)` from the
//! pair tiles + sidecar dims the walkers already hold — the same inputs
//! [`super::wait::render_wait_assign`] already consumed. It exposes the
//! sender-side payload expression, the sender-side wire-encode
//! expression, the payload extent (elements / bytes), AND the receiver
//! `_tmp` offset basis, so every site reads ONE source of truth.
//!
//! # This task does NOT change wire behaviour
//!
//! [`WireShape::derive`] today always resolves to the whole-array
//! transmitted shape (`recv_basis == None` ⇒ the whole-symbol sender
//! payload + the whole-array receiver assign). The emitted code is
//! BYTE-IDENTICAL to pre-TASK-0455.07. The point of consolidation is
//! exactly so TASK-0453.22 flips the whole-array decision to the
//! inferred region in ONE place (the `recv_basis`/sender-expr coupling
//! here) with per-backend validation, instead of five-plus
//! independent edits.
//!
//! # Parametric-substrate precedent
//!
//! This follows the same single-chokepoint pattern as
//! `crate::tcp_plan::WirePrimitives` (the three wire calls),
//! `crate::event_plan::EventTransport` (the mio-reactor seam) and
//! `crate::mpi_plan::MpiRendezvous` (the MPI prelude): one place owns
//! the cross-backend-shared decision, the backends consume it.

use std::collections::BTreeMap;

use nucleus_compiler::algo::ResolvedType;
use nucleus_compiler::event::{DataId, IterTile, IterVar, SeqTag};
use nucleus_compiler::sidecar::NameSidecar;

use crate::render::EmitError;

/// Receiver-side gather basis for a Wait event's tile — the offset
/// arithmetic the `_tmp` paste arms in [`super::wait::render_wait_assign`]
/// use to copy the received payload into the destination.
///
/// `None` (carried as the absence of a [`RecvBasis`] on [`WireShape`])
/// is the WHOLE-ARRAY case: the wire payload is the full symbol and the
/// receiver does a whole-array assign. The three present variants are
/// the slice-paste shapes:
///
/// - `Flat { lo, hi }` — 1D leading-axis slice-paste
///   (`name[lo..hi].copy_from_slice(&_tmp[lo..hi])`). The TASK-0117
///   path. `lo`/`hi` are pre-multiplied flat-element offsets (i.e.
///   leading-axis index × product of inner dims).
/// - `Rows { outer_lo, outer_hi, row_stride, inner_lo_off,
///   inner_hi_off }` — 2D row-loop slice-paste, one `copy_from_slice`
///   per outer-axis iteration. The TASK-0294 path. Selected when the
///   tile has rank >= 2 AND the data has dim rank >= 2. Each worker
///   under `partition=blocks2d` owns a 2D rectangle of its data; the 1D
///   leading-axis path would paste the worker's whole y-band
///   (overwriting adjacent workers' columns with default-zero values),
///   so a row-loop is required for bit-identical gather. `row_stride`
///   is the per-outer-axis-element flat-element count (= product of
///   `dims[1..]`); `inner_lo_off` / `inner_hi_off` are the per-row
///   flat-element offsets of the inner-axis range (= inner-axis index ×
///   product of `dims[2..]`).
/// - `NestedRows { leading, band_lo_off, band_hi_off }` — N-D
///   nested-loop slice-paste (TASK-0341.02.02.01.01). One or more FULL
///   leading axes (each a `for` loop accumulating a flat base offset),
///   exactly ONE BANDED axis (the partitioned axis — a contiguous
///   sub-range), and zero or more FULL trailing axes (absorbed into the
///   contiguous copy span). The 16-jacobi/distributed
///   `field[ITERS+1][H][W]` × `partition=rows(y)` write-band tile
///   `[(t, 0..T FULL), (y, band), (x, 0..W FULL)]` is the load-bearing
///   case. `leading`: per full-leading-axis `(dim, stride)` outer-to-
///   inner, `dim` = `ty.dims[k]` (loop count), `stride` =
///   `product(ty.dims[k+1..])`. `band_lo_off`/`band_hi_off`: the
///   banded-axis range × `product(ty.dims[banded+1..])`.
///
/// This enum is the verbatim shape that used to live as the
/// module-private `WaitSlice` in `super::wait` (TASK-0117 / TASK-0294 /
/// TASK-0341.02.02.01.01); it moved here unchanged when TASK-0455.07
/// unified the sender + receiver derivation into [`WireShape`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecvBasis {
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
    NestedRows {
        leading: Vec<(usize, usize)>,
        band_lo_off: usize,
        band_hi_off: usize,
    },
}

/// The single derived cross-worker payload shape for one `(DataId,
/// SeqTag)` transfer edge (TASK-0455.07).
///
/// Constructed once per edge by [`WireShape::derive`] from the pair
/// tile + sidecar type. Carries everything the SENDER sites and the
/// RECEIVER chokepoint need so neither computes the shape locally:
///
/// - [`recv_basis`](Self::recv_basis) — the receiver `_tmp` offset
///   basis ([`None`] ⇒ whole-array assign; [`Some`] ⇒ a slice-paste
///   arm). Consumed by [`super::wait::render_wait_assign`].
/// - [`sender_value_expr`](Self::sender_value_expr) — the in-process
///   `.push(<expr>)` payload expression (e.g. `c.clone()`). Consumed by
///   the shared walker's `Event::Push` arm.
/// - [`sender_encode_expr`](Self::sender_encode_expr) — the wire-encode
///   expression (`wire::enc_*` / `wire::enc_vec(&name, …)`). Consumed by
///   the sync-TCP plan's `Event::Push` arm.
/// - [`extent_elems`](Self::extent_elems) /
///   [`extent_bytes`](Self::extent_bytes) — the transmitted element /
///   byte count. Consumed by MPI buffered-send sizing and the embedded
///   `size_of_val`-equivalent length argument.
///
/// # Whole-array today (TASK-0455.07), inferred-region next (TASK-0453.22)
///
/// Today `recv_basis` is the whole-array/slice-paste classification the
/// receiver already computed, and the SENDER helpers always return the
/// WHOLE symbol regardless of `recv_basis` — preserving byte-identical
/// emit (the wire still carries the whole array even when the receiver
/// only pastes a band). TASK-0453.22 will tighten the sender helpers to
/// emit only the `recv_basis` region (a slice expression / a
/// sub-extent), flipping the transmitted shape in this ONE place; the
/// receiver basis then indexes `_tmp` from 0 rather than from the
/// destination offset. That coupling is why the sender expression and
/// the receiver basis must be derived together here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireShape {
    /// The receiver `_tmp` offset basis. `None` ⇒ whole-array assign;
    /// `Some` ⇒ one of the slice-paste arms. See [`RecvBasis`].
    pub recv_basis: Option<RecvBasis>,
    /// Total flat element count of the data symbol (product of dims;
    /// `1` for a scalar). The whole-array extent — the transmitted
    /// element count THIS task (TASK-0455.07); TASK-0453.22 narrows it
    /// to the `recv_basis` span.
    ///
    /// `None` ONLY when the sidecar has no `ResolvedType` for the data
    /// AND the tile is empty/absent (whole-array recv) — the
    /// contract-gap corner the receiver classification handles fail-loud
    /// DOWNSTREAM at the assign site, not here. A non-empty (slice) tile
    /// already fails loud inside [`recv_basis`] when the type is absent,
    /// so the only way `elems == None` survives is the whole-array path.
    /// The extent consumers (MPI / embedded) only call
    /// [`extent_elems`](Self::extent_elems) / [`extent_bytes`](Self::extent_bytes)
    /// for data they have already type-guarded, so they never observe
    /// the `None`.
    elems: Option<usize>,
}

impl WireShape {
    /// Derive the single payload shape for the `(data, seq)` edge from
    /// the pair tile + sidecar type.
    ///
    /// `pair_tiles` is the per-`(DataId, SeqTag)` iteration tile both
    /// endpoints carry (TASK-0117 / `collect_pair_tiles`); an absent or
    /// empty tile is the whole-array case. This is the ONE place the
    /// whole-array-vs-slice classification is computed; every sender
    /// site and [`super::wait::render_wait_assign`] consume the result.
    ///
    /// Fails LOUD ([`EmitError::ContractGap`]) when a NON-EMPTY (slice)
    /// tile's axis ranges are out of bounds for the data dims, or when a
    /// non-empty tile references a data symbol the sidecar has no
    /// `ResolvedType` for. A WHOLE-ARRAY edge (empty/absent tile)
    /// deliberately does NOT require the type at derivation time — this
    /// preserves the exact pre-TASK-0455.07 `wait_slice` ordering (the
    /// `tile.bounds.first()` empty-tile early-return preceded the sidecar
    /// lookup), so `is_whole_array_recv` classifies an empty-tile fan-in
    /// even on a type-absent data symbol; the missing type then surfaces
    /// as a LOUD `ContractGap` DOWNSTREAM in `render_wait_assign` (pinned
    /// by `tests/whole_array_classifier.rs::
    /// accumulate_classifies_empty_tile_even_when_data_type_absent`).
    pub fn derive(
        sidecar: &NameSidecar,
        pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
        data: DataId,
        seq: SeqTag,
    ) -> Result<WireShape, EmitError> {
        Self::from_tile(sidecar, data, pair_tiles.get(&(data, seq)))
    }

    /// Derive the payload shape from an already-resolved single tile
    /// (or its absence). Same derivation as [`WireShape::derive`]; this
    /// constructor exists for the sibling collect-pass consumers
    /// (`super::collect`) that hold a `&IterTile` directly rather than
    /// the per-`(DataId, SeqTag)` map. `tile == None` is the
    /// whole-array case (no enclosing iteration nest).
    ///
    /// Ordering is load-bearing (see [`WireShape::derive`] doc): the
    /// `recv_basis` empty-tile/no-tile early-return precedes the sidecar
    /// type lookup, exactly as the pre-TASK-0455.07 `wait_slice` did, so
    /// a whole-array edge on a type-absent data symbol is classified
    /// whole-array WITHOUT erroring here.
    pub fn from_tile(
        sidecar: &NameSidecar,
        data: DataId,
        tile: Option<&IterTile>,
    ) -> Result<WireShape, EmitError> {
        // `recv_basis` looks up the type INTERNALLY, but only AFTER the
        // empty-tile early-return — so an empty/absent tile yields
        // `None` (whole-array) even when the type is absent. A non-empty
        // tile fails loud inside `recv_basis` on a missing type.
        let recv_basis = match tile {
            Some(tile) => recv_basis(sidecar, data, tile)?,
            None => None,
        };
        // The whole-array element extent is best-effort: present when the
        // sidecar carries the type, `None` on the type-absent whole-array
        // corner above. Extent consumers are already type-guarded.
        let elems = sidecar.data_type(data).map(extent_of);
        Ok(WireShape { recv_basis, elems })
    }

    /// `true` iff this edge's receiver does a WHOLE-ARRAY assign (the
    /// `recv_basis == None` case) — i.e. NOT a slice-paste. The sibling
    /// `super::collect` accumulator / let-at-wait classifiers consume
    /// this to decide whether a Wait is whole-array (TASK-0349 cycle
    /// 220). Was the module-private `super::wait::is_whole_array_recv`
    /// before TASK-0455.07 routed it through the one derivation.
    pub fn is_whole_array(&self) -> bool {
        self.recv_basis.is_none()
    }

    /// The in-process `.push(<expr>)` payload expression for a value
    /// named `name`.
    ///
    /// WHOLE-ARRAY today (`{name}.clone()`) regardless of
    /// [`recv_basis`](Self::recv_basis) — TASK-0453.22 narrows this to
    /// the `recv_basis` slice. Consumed by the shared walker's
    /// `Event::Push` arm (`super::event_walker`).
    pub fn sender_value_expr(&self, name: &str) -> String {
        // TASK-0455.07: whole-symbol clone preserved byte-identical.
        // TASK-0453.22 flips this to the recv_basis sub-slice.
        format!("{name}.clone()")
    }

    /// The wire-encode expression (`wire::enc_*` / `wire::enc_vec(&name,
    /// …)`) for a value named `name` of type `ty`.
    ///
    /// WHOLE-ARRAY today (delegates to
    /// `crate::tcp_plan::encode::encode_expr` over the whole symbol — a
    /// `pub(crate)` helper, so this is a code span, not an intra-doc
    /// link) — TASK-0453.22 narrows this to the `recv_basis` slice.
    /// Consumed by the sync-TCP plan's `Event::Push` arm
    /// (`crate::tcp_plan::events`).
    pub fn sender_encode_expr(
        &self,
        name: &str,
        ty: &ResolvedType,
    ) -> Result<String, EmitError> {
        // TASK-0455.07: whole-symbol encode preserved byte-identical.
        // TASK-0453.22 flips this to encode only the recv_basis span.
        crate::tcp_plan::encode::encode_expr(name, ty)
    }

    /// Transmitted flat element count, or `0` if the data symbol had no
    /// `ResolvedType` in the sidecar (the type-absent whole-array
    /// corner — see the `elems` field doc). The whole-array element
    /// count today (TASK-0455.07); TASK-0453.22 narrows it to the
    /// `recv_basis` span. Consumed by MPI buffered-send sizing
    /// (`crate::mpi_plan::plan::Plan::bsend_bytes`), which only calls it
    /// for data it has already type-guarded — so the `0` fallback is
    /// never observed there (it merely avoids panicking on the
    /// structurally-unreachable contract gap).
    pub fn extent_elems(&self) -> usize {
        self.elems.unwrap_or(0)
    }

    /// The byte-length expression an embedded (tier-3) Push/Wait passes
    /// as the transport length argument for a value named `name`.
    ///
    /// WHOLE-ARRAY today: `core::mem::size_of_val(&{name})` — the size
    /// of the whole `[T; N]` local, BYTE-IDENTICAL to the
    /// pre-TASK-0455.07 embedded emit. TASK-0453.22 narrows this to the
    /// `recv_basis` span's byte length. Returned as an EXPRESSION (not a
    /// literal) so the generated no_std crate computes it at compile
    /// time, matching the embedded backend's `[T; N]` fixed-array layout
    /// where the element count is implicit in the array type. Consumed
    /// by `backends/embedded-pattern/src/render` (the PIO `link_push` /
    /// `link_recv` and the DMA `dma_link_arm` / `dma_link_recv_arm`
    /// length args).
    pub fn sender_byte_len_expr(&self, name: &str) -> String {
        // TASK-0455.07: whole-array size_of_val preserved byte-identical.
        // TASK-0453.22 flips this to the recv_basis span byte length.
        format!("core::mem::size_of_val(&{name})")
    }

    /// Transmitted payload byte count = [`extent_elems`](Self::extent_elems)
    /// × `scalar_bytes`. `scalar_bytes` is supplied by the caller (each
    /// backend already has its own scalar-width function — this helper
    /// stays width-table-agnostic so callers reuse their existing one).
    pub fn extent_bytes(&self, scalar_bytes: usize) -> usize {
        self.extent_elems().saturating_mul(scalar_bytes)
    }
}

/// Flat element count of a resolved type — `1` for a scalar, else the
/// product of dims. Shared by [`WireShape::derive`] and the sender-side
/// extent helpers; the whole-array extent THIS task.
fn extent_of(ty: &ResolvedType) -> usize {
    if ty.is_scalar() {
        1
    } else {
        ty.dims.iter().copied().product()
    }
}

/// Compute the receiver-side gather basis for a Wait's tile.
///
/// Returns:
/// - `Ok(None)` when the tile is empty OR every consulted axis covers
///   the corresponding dim's full source range — the whole-array path.
/// - `Ok(Some(RecvBasis::Flat { .. }))` for the 1D leading-axis
///   slice-paste (TASK-0117).
/// - `Ok(Some(RecvBasis::Rows { .. }))` for the 2D row-loop slice-paste
///   (TASK-0294), fired iff the tile has rank >= 2 AND the data has dim
///   rank >= 2.
/// - `Ok(Some(RecvBasis::NestedRows { .. }))` for the N-D nested-loop
///   slice-paste (TASK-0341.02.02.01.01).
/// - `Err` on a shape mismatch — a tile axis range exceeding the
///   corresponding dim length, an empty range, or a negative start.
///   These are compiler-pass invariant violations worth failing loud
///   rather than silently emitting an out-of-bounds slice.
///
/// # AXIS-MAPPING ASSUMPTION (discharged TASK-0302; consult upstream guarantee)
///
/// Assumes `tile.bounds[i].iter_var` maps to data dim `i` (the
/// row-major / nest-order convention). The convention is upstream-
/// enforced by `transfer_inject::compute_partition_bounds_with_dim_prefix`
/// (TASK-0302, cycle 121): it consults the per-data, per-dim iv indexing
/// map and emits bounds in *data-dim* order, dropping any data symbol
/// whose partition-covered dims do not form a contiguous prefix from
/// dim 0 to whole-array (empty bounds). See the lineage in the
/// historical `wait_slice` docstring (moved here from `super::wait`
/// unchanged by TASK-0455.07).
///
/// Open shapes (not currently in the e2e matrix): a halo-bearing data
/// symbol indexed `[k][j]` while the partition pair is `(outer=i,
/// inner=j)`; an inner-axis-leading partition (`partition=blocks2d(j,
/// i)`); a non-row-major data layout. The dim-prefix logic assumes
/// dim 0 comes first.
///
/// # Type-lookup ordering (load-bearing, TASK-0455.07)
///
/// The empty-tile early-return precedes the sidecar `data_type` lookup,
/// preserving the exact pre-TASK-0455.07 `wait_slice` ordering: an empty
/// tile classifies whole-array (`Ok(None)`) EVEN when the data has no
/// `ResolvedType`. A NON-EMPTY tile then fails loud on a missing type
/// (the slice arithmetic genuinely needs the dims). Pinned by
/// `tests/whole_array_classifier.rs::
/// accumulate_classifies_empty_tile_even_when_data_type_absent`.
fn recv_basis(
    sidecar: &NameSidecar,
    data: DataId,
    tile: &IterTile,
) -> Result<Option<RecvBasis>, EmitError> {
    // Empty tile -> no per-axis slicing. (Precedes the type lookup, so
    // a type-absent whole-array edge classifies whole WITHOUT erroring.)
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

    // 2D row-loop path (TASK-0294): fires iff tile has 2+ axes AND the
    // data has 2+ dims. The inner axis is `tile.bounds[1]`, assumed to
    // map to `ty.dims[1]`.
    //
    // N-D nested-loop dispatch (TASK-0341.02.02.01.01, cycle 213): a
    // tile/data shape with rank >= 3 (or a rank-2 tile against rank
    // >= 3 data) is routed to `nd_banded_basis`. A whole-array rank-N
    // tile collapses to `Ok(None)` there (every axis full). Shapes the
    // N-D path does NOT yet support (multiple banded axes at rank >= 3)
    // still fail LOUD inside `nd_banded_basis`.
    if tile.bounds.len() >= 3 || (tile.bounds.len() >= 2 && ty.dims.len() > 2) {
        return nd_banded_basis(data, &ty.dims, &tile.bounds);
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
        return Ok(Some(RecvBasis::Rows {
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
    Ok(Some(RecvBasis::Flat {
        lo: (leading_range.start as usize).saturating_mul(stride),
        hi: (leading_range.end as usize).saturating_mul(stride),
    }))
}

/// N-D nested-loop slice-paste basis dispatch (TASK-0341.02.02.01.01,
/// cycle 213). The tile carries positional `(iv, range)` bounds with
/// the convention `bounds[i] ↔ dims[i]` (row-major nest order). Each
/// axis is classified full (range covers `0..dims[i]`) or banded (a
/// strict sub-range).
///
/// Supported shape: ANY number of FULL leading axes + exactly ONE
/// BANDED axis + ANY number of FULL trailing axes. This is the
/// `partition=rows(y)` write-band shape on a `[t][y][x]` (or deeper)
/// cumulative array.
///
/// - All axes full ⇒ `Ok(None)` (whole-array assign).
/// - Exactly one banded axis ⇒ `Ok(Some(NestedRows { .. }))`.
/// - Two or more banded axes ⇒ `Err(ContractGap)` (no shipped schedule
///   constructs a multi-banded rank >= 3 tile).
/// - Tile rank != data dim rank ⇒ `Err(ContractGap)`.
/// - Any axis range out of bounds / empty ⇒ `Err(ContractGap)`.
///
/// Moved verbatim from the historical `super::wait::nd_banded_slice`
/// by TASK-0455.07 (only the return variant renamed `WaitSlice` →
/// `RecvBasis`).
fn nd_banded_basis(
    data: DataId,
    dims: &[usize],
    bounds: &[(IterVar, std::ops::Range<i64>)],
) -> Result<Option<RecvBasis>, EmitError> {
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
    Ok(Some(RecvBasis::NestedRows {
        leading,
        band_lo_off,
        band_hi_off,
    }))
}
