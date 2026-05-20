//! Integration tests for the AST → AlgoIR lowering pass (TASK-0009).
//!
//! Positive cases: the non-legacy example algorithm files must lower
//! cleanly. We assert structural properties of the resulting IR rather
//! than full structural equality, mirroring the parser tests'
//! deliberately-non-snapshot approach (see TASK-0007 notes). Full-IR
//! snapshots would freeze the encoding and break on every shape tweak.
//!
//! Negative cases: each `LowerError` variant we want to defend is
//! exercised by a hand-written invalid input.
//!
//! 05-stencil was historically a known-failing parse (legacy 2013-style
//! kernel syntax). TASK-0078 / TASK-0031 rewrote it into v2 form;
//! `lowers_example_05_stencil` below pins that it lowers cleanly.

use compiler::algo::{
    lower_algo, parse_algo, AlgoIR, IrStmt, LowerError, LowerErrorKind, LowerErrors, ResolvedType,
    ScalarType,
};
use compiler::error::offset_to_line_col;

/// Reads a source file at a workspace-relative path. Panics on IO
/// failure — these tests are environment-dependent by design.
fn read_example(relpath: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let full = repo_root.join("nuc-nucleus").join("examples").join(relpath);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", full.display(), e))
}

fn lower_str(src: &str) -> Result<AlgoIR, LowerErrors> {
    let ast = parse_algo(src).expect("source must parse for this test");
    lower_algo(&ast)
}

// --------------------------------------------------------------------
// Positive cases — the examples lower
// --------------------------------------------------------------------

#[test]
fn lowers_example_01_elementwise_add() {
    // TASK-0013: the smoke-test algorithm lowers cleanly.
    let src = read_example("01-elementwise-add/prog.algo.nuc");
    let ast = parse_algo(&src).expect("01-elementwise-add must parse");
    let ir = lower_algo(&ast).expect("01-elementwise-add must lower");

    // 1 const (N), 3 data (a, b, c), 4 kernels.
    assert_eq!(ir.consts.len(), 1);
    assert_eq!(ir.data.len(), 3);
    assert_eq!(ir.kernels.len(), 4);

    // N resolves to 256.
    assert_eq!(ir.consts["N"].value, 256);

    // Each data array is a 1D i32 vector of length N.
    let a = &ir.data["a"].ty;
    assert_eq!(
        a,
        &ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![256],
        }
    );
    let c = &ir.data["c"].ty;
    assert_eq!(c.scalar, ScalarType::I32);
    assert_eq!(c.dims, vec![256]);

    // Statements: load a, load b, for-loop, save -> 4.
    assert_eq!(ir.stmts.len(), 4);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[2], IrStmt::For { .. }));
    assert!(matches!(ir.stmts[3], IrStmt::Effect { .. }));
}

#[test]
fn lowers_example_02_split_add() {
    // TASK-0021: example 02's algorithm IR matches example 01's
    // (the two examples share an algorithm shape; only the schedule
    // differs).
    let src = read_example("02-split-add/prog.algo.nuc");
    let ast = parse_algo(&src).expect("02-split-add must parse");
    let ir = lower_algo(&ast).expect("02-split-add must lower");

    assert_eq!(ir.consts.len(), 1);
    assert_eq!(ir.data.len(), 3);
    assert_eq!(ir.kernels.len(), 4);

    assert_eq!(ir.consts["N"].value, 256);

    let a = &ir.data["a"].ty;
    assert_eq!(
        a,
        &ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![256],
        }
    );
    let c = &ir.data["c"].ty;
    assert_eq!(c.scalar, ScalarType::I32);
    assert_eq!(c.dims, vec![256]);

    assert_eq!(ir.stmts.len(), 4);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[2], IrStmt::For { .. }));
    assert!(matches!(ir.stmts[3], IrStmt::Effect { .. }));
}

#[test]
fn lowers_example_03_reduction() {
    // TASK-0022: 03-reduction lowers cleanly.
    let src = read_example("03-reduction/prog.algo.nuc");
    let ast = parse_algo(&src).expect("03-reduction must parse");
    let ir = lower_algo(&ast).expect("03-reduction must lower");

    assert_eq!(ir.consts.len(), 3);
    assert_eq!(ir.data.len(), 5);
    assert_eq!(ir.kernels.len(), 4);

    // Const evaluation: PARTITION_SIZE = N / NUM_WORKERS = 64.
    assert_eq!(ir.consts["N"].value, 256);
    assert_eq!(ir.consts["NUM_WORKERS"].value, 4);
    assert_eq!(ir.consts["PARTITION_SIZE"].value, 64);

    // `a : i32[NUM_WORKERS][PARTITION_SIZE]` -> dims [4, 64].
    let a = &ir.data["a"].ty;
    assert_eq!(
        a,
        &ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![4, 64],
        }
    );
    // `partials : i32[NUM_WORKERS]` -> [4].
    let partials = &ir.data["partials"].ty;
    assert_eq!(partials.scalar, ScalarType::I32);
    assert_eq!(partials.dims, vec![4]);
    // `result : i32` (scalar) -> dims [].
    let result = &ir.data["result"].ty;
    assert_eq!(result.scalar, ScalarType::I32);
    assert!(result.dims.is_empty(), "result must be scalar");
    // `half1 : i32` (scalar).
    let half1 = &ir.data["half1"].ty;
    assert!(half1.dims.is_empty());

    // Top-level statements: load_input dataflow, outer-for, three
    // tree-combine dataflows (half1, half2, result), save_output
    // effect -> 6.
    assert_eq!(ir.stmts.len(), 6);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::For { .. }));
    assert!(matches!(ir.stmts[2], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[3], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[4], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[5], IrStmt::Effect { .. }));

    // The outer for-loop body has a single inner for-loop, whose
    // body has a single dataflow statement (the accumulate fold).
    if let IrStmt::For { ref body, .. } = ir.stmts[1] {
        assert_eq!(body.len(), 1, "outer-for body should have 1 stmt");
        match &body[0] {
            IrStmt::For { body: inner, .. } => {
                assert_eq!(inner.len(), 1, "inner-for body should have 1 stmt");
                assert!(matches!(inner[0], IrStmt::Dataflow { .. }));
            }
            other => panic!("expected nested for-loop; got {:?}", other),
        }
    } else {
        panic!("stmts[1] must be the outer For");
    }
}

