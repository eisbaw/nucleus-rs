//! Render-layer fail-loud coverage for the data-dependent (gather)
//! index load — `render_gather_index_load` (backend-common/src/render/
//! expr.rs:88-138, reached via the `pub` `render_int_expr` on an
//! `IrExpr::DataRef` in integer-index position). TASK-0374, rigour
//! follow-up to the native-gather landing TASK-0341.03.01.
//!
//! WHY THIS EXISTS: `render_gather_index_load` has FOUR fail-loud
//! paths, and the 7 shipped e2e gather cells (17-spmv/gather across
//! the tier-1 + tier-3 backends) exercise ONLY the full-rank happy
//! path. None of the four error arms had a unit test. The
//! partial-rank arm in particular is SOURCE-REACHABLE: `x[col[i]]`
//! where `col` is a 2D array lowers fine (lowering does not rank-check
//! the inner ref — see `lower_index_expr`'s `allow_gather`), so this
//! render guard at expr.rs:119-128 is the SOLE defense against
//! emitting a wrong (sub-array-start) offset for a partial-rank gather
//! index. The other three arms (empty-indices, missing-DataId,
//! missing-ResolvedType) are contract / structural guards.
//!
//! Each test pins ONE arm by EmitError variant + a load-bearing
//! message substring, and names the expr.rs line being pinned. The
//! positive control proves the guards reject ONLY the bad shapes: a
//! full-rank rank-1 `col[k]` still renders the unchanged
//! `col[(k) as usize]`.

use nucleus_compiler::algo::{IndexedRef, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::DataId;
use nucleus_compiler::name_tables::NameTables;
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::render::{render_int_expr, EmitError, RenderCtx};

/// Build a `(NameTables, NameSidecar)` carrying ZERO data symbols.
/// Each test then opts IN to exactly the table entries its arm needs,
/// so the absence of a `DataId` / `ResolvedType` is a deliberate
/// fixture choice rather than an accident.
fn empty_fixtures() -> (NameTables, NameSidecar) {
    (NameTables::default(), NameSidecar::default())
}

/// Build a `(NameTables, NameSidecar)` where data `did` is named
/// `name` and has resolved shape `dims` (scalar `i32`). Used by the
/// arms that need the name and/or the type present.
fn fixtures_with_data(did: DataId, name: &str, dims: Vec<usize>) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.data.insert(did, name.to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.data_types.insert(
        did,
        ResolvedType {
            scalar: ScalarType::I32,
            dims,
        },
    );
    (names, sidecar)
}

/// Convenience: the `IrExpr` for a gather index `name[indices...]`.
fn gather(name: &str, indices: Vec<IrExpr>) -> IrExpr {
    IrExpr::DataRef(IndexedRef {
        name: name.to_string(),
        indices,
    })
}

#[test]
fn empty_indices_is_unsupported_feature() {
    // Arm 1 — expr.rs:92-98. A whole-array reference `col` (NO
    // indices) used as an integer index is not a scalar load; it is
    // rejected fail-loud BEFORE any NameTables/sidecar lookup (this
    // check is first). The fixtures are therefore irrelevant — even
    // an empty table must hit this arm.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = gather("col", vec![]); // whole-array ref, no indices

    let err = render_int_expr(&expr, &ctx)
        .expect_err("a whole-array reference in index position must fail loud (expr.rs:92-98)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("whole-array reference") && msg.contains("col"),
                "expr.rs:92-98 message must name the whole-array reject and the array `col`: {msg}"
            );
            assert!(
                msg.contains("fully-indexed scalar load") || msg.contains("col[k]"),
                "expr.rs:92-98 message must explain a gather index needs a scalar load: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for an empty-indices gather, got {other:?}"),
    }
}

