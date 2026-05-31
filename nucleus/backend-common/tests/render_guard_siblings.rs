//! Render-layer fail-loud coverage for the FIVE defense-in-depth
//! SIBLINGS of the gather-index-load guards pinned by TASK-0374
//! (`render_gather_negative.rs`). TASK-0379, rigour follow-up to the
//! native-gather landing TASK-0341.03.01 (architect P3.1).
//!
//! WHY THIS EXISTS: TASK-0374 unit-pinned the four fail-loud arms of
//! `render_gather_index_load`. Three OTHER render entry points carry
//! fail-loud guards that had no unit test:
//!   - `render_int_expr`   — the `IrExpr::Call` arm (expr.rs:72-74);
//!   - `render_const_expr` — the `IrExpr::DataRef | Call` loop-bound
//!     arm (expr.rs:201-203);
//!   - `render_flat_index` — its OWN three guards (fire.rs:519-522
//!     empty, :529-536 missing-ResolvedType, :538-544 rank mismatch).
//!
//! REACHABILITY — be honest about it. These are DEFENSE-IN-DEPTH and,
//! UNLIKE TASK-0374's partial-rank arm (which IS source-reachable —
//! `x[col[i]]` with a 2D `col` lowers fine), they are mostly NOT
//! reachable from valid source today:
//!   - a kernel call in index position is rejected at lowering
//!     (`lower_index_expr`), so `render_int_expr`'s `Call` arm never
//!     sees a `Call` from a real program;
//!   - a `DataRef`/`Call` in loop-bound position is rejected at
//!     lowering (`allow_gather = false` for bounds), so
//!     `render_const_expr`'s arm is unreachable from real source;
//!   - `render_flat_index`'s three guards sit BEHIND shape-valid
//!     callers (every production caller passes a non-empty, shape-
//!     resolved, rank-matched slice), so they fire only on a
//!     contract-violating IR a future refactor might construct.
//!
//! Their VALUE is therefore not "catch a user error" but "pin the
//! guard BEHAVIOUR so a future refactor cannot silently delete a
//! guard and re-introduce a wrong-offset / latent-panic emission".
//! Each test names the exact `file:line` guard it pins.
//!
//! Fixture style mirrors `render_gather_negative.rs` /
//! `fire_args_nostd.rs`: build a minimal `(NameTables, NameSidecar)`
//! and opt IN to exactly the table entries each arm's reachability
//! ordering requires (see the `render_flat_index` tests — the
//! ordering of the empty / data_name / ResolvedType / rank checks is
//! load-bearing).

use nucleus_compiler::algo::{IndexedRef, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, DataSlice};
use nucleus_compiler::name_tables::NameTables;
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::render::{
    render_const_expr, render_flat_index, render_int_expr, EmitError, RenderCtx,
};

/// A `(NameTables, NameSidecar)` carrying ZERO data symbols. Each test
/// then opts IN to exactly the entries its arm's reachability ordering
/// requires, so a missing name / type is a deliberate fixture choice.
fn empty_fixtures() -> (NameTables, NameSidecar) {
    (NameTables::default(), NameSidecar::default())
}

/// A `(NameTables, NameSidecar)` where data `did` is NAMED `name` and
/// has a resolved shape `dims` (scalar `i32`).
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

/// Convenience: an `i32`-valued ident index expression.
fn ident(n: &str) -> IrExpr {
    IrExpr::Ident(n.to_string())
}

// --------------------------------------------------------------------
// render_int_expr — the `IrExpr::Call` arm (expr.rs:72-74)
// --------------------------------------------------------------------

#[test]
fn int_expr_kernel_call_in_index_is_unsupported_feature() {
    // expr.rs:72-74. A kernel call `f(k)` appearing where an integer
    // index expression is expected is rejected fail-loud. NOT
    // source-reachable: `lower_index_expr` rejects a Call in index
    // position before codegen ever sees it. The check is independent
    // of the tables, so empty fixtures suffice.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "f".to_string(),
        args: vec![ident("k")],
    };

    let err = render_int_expr(&expr, &ctx)
        .expect_err("a kernel call in integer-index position must fail loud (expr.rs:72-74)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("kernel call") && msg.contains("index"),
                "expr.rs:72-74 message must name a kernel call in an index expression: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a Call-in-index, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// render_const_expr — the `DataRef | Call` loop-bound arm
// (expr.rs:201-203)
// --------------------------------------------------------------------

#[test]
fn const_expr_data_ref_in_loop_bound_is_unsupported_feature() {
    // expr.rs:201-203. A data read `n[k]` used as a loop BOUND is
    // rejected fail-loud (a bound must be a static const expression).
    // NOT source-reachable: loop bounds lower with `allow_gather =
    // false`, so a DataRef never reaches this renderer from real
    // source. Tables irrelevant — the arm matches on the IR shape.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::DataRef(IndexedRef {
        name: "n".to_string(),
        indices: vec![ident("k")],
    });

    let err = render_const_expr(&expr, &ctx)
        .expect_err("a data-ref in loop-bound position must fail loud (expr.rs:201-203)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("const expression") && msg.contains("loop bound"),
                "expr.rs:201-203 message must name a const/loop-bound context: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a DataRef-in-loop-bound, got {other:?}"),
    }
}