#[test]
fn lowers_example_05_stencil() {
    // TASK-0031: 3x3 stencil. H=W=16, two flat images, three kernels
    // (blur3 pure, load_image / save_image effectful).
    let src = read_example("05-stencil/prog.algo.nuc");
    let ast = parse_algo(&src).expect("05-stencil must parse");
    let ir = lower_algo(&ast).expect("05-stencil must lower");

    assert_eq!(ir.consts.len(), 2, "expected 2 const decls (H, W)");
    assert_eq!(ir.data.len(), 2, "expected 2 data decls (img_in, img_out)");
    assert_eq!(ir.kernels.len(), 3, "expected 3 kernel decls");

    assert_eq!(ir.consts["H"].value, 16);
    assert_eq!(ir.consts["W"].value, 16);

    // Both images are i32[16][16].
    let img_in = &ir.data["img_in"].ty;
    assert_eq!(
        img_in,
        &ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16, 16],
        }
    );
    let img_out = &ir.data["img_out"].ty;
    assert_eq!(img_out.scalar, ScalarType::I32);
    assert_eq!(img_out.dims, vec![16, 16]);

    // Top-level statements: load_image dataflow, outer-for, save_image
    // effect -> 3.
    assert_eq!(ir.stmts.len(), 3);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::For { .. }));
    assert!(matches!(ir.stmts[2], IrStmt::Effect { .. }));

    // The outer for-loop body holds exactly one inner for-loop, whose
    // body has exactly one dataflow statement (the blur3 call).
    if let IrStmt::For { ref body, .. } = ir.stmts[1] {
        assert_eq!(body.len(), 1, "outer-for body should have 1 stmt");
        match &body[0] {
            IrStmt::For { body: inner, .. } => {
                assert_eq!(inner.len(), 1, "inner-for body should have 1 stmt");
                assert!(matches!(inner[0], IrStmt::Dataflow { .. }));
            }
            other => panic!("expected nested for-loop; got {:?}", other),
        }
    } else {
        panic!("stmts[1] must be the outer For");
    }
}

#[test]
fn lowers_example_07_matmul() {
    // TASK-0032: blocked matmul. N=16, three flat NxN matrices, four
    // kernels (madd pure; load_a/load_b/save_c effectful). Triple-
    // nested loop with reduction on the innermost axis.
    let src = read_example("07-matmul/prog.algo.nuc");
    let ast = parse_algo(&src).expect("07-matmul must parse");
    let ir = lower_algo(&ast).expect("07-matmul must lower");

    assert_eq!(ir.consts.len(), 1, "expected 1 const decl (N)");
    assert_eq!(ir.data.len(), 3, "expected 3 data decls (a, b, c)");
    assert_eq!(ir.kernels.len(), 4, "expected 4 kernel decls");

    assert_eq!(ir.consts["N"].value, 16);

    for name in ["a", "b", "c"] {
        let ty = &ir.data[name].ty;
        assert_eq!(
            ty,
            &ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16, 16],
            },
            "data `{}` should be i32[16][16]",
            name
        );
    }

    // Top-level statements: load_a dataflow, load_b dataflow,
    // outer-for, save_c effect -> 4.
    assert_eq!(ir.stmts.len(), 4);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[2], IrStmt::For { .. }));
    assert!(matches!(ir.stmts[3], IrStmt::Effect { .. }));

    // Drill into the three-deep loop nest. for i / for j / for k,
    // each body holding exactly one statement, the innermost being
    // the madd dataflow.
    if let IrStmt::For { ref body, .. } = ir.stmts[2] {
        assert_eq!(body.len(), 1, "outer-for body should have 1 stmt");
        match &body[0] {
            IrStmt::For { body: mid, .. } => {
                assert_eq!(mid.len(), 1, "middle-for body should have 1 stmt");
                match &mid[0] {
                    IrStmt::For { body: inner, .. } => {
                        assert_eq!(inner.len(), 1, "inner-for body should have 1 stmt");
                        assert!(matches!(inner[0], IrStmt::Dataflow { .. }));
                    }
                    other => panic!("expected inner for-loop; got {:?}", other),
                }
            }
            other => panic!("expected middle for-loop; got {:?}", other),
        }
    } else {
        panic!("stmts[2] must be the outer For");
    }
}

#[test]
fn lowers_example_13_cnn_inference() {
    let src = read_example("13-cnn-inference/prog.algo.nuc");
    let ast = parse_algo(&src).expect("13-cnn-inference must parse");
    let ir = lower_algo(&ast).expect("13-cnn-inference must lower");

    // 7 consts, 4 data, 5 kernels (matches parser test counts).
    assert_eq!(ir.consts.len(), 7);
    assert_eq!(ir.data.len(), 4);
    assert_eq!(ir.kernels.len(), 5);

    // Spot-check resolved const values.
    assert_eq!(ir.consts["B"].value, 16);
    assert_eq!(ir.consts["H"].value, 28);
    assert_eq!(ir.consts["W"].value, 28);
    assert_eq!(ir.consts["N_CLASSES"].value, 10);

    // feat1 : f32[B][C1][H/2][W/2] should resolve to [16, 8, 14, 14].
    let feat1 = &ir.data["feat1"].ty;
    assert_eq!(
        feat1,
        &ResolvedType {
            scalar: ScalarType::F32,
            dims: vec![16, 8, 14, 14],
        }
    );
    // feat2 : f32[B][C2][H/4][W/4] should resolve to [16, 16, 7, 7].
    let feat2 = &ir.data["feat2"].ty;
    assert_eq!(feat2.dims, vec![16, 16, 7, 7]);
    // output : f32[B][N_CLASSES] -> [16, 10].
    assert_eq!(ir.data["output"].ty.dims, vec![16, 10]);

    // Statements: load_input dataflow, for-loop, save_output effect.
    assert_eq!(ir.stmts.len(), 3);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::For { .. }));
    assert!(matches!(ir.stmts[2], IrStmt::Effect { .. }));

    // Inside the for loop, three dataflow statements.
    if let IrStmt::For { ref body, .. } = ir.stmts[1] {
        assert_eq!(body.len(), 3);
        for s in body {
            assert!(matches!(s, IrStmt::Dataflow { .. }));
        }
    } else {
        panic!("stmts[1] must be a For");
    }
}

#[test]
fn lowers_example_14_hearing_aid() {
    let src = read_example("14-hearing-aid/prog.algo.nuc");
    let ast = parse_algo(&src).expect("14-hearing-aid must parse");
    let ir = lower_algo(&ast).expect("14-hearing-aid must lower");

    assert_eq!(ir.consts.len(), 2);
    assert_eq!(ir.data.len(), 4);
    assert_eq!(ir.kernels.len(), 6);
    assert_eq!(ir.stmts.len(), 1);

    // mic_in : f32[N_FRAMES][SAMPLES_PER_FRAME] = f32[1000][256].
    assert_eq!(ir.data["mic_in"].ty.dims, vec![1000, 256]);

    // The single top-level statement is a `for frame : 0 .. N_FRAMES`
    // with six body statements (two captures, two outbound, two
    // inbound).
    if let IrStmt::For {
        ref var, ref body, ..
    } = ir.stmts[0]
    {
        assert_eq!(var, "frame");
        assert_eq!(body.len(), 6);
        // Four dataflow stmts + two effect stmts (rf_transmit and
        // fe_emit are bare calls).
        let n_dataflow = body
            .iter()
            .filter(|s| matches!(s, IrStmt::Dataflow { .. }))
            .count();
        let n_effect = body
            .iter()
            .filter(|s| matches!(s, IrStmt::Effect { .. }))
            .count();
        assert_eq!(n_dataflow, 4);
        assert_eq!(n_effect, 2);
    } else {
        panic!("stmts[0] must be a For");
    }
}