#[test]
fn missing_data_id_is_contract_gap() {
    // Arm 2 — expr.rs:107-112. The index `col[k]` is non-empty (so it
    // passes arm 1), but `col` is absent from NameTables.data. The
    // DataId-inversion scan finds nothing -> ContractGap. Fixtures
    // carry NO data symbol named `col`.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = gather("col", vec![IrExpr::Ident("k".to_string())]);

    let err = render_int_expr(&expr, &ctx)
        .expect_err("a gather index array absent from NameTables must fail loud (expr.rs:107-112)");
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("no DataId") && msg.contains("col"),
                "expr.rs:107-112 message must name the missing DataId for `col`: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-DataId gather, got {other:?}"),
    }
}

#[test]
fn missing_resolved_type_is_contract_gap() {
    // Arm 3 — expr.rs:113-118. `col` IS in NameTables.data (so arm 2
    // passes) but has NO ResolvedType in the sidecar. The rank check
    // needs the type, so its absence is a ContractGap. Fixtures put
    // the NAME in but leave the sidecar empty.
    let did = DataId(0);
    let mut names = NameTables::default();
    names.data.insert(did, "col".to_string());
    let sidecar = NameSidecar::default(); // deliberately NO data_types entry
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = gather("col", vec![IrExpr::Ident("k".to_string())]);

    let err = render_int_expr(&expr, &ctx).expect_err(
        "a gather index array with no sidecar ResolvedType must fail loud (expr.rs:113-118)",
    );
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("no ResolvedType") && msg.contains("col"),
                "expr.rs:113-118 message must name the missing ResolvedType for `col`: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-ResolvedType gather, got {other:?}"),
    }
}

#[test]
fn partial_rank_is_unsupported_feature() {
    // Arm 4 — expr.rs:119-128. THE SOURCE-REACHABLE one. `col` is a
    // 2D array (`i32[8][8]`, dims.len() == 2) but is indexed with a
    // SINGLE expression `col[i]` (indices.len() == 1). That is a
    // sub-array (a row), not an integer index value. Lowering does
    // NOT rank-check the inner ref, so this render guard is the sole
    // defense — without it the gather would emit a wrong
    // (row-start) offset as if it were a scalar.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "col", vec![8, 8]);
    let ctx = RenderCtx::new(&names, &sidecar);

    // `col[i]` — rank 1 index over rank-2 data: partial rank.
    let expr = gather("col", vec![IrExpr::Ident("i".to_string())]);

    let err = render_int_expr(&expr, &ctx).expect_err(
        "a partial-rank gather index (rank-1 index over rank-2 data) must fail loud \
         (expr.rs:119-128) — this is the SOLE defense; lowering does not rank-check it",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("FULL-RANK") && msg.contains("col"),
                "expr.rs:119-128 message must demand a FULL-RANK scalar load for `col`: {msg}"
            );
            // The diagnostic names the actual rank mismatch (1 index
            // vs rank 2) so a future shape change is attributable.
            assert!(
                msg.contains("rank 2") && msg.contains("1 expression"),
                "expr.rs:119-128 message must name the 1-index-vs-rank-2 mismatch: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a partial-rank gather, got {other:?}"),
    }
}

#[test]
fn full_rank_gather_renders_ok_positive_control() {
    // POSITIVE CONTROL — the happy path the guards must NOT reject.
    // `col` is a rank-1 array (`i32[8]`) indexed FULL-RANK with a
    // single scalar `col[k]`. render_gather_index_load reuses the
    // shared row-major flattener (render_flat_index): a 1-index slice
    // yields `(k) as usize`, wrapped as `col[(k) as usize]`. The exact
    // string is snapshot-pinned so a future change to the flattener or
    // the wrapping format is caught here, not silently in e2e.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "col", vec![8]);
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = gather("col", vec![IrExpr::Ident("k".to_string())]);

    let rendered = render_int_expr(&expr, &ctx)
        .expect("a full-rank rank-1 gather index `col[k]` must render Ok");
    assert_eq!(
        rendered, "col[(k) as usize]",
        "full-rank rank-1 gather `col[k]` must render the unchanged flat load"
    );
    assert!(
        rendered.contains("col["),
        "the positive-control load must subscript the index array `col`: {rendered}"
    );
}
