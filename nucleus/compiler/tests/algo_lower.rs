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
    lower_algo, parse_algo, AlgoIR, IrStmt, LowerError, ResolvedType, ScalarType,
};

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

fn lower_str(src: &str) -> Result<AlgoIR, LowerError> {
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
    match lower_str(src) {
        Err(LowerError::DuplicateKernel(n)) => assert_eq!(n, "k"),
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
    match lower_str(src) {
        Err(LowerError::DuplicateData(n)) => assert_eq!(n, "x"),
        other => panic!("expected DuplicateData; got {other:?}"),
    }
}

#[test]
fn duplicate_const_name_is_error() {
    let src = "\
const N : usize = 4;
const N : usize = 8;
";
    match lower_str(src) {
        Err(LowerError::DuplicateConst(n)) => assert_eq!(n, "N"),
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
    let err = lower_str(src).expect_err("must error");
    // The variant is DuplicateData (the second declaration); the
    // important thing is the collision is detected.
    assert!(
        matches!(err, LowerError::DuplicateData(ref n) if n == "N"),
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
    match lower_str(src) {
        Err(LowerError::DoubleAssignment { data, .. }) => assert_eq!(data, "x"),
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
    match lower_str(src) {
        Err(LowerError::IterVarOutOfScope(n)) => assert_eq!(n, "y"),
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
    match lower_str(src) {
        Err(LowerError::ConstDivByZero { in_const }) => assert_eq!(in_const, "Q"),
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
    match lower_str(src) {
        Err(LowerError::ConstRefersToNonConst {
            in_const,
            unknown_ident,
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
    match lower_str(src) {
        Err(LowerError::NonPositiveDim { decl, value }) => {
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
    match lower_str(src) {
        Err(LowerError::UnknownIdent(n)) => assert_eq!(n, "nope"),
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
    match lower_str(src) {
        Err(LowerError::AssignmentTargetNotData(n)) => assert_eq!(n, "N"),
        other => panic!("expected AssignmentTargetNotData; got {other:?}"),
    }
}