// --------------------------------------------------------------------
// Negative cases
// --------------------------------------------------------------------

#[test]
fn duplicate_kernel_name_is_error() {
    let src = "\
kernel k : () -> () effectful;
kernel k : () -> () effectful;
";
    // Multi-error migration (TASK-0092): `lower_str` now yields
    // `LowerErrors`. `.first()` is the source-order-earliest error —
    // exactly the single error the pre-multi-error `?`-bail pass would
    // have returned — so the discriminating match below is unchanged
    // and assertion strength is preserved (no blanket `len()` assert).
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::DuplicateKernel(n),
            ..
        }) => assert_eq!(n, "k"),
        other => panic!("expected DuplicateKernel; got {other:?}"),
    }
}

#[test]
fn duplicate_data_name_is_error() {
    let src = "\
const N : usize = 4;
data x : f32[N];
data x : f32[N];
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::DuplicateData(n),
            ..
        }) => assert_eq!(n, "x"),
        other => panic!("expected DuplicateData; got {other:?}"),
    }
}

#[test]
fn duplicate_const_name_is_error() {
    let src = "\
const N : usize = 4;
const N : usize = 8;
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::DuplicateConst(n),
            ..
        }) => assert_eq!(n, "N"),
        other => panic!("expected DuplicateConst; got {other:?}"),
    }
}

#[test]
fn const_and_data_share_namespace() {
    // PRD §6.2.3: identifiers share one namespace at the algorithm
    // level. `N` declared first as a const then as data must error.
    let src = "\
const N : usize = 4;
data N : f32[4];
";
    // `.first().clone()` = the source-order-earliest error, owned —
    // exactly the single error the pre-multi-error pass returned
    // (TASK-0092 migration; assertion strength preserved).
    let err = lower_str(src).expect_err("must error").first().clone();
    // The variant is DuplicateData (the second declaration); the
    // important thing is the collision is detected.
    assert!(
        matches!(err.kind, LowerErrorKind::DuplicateData(ref n) if n == "N"),
        "expected DuplicateData(N); got {err:?}"
    );
}

#[test]
fn double_assignment_to_same_data_is_error() {
    // Two dataflow statements with the same LHS data symbol.
    let src = "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel src : () -> f32[N] effectful;
kernel f   : (f32[N]) -> f32[N] pure;
x <-- src();
x <-- f(y);
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::DoubleAssignment { data, .. },
            ..
        }) => assert_eq!(data, "x"),
        other => panic!("expected DoubleAssignment; got {other:?}"),
    }
}

#[test]
fn iter_var_outside_its_loop_is_error() {
    // `y` is the loop variable inside the for; using it after the
    // for body ended must error.
    let src = "\
const H : usize = 4;
const W : usize = 4;
data img : f32[H][W];
kernel f : () -> f32 effectful;
for y : 0 .. H {
    img[y][0] <-- f();
}
data probe : f32[H];
kernel g : (f32) -> f32 pure;
probe[y] <-- g(probe[0]);
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::IterVarOutOfScope(n),
            ..
        }) => assert_eq!(n, "y"),
        other => panic!("expected IterVarOutOfScope; got {other:?}"),
    }
}

// --------------------------------------------------------------------
// Bonus negative coverage — const evaluator edges
// --------------------------------------------------------------------

#[test]
fn const_divide_by_zero_is_error() {
    let src = "\
const Z : usize = 0;
const Q : usize = 10 / Z;
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::ConstDivByZero { in_const },
            ..
        }) => assert_eq!(in_const, "Q"),
        other => panic!("expected ConstDivByZero; got {other:?}"),
    }
}

#[test]
fn const_forward_reference_is_error() {
    // `M = N` with `N` declared after must error: declarations-before-use.
    let src = "\
const M : usize = N;
const N : usize = 4;
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind:
                LowerErrorKind::ConstRefersToNonConst {
                    in_const,
                    unknown_ident,
                },
            ..
        }) => {
            assert_eq!(in_const, "M");
            assert_eq!(unknown_ident, "N");
        }
        other => panic!("expected ConstRefersToNonConst; got {other:?}"),
    }
}

#[test]
fn non_positive_shape_dim_is_error() {
    // 4 - 4 = 0 — a zero dimension is non-positive and rejected.
    let src = "\
const N : usize = 4;
data x : f32[N - 4];
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::NonPositiveDim { decl, value },
            ..
        }) => {
            assert_eq!(decl, "x");
            assert_eq!(value, 0);
        }
        other => panic!("expected NonPositiveDim; got {other:?}"),
    }
}

#[test]
fn unknown_kernel_in_dataflow_rhs_is_error() {
    let src = "\
data x : f32[1];
x <-- nope();
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::UnknownIdent(n),
            ..
        }) => assert_eq!(n, "nope"),
        other => panic!("expected UnknownIdent; got {other:?}"),
    }
}

#[test]
fn assignment_to_const_is_rejected() {
    // The LHS of `<--` must be a `data` symbol. Assigning to a const
    // is `AssignmentTargetNotData` because the name is known but the
    // wrong kind.
    let src = "\
const N : usize = 4;
kernel f : () -> f32 effectful;
N <-- f();
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::AssignmentTargetNotData(n),
            ..
        }) => assert_eq!(n, "N"),
        other => panic!("expected AssignmentTargetNotData; got {other:?}"),
    }
}

// --------------------------------------------------------------------
// TASK-0090: located lowering diagnostics — the error carries the byte
// span of the offending node, which resolves to the CORRECT line:col.
// --------------------------------------------------------------------

/// Resolve a lowered error's stored byte span to a 1-based
/// `(line, column)` against `src`, the same way the driver does
/// (`LowerError::display_with_src`). Panics if the variant is
/// position-less — every case asserted below is a single-offending-node
/// variant that MUST carry a span (AC#1), so a `None` here is a real
/// regression, not test noise.
fn err_line_col(src: &str, err: &LowerError) -> (usize, usize) {
    let span = err
        .span
        .clone()
        .unwrap_or_else(|| panic!("expected a located error, got position-less: {err:?}"));
    offset_to_line_col(src, span.start)
}

