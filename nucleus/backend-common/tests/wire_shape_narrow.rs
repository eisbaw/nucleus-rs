//! TASK-0453.22 — wire-level precise transfer narrowing pins.
//!
//! These tests drive [`backend_common::multi_worker_walker::WireShape`]
//! directly (via `from_tile`) and pin the CONTIGUITY predicate
//! ([`WireShape::contiguous_span`]) plus the three sender helpers that
//! key off it, per [`RecvBasis`] arm. The load-bearing invariant the
//! flip rests on is: **the sender narrows to exactly the span the
//! receiver pastes from**, because both derive from the ONE `WireShape`
//! for the `(data, seq)` edge. So each test asserts the sender span the
//! helpers emit matches the receiver's destination range.
//!
//! Arm coverage:
//! - `Flat` (1D leading-axis) — NARROWED. Sender sends `name[lo..hi]`;
//!   extent = `hi - lo`.
//! - `NestedRows` with no leading axes (banded dim-0) — NARROWED. Single
//!   contiguous band `[band_lo, band_hi)`.
//! - `NestedRows` WITH leading axes (16-jacobi cumulative halo) — KEPT
//!   WHOLE-ARRAY (strided), the sound envelope.
//! - `Rows` (2D blocks2d output) — KEPT WHOLE-ARRAY (strided).
//! - whole-array / scalar — no span, whole-symbol payload.
//!
//! The receiver-side rebase (the `_tmp` from-0 index for the narrowed
//! arms) is pinned in the sibling `wait_assign_slice.rs`
//! (`flat_1d_slice_paste_for_partition_workers`) and the e2e byte-
//! identity gate; this file pins the SENDER side + the predicate.

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, IterTile, IterVar};

use backend_common::multi_worker_walker::WireShape;

mod common;
use common::Tables;

/// `i32[dims]` resolved type — the element width is 4 bytes, so a span
/// of `n` elements is `4n` bytes.
fn i32_ty(dims: Vec<usize>) -> ResolvedType {
    ResolvedType {
        scalar: ScalarType::I32,
        dims,
    }
}

#[test]
fn flat_arm_narrows_sender_to_band() {
    // 1D leading-axis band [(y, 1..8)] on i32[16, 16]. Flat arm:
    // lo = 1 * 16 = 16, hi = 8 * 16 = 128. span = 112 elems.
    let data = DataId(7);
    let (_names, sidecar) = Tables::new().with_data(data, "img_in", vec![16, 16]).build();
    let tile = IterTile::new(vec![(IterVar(1), 1..8)]);
    let wire = WireShape::from_tile(&sidecar, data, Some(&tile)).expect("derive");

    // Predicate: contiguous span is the Flat range verbatim.
    assert_eq!(
        wire.contiguous_span(),
        Some((16, 128)),
        "Flat arm must expose the contiguous span [16, 128)"
    );

    // Sender value (shared-mem .push payload): narrowed slice-to-vec.
    assert_eq!(
        wire.sender_value_expr("img_in"),
        "img_in[16usize..128usize].to_vec()",
        "Flat sender_value_expr must narrow to the band slice"
    );

    // Sender encode (sync-TCP wire): encode only the band slice.
    let enc = wire
        .sender_encode_expr("img_in", &i32_ty(vec![16, 16]))
        .expect("encode");
    assert_eq!(
        enc, "wire::enc_vec(&img_in[16usize..128usize], i32::to_le_bytes)",
        "Flat sender_encode_expr must encode only the band slice"
    );

    // Extent (MPI bsend / SO_BUF): the band element count, not 256.
    assert_eq!(
        wire.extent_elems(),
        112,
        "Flat extent_elems must be the band span (hi - lo), not whole-array 256"
    );
    assert_eq!(
        wire.extent_bytes(4),
        112 * 4,
        "Flat extent_bytes = span * scalar width"
    );
}

#[test]
fn nested_rows_no_leading_narrows_as_contiguous_band() {
    // A banded dim-0 tile with all trailing axes full: [(y, 1..3 BANDED),
    // (x, 0..8 FULL)] on i32[8, 8]. Since the banded axis IS dim 0 and
    // there are NO full LEADING axes before it, the band is a single
    // contiguous span [1*8, 3*8) = [8, 24). This routes through
    // `nd_banded_basis` (rank-2 tile on rank-2 data with a banded leading
    // axis would normally hit `Rows`, so use a rank-3 data/tile to force
    // NestedRows with empty leading — banded dim 0, two full trailing).
    let data = DataId(0);
    let (_names, sidecar) = Tables::new()
        .with_data(data, "field", vec![4, 8, 8])
        .build();
    // bounds[0] = banded dim 0 (z, 1..3), bounds[1]/[2] = full trailing.
    let tile = IterTile::new(vec![(IterVar(2), 1..3), (IterVar(3), 0..8), (IterVar(4), 0..8)]);
    let wire = WireShape::from_tile(&sidecar, data, Some(&tile)).expect("derive");

    // band_lo = 1 * (8*8) = 64, band_hi = 3 * 64 = 192. Contiguous.
    assert_eq!(
        wire.contiguous_span(),
        Some((64, 192)),
        "NestedRows with empty leading must expose the contiguous band"
    );
    assert_eq!(
        wire.sender_value_expr("field"),
        "field[64usize..192usize].to_vec()",
        "empty-leading NestedRows narrows the sender to the band"
    );
    assert_eq!(wire.extent_elems(), 128, "band span = 192 - 64");
}