#[test]
fn const_expr_kernel_call_in_loop_bound_is_unsupported_feature() {
    // expr.rs:201-203, the `Call` half of the SAME arm (bonus). A
    // kernel call as a loop bound hits the identical fail-loud path as
    // the DataRef case — pinned separately so a future refactor that
    // splits the combined `DataRef | Call` pattern cannot drop one half
    // silently.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "f".to_string(),
        args: vec![ident("k")],
    };

    let err = render_const_expr(&expr, &ctx)
        .expect_err("a kernel call in loop-bound position must fail loud (expr.rs:201-203)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("const expression") && msg.contains("loop bound"),
                "expr.rs:201-203 message must name a const/loop-bound context: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a Call-in-loop-bound, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// render_flat_index — its own three guards (fire.rs:519/529/538)
// --------------------------------------------------------------------

#[test]
fn flat_index_empty_indices_is_unsupported_feature() {
    // fire.rs:519-522. A non-indexed (whole-array) slice has no flat
    // offset. This is the FIRST check, before any table lookup, so
    // empty fixtures hit it.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: DataId(0),
        indices: vec![], // non-indexed reference
    };

    let err = render_flat_index(&slice, &ctx).expect_err(
        "render_flat_index on a non-indexed reference must fail loud (fire.rs:519-522)",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("non-indexed reference"),
                "fire.rs:519-522 message must name the non-indexed reference: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for an empty-indices slice, got {other:?}"),
    }
}

#[test]
fn flat_index_missing_resolved_type_is_contract_gap() {
    // fire.rs:529-536. REACHABILITY ORDERING (load-bearing):
    //   * indices.len() == 1 returns Ok EARLY at fire.rs:524-526 — so
    //     we MUST use >= 2 indices to get past it;
    //   * fire.rs:528 calls `data_name`, which itself ContractGaps
    //     ("has no name in NameTables") FIRST if the name is absent —
    //     so the data NAME must be PRESENT;
    //   * the ResolvedType must be ABSENT to hit THIS guard.
    // So: name present in NameTables.data, NO entry in
    // sidecar.data_types.
    let did = DataId(0);
    let mut names = NameTables::default();
    names.data.insert(did, "grid".to_string());
    let sidecar = NameSidecar::default(); // deliberately NO data_types entry
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")], // rank 2 → past the len==1 early return
    };

    let err = render_flat_index(&slice, &ctx).expect_err(
        "a rank>=2 slice over data with no ResolvedType must fail loud (fire.rs:529-536)",
    );
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("no ResolvedType") && msg.contains("grid"),
                "fire.rs:529-536 message must name the missing ResolvedType for `grid`: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-ResolvedType slice, got {other:?}"),
    }
}

#[test]
fn flat_index_rank_shape_mismatch_is_unsupported_feature() {
    // fire.rs:538-544. The name AND ResolvedType are present (so we
    // pass :528 and :529), the index list has >= 2 entries (so we pass
    // the :524 early return), but the index COUNT disagrees with the
    // declared rank: 2 indices over a rank-3 datum (`i32[3][3][3]`).
    // That is a sub-array, not a scalar slot, so it is rejected rather
    // than silently emitting a wrong row-major offset.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "grid", vec![3, 3, 3]); // rank 3
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")], // rank 2 index over rank-3 data
    };

    let err = render_flat_index(&slice, &ctx).expect_err(
        "a rank/shape mismatch (2 indices over rank-3 data) must fail loud (fire.rs:538-544)",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("rank/shape mismatch") && msg.contains("grid"),
                "fire.rs:538-544 message must name the rank/shape mismatch for `grid`: {msg}"
            );
            // The diagnostic carries the actual dims + index count so a
            // future shape change is attributable.
            assert!(
                msg.contains("[3, 3, 3]") && msg.contains("indices=2"),
                "fire.rs:538-544 message must name the dims and index count: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a rank/shape mismatch, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// Positive controls — the guards must NOT reject valid shapes
// --------------------------------------------------------------------

#[test]
fn flat_index_full_rank_renders_row_major_positive_control() {
    // POSITIVE CONTROL for the rank>=2 path: `grid[y][x]` over an
    // `i32[3][4]` datum is FULL-RANK, so the three guards pass and the
    // shared flattener emits the row-major stride form `(y*4 + x) as
    // usize` (D1 = 4 for the first axis, stride 1 for the last). The
    // exact string is snapshot-pinned so a future flattener change is
    // caught here, not silently in e2e.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "grid", vec![3, 4]);
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")],
    };

    let rendered =
        render_flat_index(&slice, &ctx).expect("a full-rank rank-2 slice must render Ok");
    assert_eq!(
        rendered, "((y) * 4 + (x)) as usize",
        "full-rank rank-2 `grid[y][x]` over i32[3][4] must render the row-major flat offset"
    );
}

#[test]
fn flat_index_missing_name_is_contract_gap() {
    // BONUS: the `data_name` guard (fire.rs:31, hit via :528). A rank-2
    // slice whose DataId is absent from NameTables.data fails loud at
    // the name lookup BEFORE the ResolvedType check — pinning the
    // ordering nuance documented on the missing-ResolvedType test.
    let (names, sidecar) = empty_fixtures(); // no name for DataId(0)
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: DataId(0),
        indices: vec![ident("y"), ident("x")], // rank 2 → reaches data_name at :528
    };

    let err = render_flat_index(&slice, &ctx)
        .expect_err("a rank>=2 slice whose DataId has no name must fail loud (fire.rs:31)");
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("has no name in NameTables"),
                "fire.rs:31 message must name the missing NameTables entry: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-name slice, got {other:?}"),
    }
}