/// AC#3: representative bad programs assert the EXACT line:col, each
/// independently validated against the crafted source — the expected
/// `(line, col)` is recomputed by finding the offending token in the
/// source string and feeding that offset through `offset_to_line_col`,
/// so the test pins the real source position, not a guessed constant.
#[test]
fn located_errors_carry_correct_line_col() {
    // Case 1: duplicate const. The diagnostic must point at the
    // *second* (duplicate) `N`, on line 2.
    {
        let src = "const N : usize = 4;\nconst N : usize = 8;\n";
        let err = lower_str(src)
            .expect_err("duplicate const must error")
            .first()
            .clone();
        assert!(
            matches!(err.kind, LowerErrorKind::DuplicateConst(ref n) if n == "N"),
            "got {err:?}"
        );
        // The duplicate `N` is the second occurrence of "N " ("N :").
        let second_n = src.match_indices("N :").nth(1).expect("two `N :`").0;
        let expected = offset_to_line_col(src, second_n);
        assert_eq!(expected, (2, 7), "sanity: duplicate `N` is at line 2 col 7");
        assert_eq!(
            err_line_col(src, &err),
            expected,
            "DuplicateConst must point at the duplicate declaration's identifier"
        );
        // And the driver-facing rendering carries it.
        assert_eq!(
            err.display_with_src(src),
            "duplicate const `N` at 2:7"
        );
    }

    // Case 2: unknown identifier in a kernel-call RHS. Points at the
    // undeclared callee `nope` on line 2.
    {
        let src = "data x : f32[1];\nx <-- nope();\n";
        let err = lower_str(src)
            .expect_err("unknown ident must error")
            .first()
            .clone();
        assert!(
            matches!(err.kind, LowerErrorKind::UnknownIdent(ref n) if n == "nope"),
            "got {err:?}"
        );
        let nope_at = src.find("nope").expect("`nope` in source");
        let expected = offset_to_line_col(src, nope_at);
        assert_eq!(expected, (2, 7), "sanity: `nope` is at line 2 col 7");
        assert_eq!(
            err_line_col(src, &err),
            expected,
            "UnknownIdent must point at the undeclared reference"
        );
        assert_eq!(
            err.display_with_src(src),
            "unknown identifier `nope` at 2:7"
        );
    }

    // Case 3: double assignment. Points at the LHS of the *second*
    // (violating) dataflow statement, on line 7.
    {
        let src = "const N : usize = 4;\n\
data x : f32[N];\n\
data y : f32[N];\n\
kernel src : () -> f32[N] effectful;\n\
kernel f   : (f32[N]) -> f32[N] pure;\n\
x <-- src();\n\
x <-- f(y);\n";
        let err = lower_str(src)
            .expect_err("double assignment must error")
            .first()
            .clone();
        assert!(
            matches!(err.kind, LowerErrorKind::DoubleAssignment { ref data, .. } if data == "x"),
            "got {err:?}"
        );
        // The violating LHS is the `x` at the start of line 7 (the
        // second `x <-- `).
        let second_assign = src.match_indices("x <-- ").nth(1).expect("two `x <-- `").0;
        let expected = offset_to_line_col(src, second_assign);
        assert_eq!(expected, (7, 1), "sanity: re-assignment `x` is at line 7 col 1");
        assert_eq!(
            err_line_col(src, &err),
            expected,
            "DoubleAssignment must point at the re-assignment's LHS"
        );
        assert_eq!(
            err.display_with_src(src),
            "data `x` is assigned twice in the same scope (<top-level>); single-assignment violated at 7:1"
        );
    }
}

/// `ConstCycle` — the SOLE genuinely position-less variant — stays
/// position-less on purpose (honest-partial — see `LowerError` docs).
/// This pins that decision so a future change that silently attaches a
/// (likely wrong) span to it is caught. (The synthetic
/// `NonIntegerShapeExpr` is NOT position-less; pinning its located-ness
/// is the separate TASK-0195 coverage gap.)
#[test]
fn multi_site_variants_are_position_less() {
    // A const self-cycle: `ConstCycle` spans several decls, no single
    // primary node.
    let src = "const A : usize = A;\n";
    let err = lower_str(src)
        .expect_err("self-referential const must error")
        .first()
        .clone();
    assert!(
        matches!(err.kind, LowerErrorKind::ConstCycle(_)),
        "got {err:?}"
    );
    assert!(
        err.span.is_none(),
        "ConstCycle is documented position-less; got {:?}",
        err.span
    );
    // Display falls back to the kind alone — no fabricated location.
    assert_eq!(err.display_with_src(src), err.kind.to_string());
}

/// TASK-0195: the OTHER side of the located-vs-position-less boundary.
/// The synthetic `<index/loop-bound expression>` `NonIntegerShapeExpr`
/// (its `decl` *label* is synthetic, but the offending node is a
/// genuine source expression) MUST carry a real `Some(span)` pointing
/// at that expression — only `ConstCycle` is position-less
/// (`multi_site_variants_are_position_less` pins that). Without this
/// positive pin, a future change could silently flip the synthetic
/// variant to `None` or a wrong span and no test would bite. Here a
/// loop's upper bound is a kernel call (`f()`), illegal in a loop-bound
/// position — the `Expr::Call` arm of `lower_index_expr` raises the
/// synthetic variant located at the call's span.
#[test]
fn synthetic_non_integer_shape_expr_is_located() {
    let src = "\
const N : usize = 4;
data x : f32[N];
kernel f : () -> usize pure;
for j : 0 .. f() {
    x[j] <-- f();
}
";
    let err = lower_str(src)
        .expect_err("kernel call as loop bound must error")
        .first()
        .clone();

    // Semantic kind: the SYNTHETIC-label variant (note the decl is the
    // synthetic placeholder, not a real declaration name).
    match &err.kind {
        LowerErrorKind::NonIntegerShapeExpr { decl, reason } => {
            assert_eq!(
                decl, "<index/loop-bound expression>",
                "must be the synthetic-label NonIntegerShapeExpr (the path \
                 whose located-ness this test pins)"
            );
            assert_eq!(reason, "kernel calls are not allowed here");
        }
        other => panic!("expected synthetic NonIntegerShapeExpr, got {other:?}"),
    }

    // AC#1: it is LOCATED — `Some(span)`, NOT `None`. A `None` here is
    // a real regression (the doc in algo/ir.rs explicitly states this
    // synthetic variant carries the real `expr.span`).
    let span = err
        .span
        .clone()
        .expect("synthetic NonIntegerShapeExpr MUST be located (Some(span)), got None");

    // AC#1: the span points at the CORRECT offset. The offending node
    // is the loop upper-bound call `f()` — the FIRST `f()` in source
    // (the loop-bound one; the body's `f()` is never reached because
    // the bound errors first). Validate via `offset_to_line_col`
    // against the crafted source, not a guessed constant.
    let bound_call_at = src.find("0 .. f()").map(|i| i + "0 .. ".len()).expect(
        "loop-bound call `f()` present in crafted source",
    );
    let expected = offset_to_line_col(src, bound_call_at);
    assert_eq!(
        expected,
        (4, 14),
        "sanity: loop-bound `f()` is at line 4 col 14 in the crafted source"
    );
    assert_eq!(
        offset_to_line_col(src, span.start),
        expected,
        "synthetic NonIntegerShapeExpr must point at the loop-bound call `f()`"
    );
}

