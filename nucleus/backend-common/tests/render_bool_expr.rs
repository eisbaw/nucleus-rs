//! Unit coverage for `render_bool_expr` — the bool render path for the
//! `for..until` early-exit predicate (epic S4, TASK-0341.02.01.05.04).
//!
//! `render_bool_expr` is the bool DUAL of the integer renderers
//! `render_int_expr` / `render_const_expr`: a relational comparison
//! [`IrExpr::Compare`] is the ONLY shape it accepts (it renders the Rust
//! bool expression `(<lhs> <op> <rhs>)`), whereas the integer renderers
//! REJECT a `Compare` (an integer index / loop bound must not be a bool).
//! The two are duals, not duplicates; this file pins:
//!
//!   1. each `IrCmpOp` variant maps to the right Rust operator;
//!   2. the operands are rendered through the scalar VALUE renderer
//!      (`render_int_expr`): an `Ident` -> the bare scalar variable, an
//!      `IntLit` -> the literal, a `BinOp` -> a parenthesised arithmetic
//!      expression, a gather `DataRef` -> a runtime load;
//!   3. the NEGATIVE arm bites: a non-`Compare` top-level expression in
//!      bool position is a typed `EmitError`, NOT a panic and NOT a
//!      silent acceptance (a bool position must be a `Compare` today).
//!
//! Fixture style mirrors `render_guard_siblings.rs`: a minimal
//! `(NameTables, NameSidecar)` and opt-in entries per test.

use nucleus_compiler::algo::{IndexedRef, IrBinOp, IrCmpOp, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::DataId;
use nucleus_compiler::name_tables::NameTables;
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::render::{render_bool_expr, EmitError, RenderCtx};

fn empty_fixtures() -> (NameTables, NameSidecar) {
    (NameTables::default(), NameSidecar::default())
}

/// A `(NameTables, NameSidecar)` where data `did` is NAMED `name` with a
/// resolved scalar `i32` shape `dims` (for the gather-operand test).
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

fn ident(n: &str) -> IrExpr {
    IrExpr::Ident(n.to_string())
}

fn compare(op: IrCmpOp, l: IrExpr, r: IrExpr) -> IrExpr {
    IrExpr::Compare(op, Box::new(l), Box::new(r))
}

// --------------------------------------------------------------------
// Positive: a scalar convergence Compare renders the Rust bool form.
// --------------------------------------------------------------------

#[test]
fn bool_expr_scalar_convergence_compare_renders_le() {
    // The 16-jacobi convergence shape: `max_abs_diff < epsilon`, two
    // runtime SCALAR values. Each operand is a bare scalar ident, so it
    // renders verbatim; the whole thing is the parenthesised bool
    // `(max_abs_diff <= epsilon)`.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = compare(IrCmpOp::Le, ident("max_abs_diff"), ident("epsilon"));
    let rendered = render_bool_expr(&expr, &ctx).expect("a relational Compare must render in bool position");
    assert_eq!(
        rendered, "(max_abs_diff <= epsilon)",
        "a scalar `max_abs_diff <= epsilon` Compare must render the Rust bool form"
    );
}

#[test]
fn bool_expr_each_cmp_op_maps_to_the_right_rust_operator() {
    // Exhaustive over all six IrCmpOp variants -> the EXACT Rust spelling.
    // A future operator addition (no `_ =>` in cmp_op_str) is compiler-
    // forced; this pins the existing six map correctly.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let cases = [
        (IrCmpOp::Le, "(a <= b)"),
        (IrCmpOp::Lt, "(a < b)"),
        (IrCmpOp::Eq, "(a == b)"),
        (IrCmpOp::Ne, "(a != b)"),
        (IrCmpOp::Gt, "(a > b)"),
        (IrCmpOp::Ge, "(a >= b)"),
    ];
    for (op, expected) in cases {
        let expr = compare(op, ident("a"), ident("b"));
        let rendered =
            render_bool_expr(&expr, &ctx).expect("every IrCmpOp must render in bool position");
        assert_eq!(rendered, expected, "IrCmpOp {op:?} must render `{expected}`");
    }
}

#[test]
fn bool_expr_operands_route_through_the_scalar_value_renderer() {
    // The operands are RUNTIME VALUES, rendered via `render_int_expr`.
    // A literal, an arithmetic binop, and a full-rank gather DataRef each
    // render through their `render_int_expr` arm — proving the operands
    // are value-rendered (NOT a separate ad-hoc path). Here:
    //   lhs = `acc[i]` (a runtime scalar load -> gather arm)
    //   rhs = `(threshold + 1)` (binop arm)
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "acc", vec![8]);
    let ctx = RenderCtx::new(&names, &sidecar);

    let lhs = IrExpr::DataRef(IndexedRef {
        name: "acc".to_string(),
        indices: vec![ident("i")],
    });
    let rhs = IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(ident("threshold")),
        Box::new(IrExpr::IntLit(1)),
    );
    let expr = compare(IrCmpOp::Gt, lhs, rhs);

    let rendered = render_bool_expr(&expr, &ctx)
        .expect("a Compare over a gather-load lhs and a binop rhs must render");
    assert_eq!(
        rendered, "(acc[(i) as usize] > (threshold + 1))",
        "operands must route through render_int_expr (gather-load lhs + binop rhs)"
    );
}

// --------------------------------------------------------------------
// Negative: a non-Compare top-level expr in bool position fails loud.
// --------------------------------------------------------------------

#[test]
fn bool_expr_non_compare_is_typed_error_not_panic() {
    // A bare integer ident in bool position is a lowering-layer contract
    // violation (the only bool the language admits is a single relational
    // comparison). The guard must BITE with a typed EmitError, not panic
    // and not silently accept. Pins the fail-loud negative arm.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let err = render_bool_expr(&ident("x"), &ctx)
        .expect_err("a bare ident in bool position must fail loud (non-Compare guard)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("bool") && msg.contains("relational comparison"),
                "the non-Compare bool-position message must name a bool / relational context: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a non-Compare bool expr, got {other:?}"),
    }
}

#[test]
fn bool_expr_intlit_in_bool_position_is_typed_error() {
    // A second non-Compare shape (an integer literal) hits the SAME
    // fail-loud arm — pinned separately so a future refactor that special-
    // cases one non-Compare shape cannot silently admit another.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let err = render_bool_expr(&IrExpr::IntLit(7), &ctx)
        .expect_err("an integer literal in bool position must fail loud");
    assert!(
        matches!(err, EmitError::UnsupportedFeature(_)),
        "a non-Compare bool expr must be a typed UnsupportedFeature, got {err:?}"
    );
}