#[test]
fn nested_rows_with_leading_stays_whole_array() {
    // The 16-jacobi cumulative-array halo: [(t, 0..5 FULL LEADING),
    // (y, 1..3 BANDED), (x, 0..8 FULL TRAILING)] on i32[5, 8, 8]. The
    // band recurs once per t with strided gaps, so it is NOT a single
    // contiguous slice — kept WHOLE-ARRAY (the sound envelope; whole
    // COPY is the only combine that doesn't xN-double-count the shared
    // cross-iteration history).
    let data = DataId(0);
    let (_names, sidecar) = Tables::new()
        .with_data(data, "field", vec![5, 8, 8])
        .build();
    let tile = IterTile::new(vec![(IterVar(2), 0..5), (IterVar(4), 1..3), (IterVar(3), 0..8)]);
    let wire = WireShape::from_tile(&sidecar, data, Some(&tile)).expect("derive");

    assert_eq!(
        wire.contiguous_span(),
        None,
        "NestedRows with full leading t-axis is strided => no contiguous span"
    );
    assert_eq!(
        wire.sender_value_expr("field"),
        "field.clone()",
        "strided NestedRows keeps the whole-symbol clone (sound envelope)"
    );
    assert_eq!(
        wire.extent_elems(),
        5 * 8 * 8,
        "strided NestedRows extent stays whole-array (320)"
    );
}

#[test]
fn rows_arm_stays_whole_array() {
    // 2D blocks2d output band [(y, 1..8), (x, 1..8)] on i32[16, 16] — the
    // Rows arm. Strided per-row sub-ranges => NOT contiguous => kept
    // whole-array (07-matmul/distributed-2d `c`).
    let data = DataId(7);
    let (_names, sidecar) = Tables::new().with_data(data, "c", vec![16, 16]).build();
    let tile = IterTile::new(vec![(IterVar(1), 1..8), (IterVar(2), 1..8)]);
    let wire = WireShape::from_tile(&sidecar, data, Some(&tile)).expect("derive");

    assert_eq!(
        wire.contiguous_span(),
        None,
        "Rows arm is strided => no contiguous span"
    );
    assert_eq!(
        wire.sender_value_expr("c"),
        "c.clone()",
        "Rows arm keeps the whole-symbol clone"
    );
    assert_eq!(wire.extent_elems(), 256, "Rows extent stays whole-array");
}

#[test]
fn whole_array_and_scalar_stay_whole() {
    // Empty tile => whole-array. Scalar => whole-value. Both have no
    // contiguous span (None) and keep the whole-symbol payload.
    let data = DataId(7);

    // Empty tile (top-level transfer).
    let (_n1, sc1) = Tables::new().with_data(data, "img", vec![16, 16]).build();
    let whole = WireShape::from_tile(&sc1, data, None).expect("derive whole");
    assert_eq!(whole.contiguous_span(), None, "no tile => no span");
    assert_eq!(whole.sender_value_expr("img"), "img.clone()");
    assert_eq!(whole.extent_elems(), 256, "whole-array extent");

    // Scalar (zero dims): recv_basis None, no span.
    let scalar = DataId(8);
    let (_n2, sc2) = Tables::new().with_data(scalar, "acc", vec![]).build();
    let tile = IterTile::new(vec![(IterVar(1), 0..1)]);
    let sw = WireShape::from_tile(&sc2, scalar, Some(&tile)).expect("derive scalar");
    assert_eq!(sw.contiguous_span(), None, "scalar => no span");
    assert_eq!(sw.sender_value_expr("acc"), "acc.clone()");
    assert_eq!(sw.extent_elems(), 1, "scalar extent = 1");
}

/// The cornerstone invariant: the sender's narrowed span EQUALS the
/// receiver's pasted destination range length, edge by edge. Both come
/// from the SAME `WireShape`, so they cannot diverge — this test pins
/// that they agree numerically for the narrowable `Flat` arm (the e2e
/// gate covers the runtime byte-identity end-to-end).
#[test]
fn sender_span_equals_receiver_paste_length_flat() {
    let data = DataId(7);
    let (_names, sidecar) = Tables::new().with_data(data, "x", vec![16, 16]).build();
    // Three distinct bands (worker partitions of differing widths).
    for (lo_iv, hi_iv) in [(0i64, 6i64), (4, 10), (8, 13), (11, 16)] {
        let tile = IterTile::new(vec![(IterVar(1), lo_iv..hi_iv)]);
        let wire = WireShape::from_tile(&sidecar, data, Some(&tile)).expect("derive");
        let (lo, hi) = wire.contiguous_span().expect("Flat band has a span");
        // Destination range length the receiver pastes (hi - lo) must
        // equal the transmitted extent (the narrowed payload length).
        assert_eq!(
            hi - lo,
            wire.extent_elems(),
            "sender extent ({}) must equal receiver paste length ({}) for band {:?}",
            wire.extent_elems(),
            hi - lo,
            lo_iv..hi_iv
        );
        // Wave-5 review P3.4: the equality above is extent_elems'
        // definition for a present span — also pin the RENDERED sender
        // expression text against the span, crossing derivation ->
        // emitted code (catches a sender helper drifting off the span
        // while extent_elems still agrees with it).
        assert_eq!(
            wire.sender_value_expr("x"),
            format!("x[{lo}usize..{hi}usize].to_vec()"),
            "narrowed Flat sender expression must slice exactly the \
             contiguous span for band {:?}",
            lo_iv..hi_iv
        );
    }
}