// --------------------------------------------------------------------
// TASK-0092: multi-error accumulation + cascade discipline.
//
// The project's #1 recurring defect (memory
// `feedback-comment-doc-lie-recurring`) is mis-stating the emitted
// error COUNT — measured at ONE input shape and pinned by a
// single-shape fixture that masks the real (mis)behaviour. These tests
// are deliberately SIZE-PARAMETRISED over BOTH dimensions:
//
//   - M independent bad declarations  → EXACTLY M errors.
//   - 1 failed const with N dependents → EXACTLY 1 error (no N-cascade).
//
// A single-shape fixture is itself the masking defect; iterating M and
// N is the regression that closes that class.
// --------------------------------------------------------------------

/// Dimension M — independence. `M` mutually-independent bad const
/// declarations (`const Bad{i} : usize = 1 / 0;`, each an isolated
/// `ConstDivByZero` referencing nothing) must produce EXACTLY M errors,
/// each located at its own declaration. Iterated over several M so a
/// fixed-M fixture cannot mask an off-by-one or a collapse.
#[test]
fn m_independent_bad_decls_yield_exactly_m_errors() {
    for m in [1usize, 2, 3, 5, 8] {
        // Each line: `const Bad{i} : usize = 1 / 0;` — div-by-zero is
        // independent (no identifier reference), so none is a cascade
        // of another. One source line per decl ⇒ error i is on line
        // i+1, column 1 (the `const` keyword).
        let mut src = String::new();
        for i in 0..m {
            src.push_str(&format!("const Bad{i} : usize = 1 / 0;\n"));
        }
        let errs = lower_str(&src)
            .expect_err("every Bad{i} is an independent div-by-zero");

        assert_eq!(
            errs.errors().len(),
            m,
            "M={m} independent bad decls must yield EXACTLY M errors, \
             got {} — source:\n{src}",
            errs.errors().len()
        );

        // Each error is the right kind, in source order, located at
        // its own declaration line (col 1 = the `const` keyword).
        for (i, e) in errs.errors().iter().enumerate() {
            match &e.kind {
                LowerErrorKind::ConstDivByZero { in_const } => {
                    assert_eq!(
                        in_const,
                        &format!("Bad{i}"),
                        "errors must be in source/declaration order"
                    );
                }
                other => panic!("error {i}: expected ConstDivByZero, got {other:?}"),
            }
            let span = e
                .span
                .clone()
                .expect("ConstDivByZero is a located single-node variant");
            // Validate against the SOURCE (project discipline: never a
            // guessed constant). The i-th `1 / 0` is the offending
            // expression of the i-th decl; its line must be i+1 (one
            // decl per line) and its offset must match the located
            // span — proving each error points at its OWN decl, not a
            // shared/collapsed position.
            let nth_div = src
                .match_indices("1 / 0")
                .nth(i)
                .expect("one `1 / 0` per declaration")
                .0;
            let expected = offset_to_line_col(&src, nth_div);
            assert_eq!(
                expected.0,
                i + 1,
                "sanity: decl {i}'s `1 / 0` is on line {}",
                i + 1
            );
            assert_eq!(
                offset_to_line_col(&src, span.start),
                expected,
                "error {i} must be located at its own declaration's \
                 `1 / 0` expression"
            );
        }
    }
}

/// Dimension N — cascade suppression. ONE failed `const N` followed by
/// N `data` declarations that each reference `N` in their shape must
/// produce EXACTLY 1 error (the root `ConstDivByZero` for `N`) — NOT
/// `1 + N`. The N `ShapeRefersToNonConst` errors are pure cascade of
/// the already-reported root and are suppressed. Iterated over several
/// N so a fixed-N fixture cannot mask a linear `1+N` cascade (the exact
/// shape that recurred on the scheduler side).
#[test]
fn one_failed_const_with_n_dependents_yields_exactly_one_error() {
    for n in [1usize, 2, 5, 8] {
        // `const N` fails (div-by-zero). Each `data d{i} : f32[N]`
        // would, in isolation, raise ShapeRefersToNonConst{N} because
        // the failed `N` is absent from the symbol table — but `N` is
        // POISONED, so every such secondary error is a cascade of the
        // one root failure and must be suppressed.
        let mut src = String::from("const N : usize = 1 / 0;\n");
        for i in 0..n {
            src.push_str(&format!("data d{i} : f32[N];\n"));
        }
        let errs = lower_str(&src)
            .expect_err("the failed const N must produce its root error");

        assert_eq!(
            errs.errors().len(),
            1,
            "1 failed const with N={n} dependents must yield EXACTLY 1 \
             error (no {n}-cascade), got {} — source:\n{src}",
            errs.errors().len()
        );
        match &errs.errors()[0].kind {
            LowerErrorKind::ConstDivByZero { in_const } => assert_eq!(in_const, "N"),
            other => panic!("the sole error must be the root ConstDivByZero(N), got {other:?}"),
        }
    }
}

/// The two dimensions COMBINED — the discriminating case that proves
/// suppression is targeted, not a blanket "only ever one error". One
/// failed `const N` with N dependents (all suppressed) PLUS M
/// genuinely-independent bad consts must yield EXACTLY `1 + M` errors:
/// the root, then each independent — the cascade collapses to its root
/// while every independent violation still surfaces. Undercount
/// (suppressing the independents) and overcount (emitting the cascade)
/// are both caught here, across varying N and M.
#[test]
fn cascade_suppressed_while_independents_still_surface() {
    for n in [1usize, 2, 5] {
        for m in [1usize, 2, 3] {
            let mut src = String::from("const N : usize = 1 / 0;\n");
            for i in 0..n {
                src.push_str(&format!("data d{i} : f32[N];\n"));
            }
            for j in 0..m {
                src.push_str(&format!("const Indep{j} : usize = 2 / 0;\n"));
            }
            let errs = lower_str(&src).expect_err("root + M independents must error");

            assert_eq!(
                errs.errors().len(),
                1 + m,
                "N={n} suppressed dependents + M={m} independents must \
                 yield EXACTLY 1+M={} errors, got {} — source:\n{src}",
                1 + m,
                errs.errors().len()
            );
            // First error: the root failed const N (source order).
            assert!(
                matches!(
                    &errs.errors()[0].kind,
                    LowerErrorKind::ConstDivByZero { in_const } if in_const == "N"
                ),
                "first error must be the root ConstDivByZero(N), got {:?}",
                errs.errors()[0].kind
            );
            // Remaining M: each independent bad const, in source order;
            // none is a suppressed dependent and none is the root.
            for (j, e) in errs.errors()[1..].iter().enumerate() {
                assert!(
                    matches!(
                        &e.kind,
                        LowerErrorKind::ConstDivByZero { in_const }
                            if in_const == &format!("Indep{j}")
                    ),
                    "independent error {j} must be ConstDivByZero(Indep{j}) \
                     in source order, got {:?}",
                    e.kind
                );
            }
        }
    }
}

/// Transitive depth — cascade-decls are transitively poisoned
/// (TASK-0092 transitive-poison fix, the 5th-recurrence remediation).
///
/// Shape: one failed root `const N : usize = 1 / 0;` (the upstream
/// poison root), then **K** cascade-data-decls `data dk_i : f32[N];`
/// (each fails ShapeRefersToNonConst{N} and is itself a cascade), then
/// for each cascade-decl **L** statements `dump(dk_i);` that would,
/// absent the fix, each emit `UnknownIdent(dk_i)` (the data decl was
/// suppressed and is absent from `ir.data`, and the cascade rule would
/// miss them because the previous design left `dk_i` out of
/// `failed_decls`).
///
/// Expected count: EXACTLY 1 error for every (K, L) combination — the
/// root `ConstDivByZero { in_const: "N" }`. Without the transitive
/// poison, this test measures `1 + K*L` (PROBE 5 / PROBE 14 territory:
/// (K=1, L=2) → 3 errors observed; (K=1, L=1) for-body → 2 errors
/// observed — the now-fixed overcount).
///
/// Both dimensions iterated over ≥3 values; a single-shape fixture is
/// itself the masking defect that bit TASK-0080/0081/0087 and the
/// prior TASK-0092 cycles, and the discipline this cycle is to
/// genuinely parameterise both.
#[test]
fn transitive_cascade_collapses_for_any_k_l() {
    for k in [1usize, 2, 3, 5] {
        for l in [1usize, 2, 3] {
            // Root poison: `const N` divides by zero.
            let mut src = String::from("const N : usize = 1 / 0;\n");
            // Sink kernel used by the L bare-call dependants — declared
            // so the call doesn't fail on `UnknownIdent(dump)`. The
            // cascade in question is the ARGUMENT, not the callee.
            src.push_str("kernel dump : (f32[N]) -> () effectful;\n");
            // K cascade-data-decls that depend on the poisoned N.
            for i in 0..k {
                src.push_str(&format!("data dk{i} : f32[N];\n"));
            }
            // L bare-call statements per cascade-decl, each reading
            // `dki` — would emit `UnknownIdent(dki)` per-statement
            // without the transitive poison.
            for i in 0..k {
                for _ in 0..l {
                    src.push_str(&format!("dump(dk{i});\n"));
                }
            }

            let errs = lower_str(&src).expect_err("the root failed const must error");
            assert_eq!(
                errs.errors().len(),
                1,
                "K={k} cascade-decls × L={l} statements must collapse to \
                 EXACTLY 1 error (the root ConstDivByZero{{N}}), got {} — \
                 source:\n{src}",
                errs.errors().len()
            );
            match &errs.errors()[0].kind {
                LowerErrorKind::ConstDivByZero { in_const } => {
                    assert_eq!(
                        in_const, "N",
                        "the sole surviving error must be the root \
                         ConstDivByZero(N) for (K={k}, L={l})"
                    );
                }
                other => panic!(
                    "(K={k}, L={l}): the sole error must be the root \
                     ConstDivByZero(N), got {other:?}"
                ),
            }
        }
    }
}

/// Zero behaviour change for VALID input (AC#5): a well-formed program
/// that lowered before still lowers to `Ok(AlgoIR)` — multi-error
/// accumulation must never turn a valid program into an error set. The
/// determinism gate proves byte-identical codegen separately; this is
/// the unit-level guard that `Accum` returns `None` (→ `Ok`) when no
/// error is recorded.
#[test]
fn valid_program_still_lowers_under_multi_error() {
    let src = "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel src : () -> f32[N] effectful;
kernel f   : (f32[N]) -> f32[N] pure;
x <-- src();
y <-- f(x);
";
    let ir = lower_str(src).expect("a valid program must still lower to Ok(AlgoIR)");
    assert_eq!(ir.consts["N"].value, 4);
    assert_eq!(ir.data.len(), 2);
    assert_eq!(ir.kernels.len(), 2);
    assert_eq!(ir.stmts.len(), 2);
}

// --------------------------------------------------------------------
// TASK-0089: kernel-purity vs statement-form enforcement.
//
// Grammar §2 note 5 — the ONLY direction the formal grammar specifies:
// a bare-call statement (`EffectStmt`) must reference an `effectful`
// kernel. A bare-call to a `pure` kernel discards the return value
// (the only thing a pure kernel produces) and is meaningless.
//
// The OTHER direction (DataflowStmt RHS must be pure) is intentionally
// NOT enforced — every shipped example (01..07, 13, 14) puts an
// effectful load/capture kernel on the RHS of `<--` (see TASK-0201 for
// the spec decision on whether to reverse that). The positive test
// `pure_dataflow_with_effectful_rhs_load_lowers` below pins that
// load-bearing pattern so a future strict-bidirectional reinterpretation
// can't silently regress every example.
// --------------------------------------------------------------------

/// Positive: a pure kernel call on the RHS of `<--` (the textbook case)
/// lowers cleanly. Smoke check that the new check does not over-fire.
#[test]
fn pure_dataflow_with_pure_rhs_lowers() {
    let src = "\
const N : usize = 4;
data a : i32[N];
data b : i32[N];
kernel load_a : () -> i32[N] effectful;
kernel id_arr : (i32[N]) -> i32[N] pure;
a <-- load_a();
b <-- id_arr(a);
";
    let ir = lower_str(src).expect("pure RHS in dataflow must lower");
    assert_eq!(ir.stmts.len(), 2);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::Dataflow { .. }));
}

/// Positive: an effectful kernel as a bare-call statement lowers
/// cleanly. The other half of the no-over-fire check.
#[test]
fn effect_stmt_calling_effectful_lowers() {
    let src = "\
const N : usize = 4;
data x : i32[N];
kernel load  : ()       -> i32[N] effectful;
kernel store : (i32[N]) -> ()     effectful;
x <-- load();
store(x);
";
    let ir = lower_str(src).expect("effectful effect-stmt must lower");
    assert_eq!(ir.stmts.len(), 2);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
    assert!(matches!(ir.stmts[1], IrStmt::Effect { .. }));
}

/// Positive — load-bearing: the universal `data <-- effectful_load();`
/// pattern present in EVERY shipped example (01..07, 13, 14) must keep
/// lowering. If a future change ever turns DataflowStmt-RHS-must-be-pure
/// on, THIS test is the first thing to fail, surfacing the regression
/// at the unit level before the example-level lowers_example_* tests do.
/// See TASK-0089 onboarding notes and TASK-0201 for the spec context.
#[test]
fn pure_dataflow_with_effectful_rhs_load_lowers() {
    let src = "\
const N : usize = 4;
data a : i32[N];
kernel load_input : () -> i32[N] effectful;
a <-- load_input();
";
    let ir = lower_str(src).expect("the universal `data <-- effectful_load();` pattern must lower");
    assert_eq!(ir.stmts.len(), 1);
    assert!(matches!(ir.stmts[0], IrStmt::Dataflow { .. }));
}

/// Negative: an `EffectStmt` calls a `pure` kernel — grammar §2 note 5
/// violation. Reports [`LowerErrorKind::EffectCalleeNotEffectful`] at
/// the callee's identifier span.
#[test]
fn effect_stmt_calling_pure_kernel_is_error() {
    let src = "\
const N : usize = 4;
data x : i32[N];
kernel load_input : () -> i32[N] effectful;
kernel add_one   : (i32[N]) -> i32[N] pure;
x <-- load_input();
add_one(x);
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::EffectCalleeNotEffectful { callee },
            ..
        }) => assert_eq!(callee, "add_one"),
        other => panic!("expected EffectCalleeNotEffectful; got {other:?}"),
    }
}

/// Negative + located: the new variant carries the callee's identifier
/// span so the driver renders `... at L:C` correctly. Pin the exact
/// `(line, column)` recomputed from the source string (same pattern as
/// `located_errors_carry_correct_line_col`).
#[test]
fn located_effect_purity_error_has_correct_line_col() {
    let src = "data x : i32[4];\nkernel pure_k : () -> i32[4] pure;\nx <-- pure_k();\npure_k();\n";
    // The error is on the bare-call line 4 at the `pure_k` identifier.
    let err = lower_str(src)
        .expect_err("bare-call to pure kernel must error")
        .first()
        .clone();
    assert!(
        matches!(err.kind, LowerErrorKind::EffectCalleeNotEffectful { ref callee } if callee == "pure_k"),
        "got {err:?}"
    );
    // The two `pure_k(` occurrences appear at lines 3 (in dataflow RHS,
    // legal) and 4 (in effect-stmt, the violation). The error must
    // point at the line-4 occurrence.
    let last_pure_k = src.rfind("pure_k").expect("`pure_k` in source");
    let expected = offset_to_line_col(src, last_pure_k);
    assert_eq!(expected, (4, 1), "sanity: bare-call `pure_k` is at line 4 col 1");
    let span = err.span.clone().expect("EffectCalleeNotEffectful carries a span");
    assert_eq!(
        offset_to_line_col(src, span.start),
        expected,
        "EffectCalleeNotEffectful must point at the bare-call callee"
    );
    // Driver-facing rendering carries the located form.
    assert!(
        err.display_with_src(src).contains("at 4:1"),
        "rendered diagnostic must carry `at 4:1`; got `{}`",
        err.display_with_src(src)
    );
}

/// Multi-violation: two independent purity violations in one program
/// are each reported at their own site (TASK-0092 multi-error
/// infrastructure — collected, not bailed). Pins the per-site nature
/// of the check (this is NOT a cascade-class defect).
#[test]
fn multiple_effect_purity_violations_each_reported() {
    let src = "\
const N : usize = 4;
data x : i32[N];
data y : i32[N];
kernel load_x  : ()        -> i32[N] effectful;
kernel pure_a  : (i32[N])  -> i32[N] pure;
kernel pure_b  : (i32[N])  -> i32[N] pure;
x <-- load_x();
y <-- pure_a(x);
pure_a(x);
pure_b(y);
";
    let errs = lower_str(src).expect_err("two pure-bare-calls must each error");
    let kinds: Vec<&LowerErrorKind> = errs.errors().iter().map(|e| &e.kind).collect();
    assert_eq!(
        kinds.len(),
        2,
        "expected exactly two independent purity violations, got {} (kinds = {kinds:?})",
        kinds.len()
    );
    // Order: source order — first `pure_a();`, then `pure_b();`.
    match kinds[0] {
        LowerErrorKind::EffectCalleeNotEffectful { callee } => assert_eq!(callee, "pure_a"),
        other => panic!("first error: expected EffectCalleeNotEffectful(pure_a); got {other:?}"),
    }
    match kinds[1] {
        LowerErrorKind::EffectCalleeNotEffectful { callee } => assert_eq!(callee, "pure_b"),
        other => panic!("second error: expected EffectCalleeNotEffectful(pure_b); got {other:?}"),
    }
    // Each error carries its own callee span; they are distinct
    // positions in the source.
    let spans: Vec<_> = errs.errors().iter().filter_map(|e| e.span.clone()).collect();
    assert_eq!(spans.len(), 2, "both errors must carry spans");
    assert_ne!(spans[0], spans[1], "the two violation spans must differ");
}

/// Cascade discipline: a bare-call to an UNKNOWN kernel (never declared)
/// remains an [`LowerErrorKind::UnknownIdent`] — the purity check
/// naturally short-circuits when the kernel is not in `ir.kernels`, so
/// there is no double-emit. This pins the "no new cascade-suppression
/// rule needed" property documented on `Stmt::Effect` in lower.rs.
#[test]
fn effect_stmt_to_unknown_kernel_stays_unknown_ident() {
    let src = "ghost();\n";
    let err = lower_str(src)
        .expect_err("bare-call to undeclared kernel must error")
        .first()
        .clone();
    match err.kind {
        LowerErrorKind::UnknownIdent(n) => assert_eq!(n, "ghost"),
        other => panic!("expected UnknownIdent; got {other:?}"),
    }
}

/// Cascade discipline (TASK-0203, sibling of
/// [`effect_stmt_to_unknown_kernel_stays_unknown_ident`]): the
/// DECLARED-but-failed-body kernel path.
///
/// Pins the lower.rs `Stmt::Effect` comment claim that "if the kernel
/// was declared but its body failed to lower, the existing
/// `is_cascade_of_failed_decl` UnknownIdent suppression collapses the
/// error to the root declaration failure". The other test
/// `effect_stmt_to_unknown_kernel_stays_unknown_ident` covers the
/// never-declared case; this one covers the declared-but-signature-
/// failed case, which is the harder branch because it depends on the
/// TASK-0092 case-1 *transitive* poison: `BAD_CONST` failing must
/// transitively poison `bad_kernel` into `failed_decls`, otherwise the
/// downstream `bad_kernel();` bare-call leaks an `UnknownIdent`
/// cascade.
///
/// Discrimination strength:
/// - If the case-1 transitive-poison fix (TASK-0092 cycle-3, commit
///   79c654d) were reverted, the kernel name would NOT be in
///   `failed_decls`, so the bare-call's `UnknownIdent("bad_kernel")`
///   would NOT be suppressed and we'd see 2 errors (root +
///   leaked-cascade) — this test would fail with the leaked-cascade
///   error vector. The assertion is therefore the right discriminator
///   for that mechanism, not a blanket `len > 0`.
/// - The kernel is declared `pure` so that IF the implementation ever
///   regressed to half-insert the kernel into `ir.kernels` despite the
///   signature failure, the bare-call would spuriously raise
///   `EffectCalleeNotEffectful` — the test pins that no such
///   spurious purity-mismatch survives (AC#2 (c)).
#[test]
fn effect_stmt_to_declared_but_failed_kernel_collapses_to_root() {
    // `BAD_CONST` fails (div-by-zero, the root). The kernel signature
    // references the poisoned `BAD_CONST` in a dim expression, so its
    // own lowering fails with `ShapeRefersToNonConst{BAD_CONST}`. Per
    // the TASK-0092 case-1 transitive-poison fix, `bad_kernel` is
    // ALSO inserted into `failed_decls` (the signature-failure-as-
    // cascade case), so the downstream bare-call's
    // `UnknownIdent("bad_kernel")` is recognised as a cascade of the
    // root and suppressed. The kernel is declared `pure` to make the
    // "no spurious EffectCalleeNotEffectful" assertion non-trivial:
    // a regression that half-inserts the kernel anyway would surface
    // here.
    let src = "const BAD_CONST : usize = 1 / 0;\n\
kernel bad_kernel : (i32[BAD_CONST]) -> () pure;\n\
bad_kernel();\n";

    let errs = lower_str(src)
        .expect_err("the failed const must produce its root error");

    // AC#1: EXACTLY one error survives — the root `ConstDivByZero`.
    // The kernel decl's `ShapeRefersToNonConst` is a cascade
    // (suppressed). The bare-call's `UnknownIdent` is a cascade
    // (suppressed). The purity check naturally short-circuits because
    // `bad_kernel` is not in `ir.kernels`. Total: 1.
    assert_eq!(
        errs.errors().len(),
        1,
        "declared-but-failed-kernel cascade must collapse to EXACTLY 1 \
         error (the root `ConstDivByZero(BAD_CONST)`), got {} — kinds: \
         {:?} — source:\n{src}",
        errs.errors().len(),
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>()
    );

    // AC#2 (b): the sole survivor is the root with the right kind.
    let err = &errs.errors()[0];
    match &err.kind {
        LowerErrorKind::ConstDivByZero { in_const } => assert_eq!(
            in_const, "BAD_CONST",
            "the surviving error's `in_const` field must name the root \
             failed const"
        ),
        other => panic!(
            "the sole surviving error must be the root \
             `ConstDivByZero(BAD_CONST)`, got {other:?}"
        ),
    }

    // AC#1 + AC#2 (b): the line:col of the root error pins to the
    // offending `1 / 0` expression on line 1. Recompute the expected
    // `(line, col)` from the source so the test pins the real
    // position, not a guessed constant — matching the idiom used by
    // `cascade_independent_consts_each_carry_own_span` and
    // `located_errors_carry_correct_line_col`.
    let div_at = src.find("1 / 0").expect("`1 / 0` in source");
    let expected = offset_to_line_col(src, div_at);
    assert_eq!(
        expected, (1, 27),
        "sanity: the `1 / 0` of BAD_CONST is on line 1 col 27"
    );
    assert_eq!(
        err_line_col(src, err),
        expected,
        "the root `ConstDivByZero` must point at the `1 / 0` \
         expression in the BAD_CONST declaration"
    );

    // AC#2 (a): no `UnknownIdent("bad_kernel")` anywhere in the error
    // vector. This is the discriminating assertion for the TASK-0092
    // case-1 transitive-poison path: if it regressed, the bare-call
    // would leak an `UnknownIdent("bad_kernel")` cascade here.
    let leaked_unknown_ident = errs.errors().iter().any(|e| {
        matches!(&e.kind, LowerErrorKind::UnknownIdent(n) if n == "bad_kernel")
    });
    assert!(
        !leaked_unknown_ident,
        "no `UnknownIdent(\"bad_kernel\")` may leak — the kernel name \
         must be transitively poisoned into `failed_decls` by the \
         signature-failure cascade (TASK-0092 case-1). Errors: {:?}",
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>()
    );

    // AC#2 (c): no `EffectCalleeNotEffectful{bad_kernel}` spuriously
    // emitted — the kernel never made it into `ir.kernels` (its
    // signature lowering failed), so the purity check at lower.rs:804
    // is unreachable for this callee. This pins the "purity check
    // naturally short-circuits when kernel is not in `ir.kernels`"
    // half of the comment claim.
    let leaked_purity_mismatch = errs.errors().iter().any(|e| {
        matches!(
            &e.kind,
            LowerErrorKind::EffectCalleeNotEffectful { callee } if callee == "bad_kernel"
        )
    });
    assert!(
        !leaked_purity_mismatch,
        "no `EffectCalleeNotEffectful(bad_kernel)` may leak — the \
         kernel was never inserted into `ir.kernels` (signature \
         lowering failed), so the purity check must naturally short- \
         circuit. Errors: {:?}",
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>()
    );

    // Belt-and-braces: also assert no leaked
    // `ShapeRefersToNonConst{BAD_CONST}` (the kernel-decl's own
    // cascade error) — this would mean case-1 suppression broke at
    // the declaration level, not just the transitive-poison level.
    // The pre-existing `one_failed_const_with_n_dependents_yields_
    // exactly_one_error` already pins this for data-decls; here we
    // pin it for kernel-decls.
    let leaked_shape_cascade = errs.errors().iter().any(|e| {
        matches!(
            &e.kind,
            LowerErrorKind::ShapeRefersToNonConst { unknown_ident, .. } if unknown_ident == "BAD_CONST"
        )
    });
    assert!(
        !leaked_shape_cascade,
        "no `ShapeRefersToNonConst(BAD_CONST)` may leak — the kernel \
         signature's reference to the poisoned `BAD_CONST` is a \
         cascade of the root and must be suppressed (case-1). \
         Errors: {:?}",
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}
