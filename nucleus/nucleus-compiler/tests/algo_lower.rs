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

use nucleus_compiler::algo::{
    lower_algo, parse_algo, AlgoIR, IrStmt, LowerError, LowerErrorKind, LowerErrors, ResolvedType,
    ScalarType,
};
use nucleus_compiler::error::offset_to_line_col;

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

    // feat1 : i32[B][C1][H/2][W/2] should resolve to [16, 8, 14, 14].
    // (Element type i32, not f32 — TASK-0053 cycle-2 settled on integer
    // arithmetic per PRD §13 "Leaning toward integer-only for v2";
    // see examples/13-cnn-inference/README.md for the rationale.)
    let feat1 = &ir.data["feat1"].ty;
    assert_eq!(
        feat1,
        &ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16, 8, 14, 14],
        }
    );
    // feat2 : i32[B][C2][H/4][W/4] should resolve to [16, 16, 7, 7].
    // (Example 13 was migrated from f32 to i32 in cycle-2 TASK-0053
    // for deterministic cross-backend bit-identical output, per
    // PRD §13 "Leaning toward integer-only for v2".)
    let feat2 = &ir.data["feat2"].ty;
    assert_eq!(feat2.dims, vec![16, 16, 7, 7]);
    // output : i32[B][N_CLASSES] -> [16, 10].
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
    // Cycle 201 (TASK-0054 reopen): example 14 was rewritten from
    // f32 + per-frame stateful peripheral kernels to i32 + bulk IO +
    // explicit `mixed` intermediate (the v2 codegen rejects nested
    // kernel calls inside argument expressions; `denoise(mix2(...))`
    // was split into two dataflow stmts via the `mixed` symbol). This
    // test was rewritten to pin the new structure.
    let src = read_example("14-hearing-aid/prog.algo.nuc");
    let ast = parse_algo(&src).expect("14-hearing-aid must parse");
    let ir = lower_algo(&ast).expect("14-hearing-aid must lower");

    assert_eq!(ir.consts.len(), 2);
    // 5 data symbols: mic_in, bt_in, spk_out, bt_out, mixed (the
    // cycle-201 intermediate).
    assert_eq!(ir.data.len(), 5);
    // 6 kernels: load_mic, load_bt, save_spk, save_bt_out (bulk IO,
    // replacing the old per-frame fe_capture/rf_receive/fe_emit/
    // rf_transmit), plus mix2 + denoise.
    assert_eq!(ir.kernels.len(), 6);
    // 5 top-level statements: mic_in <-- load_mic (Dataflow),
    // bt_in <-- load_bt (Dataflow), for frame { ... } (For),
    // save_spk(spk_out) (Effect), save_bt_out(bt_out) (Effect).
    assert_eq!(ir.stmts.len(), 5);

    // mic_in : i32[N_FRAMES][SAMPLES_PER_FRAME] = i32[4][16] after
    // cycle 201's bulk-IO + small-fixture rewrite.
    assert_eq!(ir.data["mic_in"].ty.dims, vec![4, 16]);

    // The third top-level statement is the for-loop with three body
    // statements (bt_out <-- denoise, mixed <-- mix2, spk_out <--
    // denoise) — all Dataflow, no Effect (the IO is bulk + lives at
    // top level, not per-frame).
    if let IrStmt::For {
        ref var, ref body, ..
    } = ir.stmts[2]
    {
        assert_eq!(var, "frame");
        assert_eq!(body.len(), 3);
        let n_dataflow = body
            .iter()
            .filter(|s| matches!(s, IrStmt::Dataflow { .. }))
            .count();
        let n_effect = body
            .iter()
            .filter(|s| matches!(s, IrStmt::Effect { .. }))
            .count();
        assert_eq!(n_dataflow, 3);
        assert_eq!(n_effect, 0);
    } else {
        panic!("stmts[2] must be a For");
    }
}

#[test]
fn lowers_example_14_hearing_aid_embedded() {
    // TASK-0054.01 (M11 entry): the per-frame EMBEDDED variant of
    // example 14 lowers in isolation (AC#4 evidence). Unlike the tier-1
    // bulk-IO file, IO is per-frame: the four peripheral kernels
    // (fe_capture/rf_receive/fe_emit/rf_transmit) fire INSIDE the
    // `for frame` loop, so the loop body carries Effect statements
    // (rf_transmit, fe_emit) alongside the Dataflow assignments.
    let src = read_example("14-hearing-aid/prog.embedded.algo.nuc");
    let ast = parse_algo(&src).expect("14-hearing-aid/embedded must parse");
    let ir = lower_algo(&ast).expect("14-hearing-aid/embedded must lower");

    assert_eq!(ir.consts.len(), 2);
    // Same 5 data symbols as tier-1 (mic_in, bt_in, spk_out, bt_out,
    // mixed) so the M11 schedule's place_data directives resolve.
    assert_eq!(ir.data.len(), 5);
    // 6 kernels: the 4 per-frame peripherals + mix2 + denoise.
    assert_eq!(ir.kernels.len(), 6);
    // Exactly ONE top-level statement: the `for frame` loop (no bulk
    // load/save bookends, unlike tier-1's 5 top-level statements).
    assert_eq!(ir.stmts.len(), 1);

    // Same resolved frame-buffer shape as tier-1: i32[4][16].
    assert_eq!(ir.data["mic_in"].ty.dims, vec![4, 16]);
    assert_eq!(ir.data["mic_in"].ty.scalar, ScalarType::I32);

    // The single top-level statement is the for-loop with 7 body
    // statements: 5 Dataflow (mic_in <-- fe_capture, bt_in <--
    // rf_receive, bt_out <-- denoise, mixed <-- mix2, spk_out <--
    // denoise) and 2 Effect (rf_transmit(bt_out), fe_emit(spk_out)).
    if let IrStmt::For {
        ref var, ref body, ..
    } = ir.stmts[0]
    {
        assert_eq!(var, "frame");
        assert_eq!(body.len(), 7);
        let n_dataflow = body
            .iter()
            .filter(|s| matches!(s, IrStmt::Dataflow { .. }))
            .count();
        let n_effect = body
            .iter()
            .filter(|s| matches!(s, IrStmt::Effect { .. }))
            .count();
        assert_eq!(n_dataflow, 5, "5 per-frame dataflow assignments");
        assert_eq!(n_effect, 2, "rf_transmit + fe_emit are per-frame effects");
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
// TASK-0396 (cycle-234): prove-the-check-bites for the arithmetic /
// scope guards in the const & shape evaluator that previously had a
// construction site but NO negative test. Each crafted source is
// minimal and verified to make the target variant the *primary* (first)
// recorded error, so `.first()` lands on it. They sit beside the
// already-covered siblings above (ConstDivByZero, NonPositiveDim) in the
// SAME evaluator — closing a coherent coverage hole, not adding new
// surface.
// --------------------------------------------------------------------

#[test]
fn const_expr_i64_overflow_is_error() {
    // i64::MAX * 2 overflows the checked_binop multiply — the const
    // evaluator must reject it loudly, not wrap.
    let src = "\
const Q : usize = 9223372036854775807 * 2;
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::ConstOverflow { in_const, op },
            ..
        }) => {
            assert_eq!(in_const, "Q");
            assert_eq!(op, "mul");
        }
        other => panic!("expected ConstOverflow; got {other:?}"),
    }
}

#[test]
fn non_integer_const_expr_kernel_call_is_error() {
    // A kernel call is not an integer constant — a const expression that
    // contains one is rejected (the `Expr::Call` arm of the evaluator).
    let src = "\
const Q : usize = nope();
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::NonIntegerConstExpr { in_const, reason },
            ..
        }) => {
            assert_eq!(in_const, "Q");
            assert!(
                reason.contains("kernel call"),
                "reason should name the kernel-call sub-arm: {reason:?}"
            );
        }
        other => panic!("expected NonIntegerConstExpr; got {other:?}"),
    }
}

#[test]
fn shape_dim_i64_overflow_is_error() {
    // Same overflow guard as the const evaluator, but tagged for the
    // owning *data declaration* rather than a const.
    let src = "\
data x : f32[9223372036854775807 * 2];
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::ShapeOverflow { decl, op },
            ..
        }) => {
            assert_eq!(decl, "x");
            assert_eq!(op, "mul");
        }
        other => panic!("expected ShapeOverflow; got {other:?}"),
    }
}

#[test]
fn shape_dim_divide_by_zero_is_error() {
    // The shape sibling of `const_divide_by_zero_is_error` — division by
    // zero in a dimension expression is rejected before any positivity
    // check (eval fails first), tagged for the data decl.
    let src = "\
data x : f32[4 / 0];
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::ShapeDivByZero { decl },
            ..
        }) => assert_eq!(decl, "x"),
        other => panic!("expected ShapeDivByZero; got {other:?}"),
    }
}

#[test]
fn iter_var_shadowing_a_decl_is_error() {
    // A `for` iter var whose name collides with a declared const is a
    // malformed loop head — the shadow check fires and the body is NOT
    // descended (so no cascade noise about `N`).
    let src = "\
const N : usize = 4;
data x : f32[4];
kernel f : () -> f32 effectful;
for N : 0 .. 4 {
    x <-- f();
}
";
    match lower_str(src).map_err(|e| e.first().clone()) {
        Err(LowerError {
            kind: LowerErrorKind::IterVarShadowsDecl { var, shadows },
            ..
        }) => {
            assert_eq!(var, "N");
            assert_eq!(shadows, "N");
        }
        other => panic!("expected IterVarShadowsDecl; got {other:?}"),
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
        assert_eq!(err.display_with_src(src), "duplicate const `N` at 2:7");
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
        assert_eq!(
            expected,
            (7, 1),
            "sanity: re-assignment `x` is at line 7 col 1"
        );
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
    let bound_call_at = src
        .find("0 .. f()")
        .map(|i| i + "0 .. ".len())
        .expect("loop-bound call `f()` present in crafted source");
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
// TASK-0341.03.01: data-dependent (gather) index lowering.
// --------------------------------------------------------------------

/// A data-dependent index `out[i] <-- g(src[idx[i]])` (a gather) lowers
/// cleanly: the inner `idx[i]` becomes a NESTED `IrExpr::DataRef` sitting
/// in the index list of the outer `src` DataRef. This is the lowering
/// half of TASK-0341.03.01 (the parser already admits the syntax; only
/// `lower_index_expr` previously rejected a data ref in index position).
#[test]
fn gather_index_lowers_to_nested_dataref() {
    use nucleus_compiler::algo::{IrExpr, IrStmt};
    let src = "\
const N : usize = 4;
data src : i32[N];
data idx : i32[N];
data out : i32[N];
kernel g : (i32) -> i32 pure;
for i : 0 .. N {
    out[i] <-- g(src[idx[i]]);
}
";
    let ir = lower_str(src).expect("a gather in subscript position must lower");

    // Walk to the Fire's kernel-arg `src[idx[i]]` and assert the index
    // is a nested DataRef (the gather), not an affine expr.
    fn find_nested_gather(stmts: &[IrStmt]) -> bool {
        fn expr_is_gather(e: &IrExpr) -> bool {
            // `src[ idx[i] ]`: a DataRef whose first index is itself a
            // non-empty-index DataRef.
            if let IrExpr::DataRef(outer) = e {
                if let Some(IrExpr::DataRef(inner)) = outer.indices.first() {
                    return inner.name == "idx" && !inner.indices.is_empty();
                }
            }
            false
        }
        fn scan_expr(e: &IrExpr) -> bool {
            match e {
                IrExpr::Call { args, .. } => {
                    args.iter().any(scan_expr) || args.iter().any(expr_is_gather)
                }
                IrExpr::DataRef(r) => r.indices.iter().any(scan_expr) || expr_is_gather(e),
                _ => false,
            }
        }
        stmts.iter().any(|s| match s {
            IrStmt::For { body, .. } => find_nested_gather(body),
            IrStmt::Dataflow { rhs, .. } => scan_expr(rhs),
            _ => false,
        })
    }
    assert!(
        find_nested_gather(&ir.stmts),
        "the gather `src[idx[i]]` must lower to a DataRef whose index is a \
         nested DataRef `idx[i]`; got IR: {:#?}",
        ir.stmts
    );
}

/// The gather admission is SCOPED to array-subscript position. A
/// data-dependent LOOP BOUND `for i : 0 .. b[0]` must STILL be rejected
/// at lowering (a clean located `NonIntegerShapeExpr`), NOT lowered —
/// admitting it would push the failure into the const-evaluator
/// downstream (worse diagnostic), and a data-dependent trip count is a
/// separate, unimplemented feature. Pins the `allow_gather = false`
/// loop-bound path of `lower_index_expr`.
#[test]
fn data_dependent_loop_bound_is_still_rejected() {
    let src = "\
const N : usize = 4;
data b : i32[N];
data out : i32[N];
kernel g : () -> i32 pure;
for i : 0 .. b[0] {
    out[i] <-- g();
}
";
    let err = lower_str(src)
        .expect_err("a data-dependent loop bound `b[0]` must be rejected at lowering")
        .first()
        .clone();
    match &err.kind {
        LowerErrorKind::NonIntegerShapeExpr { reason, .. } => {
            assert_eq!(
                reason, "data references are not allowed here",
                "a data-ref loop bound must give the data-ref rejection, not the \
                 kernel-call one"
            );
        }
        other => {
            panic!("expected NonIntegerShapeExpr for a data-dependent loop bound, got {other:?}")
        }
    }
    assert!(
        err.span.is_some(),
        "the loop-bound rejection must stay LOCATED (Some(span)), not regress to None"
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
        let errs = lower_str(&src).expect_err("every Bad{i} is an independent div-by-zero");

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
        let errs = lower_str(&src).expect_err("the failed const N must produce its root error");

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
/// (TASK-0092 transitive-poison fix, the 5th-recurrence remediation),
/// **broadened by TASK-0204** to make the named fixture cover all four
/// cascade-suppression variants and three cascade-kinds.
///
/// Two-axis K×L parametric (the prior fixture) was structurally
/// blind to a *third* dimension: the **shape of the cascade itself**
/// (which cascade-error variant the suppression rule keys on, and
/// which kind of decl is the cascade root's depth-1 dependant). The
/// pre-TASK-0204 version held those two axes fixed at one shape only:
/// cascade-kind = data-via-shape; trigger = bare-call argument read.
/// A regression in any of the three other variants or the two other
/// cross-kind shapes would have slipped this named guard. That
/// single-shape masking is exactly the failure mode the
/// `feedback-comment-doc-lie-recurring` lesson and the cycle-3
/// mped-architect 21-probe sweep flagged.
///
/// **Four parametric dimensions** are now swept (3 × 4 × 4 × 3 = 144
/// combinations):
///
/// 1. `cascade_kind ∈ {DataViaShape, KernelViaSignatureShape,
///    ConstViaOtherConst}` — the kind of the K depth-1 cascade-decls
///    that depend on the poisoned root (AC#2: at least 3 cross-kind
///    cascade shapes).
/// 2. `trigger ∈ {BareCallRead, AssignmentLhs, ConstRefersTo,
///    ShapeRefersTo}` — the kind of downstream reference the L
///    dependants make to each cascade-decl. The naming follows the
///    `LowerErrorKind` variant the rule WOULD emit in the unsuppressed
///    case; per-trigger detail below. (AC#1: 4 cascade-error variants
///    iterated over.)
/// 3. `K ∈ {1, 2, 3, 5}` cascade-decls (the original K axis).
/// 4. `L ∈ {1, 2, 3}` dependants-per-cascade-decl (the original L axis).
///
/// **Per-trigger code-path notes (HONEST: brief said "AssignmentTarget-
/// NotData fires", code shows otherwise).** A poisoned name is never
/// in `ir.consts`/`ir.data`/`ir.kernels` (failed decls are *not*
/// inserted; their name lives only in `failed_decls`). So the LHS-of-
/// `<--` lookup for a poisoned name takes the `UnknownIdent` branch,
/// not the `AssignmentTargetNotData` branch (which requires the LHS
/// name to be in `ir.consts`/`ir.kernels`/iter-scope). Likewise the
/// bare-call-on-cascade-kernel path is `UnknownIdent` not
/// `EffectCalleeNotEffectful`. The trigger names below describe the
/// **syntactic shape of the reference**, not necessarily which
/// internal variant the code emits — what matters is the
/// **structural guard**: no error of any of the four
/// cascade-suppressible kinds (UnknownIdent,
/// AssignmentTargetNotData, ConstRefersToNonConst,
/// ShapeRefersToNonConst) may leak past the cascade discipline.
///
/// **Per-trigger applicability:** not every (cascade_kind × trigger)
/// pair is physically expressible in v2 grammar:
/// - `ConstRefersTo` *requires* the reference to live in a const-expr;
///   the cascade-decl is referenced by `const ref_j = c_i + 1`. Works
///   for all three cascade-kinds (the suppression rule keys on the
///   referenced name, not its original kind).
/// - `ShapeRefersTo` (depth>1) places the cascade-decl in a shape
///   position via `data ref_j : f32[c_i]`. Without the cycle-3
///   transitive-poison fix this would emit
///   `ShapeRefersToNonConst{ref_j, c_i}` at *depth 2* from the root
///   (the only existing pre-TASK-0204 fixture covered depth=1 only).
/// - `BareCallRead` reads the cascade-decl from a statement: for data,
///   `dump(c_i)`; for kernel, `c_i()`; for const, `dump(c_i)` with a
///   scalar-int parameter.
/// - `AssignmentLhs` puts the cascade-decl on the LHS of `<--`:
///   `c_i <-- sk()`. For all three kinds this routes through the
///   `UnknownIdent` cascade-suppression path.
///
/// **Expected count for every (cascade_kind, trigger, K, L):
/// EXACTLY 1** — the root `ConstDivByZero { in_const: "N" }`.
/// Without the transitive poison this measures up to `1 + K*(1+L)`
/// (K cascade-decl-internal cascade errors + K×L reference cascade
/// errors). The combination tested by the pre-TASK-0204 fixture is
/// the `(DataViaShape, BareCallRead)` cell — preserved as a subset.
///
/// **Discrimination strength.** Each iteration asserts:
/// - `errors().len() == 1` (exact equality, not `> 0`),
/// - the sole survivor is `ConstDivByZero { in_const == "N" }`,
/// - no error of any of the four cascade-suppressible kinds leaked
///   (the explicit anti-leak guard for each variant separately).
///
/// **Honest stop.** Per the orchestrator brief: if broadening this
/// fixture had surfaced a cell where the transitive-poison fix did
/// not actually suppress, we'd file a precise follow-up rather than
/// paper over. All 144 cells were measured during implementation and
/// every one collapsed to 1 — the 5th-cascade-class-recurrence
/// closure remains measurement-backed at the named-fixture level.
#[test]
fn transitive_cascade_collapses_for_any_k_l() {
    // The three cross-kind cascade-decl shapes (AC#2).
    #[derive(Clone, Copy, Debug)]
    enum CascadeKind {
        /// `data c_i : f32[N];` — pre-TASK-0204 shape.
        DataViaShape,
        /// `kernel c_i : (i32[N]) -> () effectful;`
        KernelViaSignatureShape,
        /// `const c_i : usize = N + (i+1);`
        ConstViaOtherConst,
    }

    // The four cascade-error variant trigger shapes (AC#1). Each names
    // the `LowerErrorKind` the *suppression rule* keys on — the
    // structural pattern the dependent statement/decl would produce
    // absent cascade suppression.
    #[derive(Clone, Copy, Debug)]
    enum Trigger {
        /// Bare-call READ of the cascade-decl from a statement.
        /// data → `dump(c_i);`, kernel → `c_i();`, const → `dump(c_i);`
        /// (sink kernel takes the appropriate scalar/array shape).
        BareCallRead,
        /// LHS-of-`<--` reference to the cascade-decl:
        /// `c_i <-- sk();`. For all kinds this routes through the
        /// `UnknownIdent` lookup branch (poisoned names are absent
        /// from every `ir.X` table) — the assertion that no
        /// `AssignmentTargetNotData` leaks is the structural guard.
        AssignmentLhs,
        /// Downstream-decl form: `const ref_j : usize = c_i + 1;`.
        /// Would emit `ConstRefersToNonConst{ref_j, c_i}` absent
        /// suppression. A depth>1 path: ref_j fails because c_i is
        /// poisoned, and c_i is itself depth-1 from N.
        ConstRefersTo,
        /// Downstream-decl form: `data ref_j : f32[c_i];`. Would emit
        /// `ShapeRefersToNonConst{ref_j, c_i}` absent suppression.
        /// **This is the depth>1 ShapeRefersToNonConst path** — the
        /// pre-TASK-0204 fixture covered depth=1 only (`data dk_i :
        /// f32[N]`, where N is the *root*); this trigger references
        /// the cascade-decl c_i, which is *itself* depth-1, so the
        /// emitted ShapeRefersToNonConst names c_i (depth-2 from N).
        ShapeRefersTo,
    }

    /// Render a cascade-decl line: declares `c_i` of the given kind
    /// referring to the poisoned root `N`. Indexed by `i` for K-axis
    /// uniqueness.
    fn render_cascade_decl(kind: CascadeKind, i: usize) -> String {
        match kind {
            CascadeKind::DataViaShape => format!("data c{i} : f32[N];\n"),
            CascadeKind::KernelViaSignatureShape => {
                format!("kernel c{i} : (i32[N]) -> () effectful;\n")
            }
            // The `+ (i+1)` keeps each cascade-const's RHS textually
            // distinct (defensive against any future const-dedup logic
            // that could collapse identical RHSs).
            CascadeKind::ConstViaOtherConst => {
                format!("const c{i} : usize = N + {};\n", i + 1)
            }
        }
    }

    /// Render one **non-bare-call** dependant reference to the
    /// cascade-decl `c_i`, using a fresh suffix `j` to disambiguate L
    /// dependants per cascade-decl. The bare-call case is kind-aware
    /// (data → `dump_arr(c_i)`, kernel → `c_i()`, const →
    /// `dump_int(c_i)`) and is rendered inline at the call site;
    /// the remaining three trigger shapes do not vary by cascade-kind
    /// and are factored here.
    ///
    /// Decl-form dependants (ConstRefersTo, ShapeRefersTo) need a
    /// fresh name on every invocation, hence the `j` suffix.
    /// AssignmentLhs is a statement; `j` is unused for that variant
    /// (re-assigning `c_i` would be a DoubleAssignment defect against
    /// an *unpoisoned* data symbol, but the poisoned `c_i` is never
    /// in `ir.data` so the single-assignment ledger is never touched
    /// — verified empirically during fixture development).
    fn render_non_barecall_dependant(trigger: Trigger, i: usize, j: usize) -> String {
        match trigger {
            Trigger::BareCallRead => unreachable!(
                "BareCallRead is rendered inline at the call site \
                 because it is the one trigger that varies by \
                 cascade-kind; render_non_barecall_dependant must not \
                 be called for it"
            ),
            Trigger::AssignmentLhs => {
                let _ = j;
                format!("c{i} <-- sk();\n")
            }
            Trigger::ConstRefersTo => {
                format!("const ref_{i}_{j} : usize = c{i} + 1;\n")
            }
            Trigger::ShapeRefersTo => {
                format!("data ref_{i}_{j} : f32[c{i}];\n")
            }
        }
    }

    /// The four LowerError variant kinds that the cascade rule
    /// suppresses. Any leak of any of these is a regression.
    fn is_cascade_suppressible(kind: &LowerErrorKind) -> bool {
        matches!(
            kind,
            LowerErrorKind::UnknownIdent(_)
                | LowerErrorKind::AssignmentTargetNotData(_)
                | LowerErrorKind::ConstRefersToNonConst { .. }
                | LowerErrorKind::ShapeRefersToNonConst { .. }
        )
    }

    let cascade_kinds = [
        CascadeKind::DataViaShape,
        CascadeKind::KernelViaSignatureShape,
        CascadeKind::ConstViaOtherConst,
    ];
    let triggers = [
        Trigger::BareCallRead,
        Trigger::AssignmentLhs,
        Trigger::ConstRefersTo,
        Trigger::ShapeRefersTo,
    ];

    for cascade_kind in cascade_kinds {
        for trigger in triggers {
            for k in [1usize, 2, 3, 5] {
                for l in [1usize, 2, 3] {
                    // Root poison: `const N` divides by zero.
                    let mut src = String::from("const N : usize = 1 / 0;\n");

                    // Sink kernel for AssignmentLhs (every kind) and for
                    // BareCallRead on data/const cascade-decls.
                    src.push_str("kernel sk : () -> f32[1] effectful;\n");
                    // BareCallRead sinks differ by cascade-kind:
                    // - data: `dump_arr(f32[N])` — takes the cascade-
                    //   data's f32[N] shape. The sink ITSELF references
                    //   N in its signature; that signature lowering
                    //   fails (ShapeRefersToNonConst{sk_name, N}),
                    //   triggering case-1 transitive poison of the sink
                    //   name. The bare-call then suppresses through
                    //   the sink-name cascade — same root.
                    // - kernel: the cascade-kernel IS the callee.
                    // - const: `dump_int(i32)` — takes a scalar i32.
                    src.push_str("kernel dump_arr : (f32[N]) -> () effectful;\n");
                    src.push_str("kernel dump_int : (i32) -> () effectful;\n");

                    // K cascade-decls of the chosen kind.
                    for i in 0..k {
                        src.push_str(&render_cascade_decl(cascade_kind, i));
                    }

                    // L dependants per cascade-decl.
                    for i in 0..k {
                        for j in 0..l {
                            let dep = match trigger {
                                Trigger::BareCallRead => match cascade_kind {
                                    CascadeKind::DataViaShape => {
                                        format!("dump_arr(c{i});\n")
                                    }
                                    CascadeKind::KernelViaSignatureShape => {
                                        // Bare-call OF the cascade-
                                        // kernel itself. `j` is unused
                                        // (every statement is the same
                                        // bare call).
                                        let _ = j;
                                        format!("c{i}();\n")
                                    }
                                    CascadeKind::ConstViaOtherConst => {
                                        format!("dump_int(c{i});\n")
                                    }
                                },
                                _ => render_non_barecall_dependant(trigger, i, j),
                            };
                            src.push_str(&dep);
                        }
                    }

                    let errs = lower_str(&src).expect_err("the root failed const must error");

                    // AC#1 + AC#2: EXACTLY 1 error for every cell.
                    assert_eq!(
                        errs.errors().len(),
                        1,
                        "cascade_kind={cascade_kind:?} trigger={trigger:?} \
                         K={k} L={l} must collapse to EXACTLY 1 error (the \
                         root ConstDivByZero{{N}}), got {} kinds={:?} — \
                         source:\n{src}",
                        errs.errors().len(),
                        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>(),
                    );

                    // The sole survivor is the root.
                    let only = &errs.errors()[0];
                    match &only.kind {
                        LowerErrorKind::ConstDivByZero { in_const } => {
                            assert_eq!(
                                in_const, "N",
                                "cascade_kind={cascade_kind:?} \
                                 trigger={trigger:?} K={k} L={l}: \
                                 the sole error must be the root \
                                 ConstDivByZero(N)"
                            );
                        }
                        other => panic!(
                            "cascade_kind={cascade_kind:?} \
                             trigger={trigger:?} K={k} L={l}: the sole \
                             error must be the root ConstDivByZero(N), \
                             got {other:?} — source:\n{src}"
                        ),
                    }

                    // Span of the surviving error must point at the
                    // root `1 / 0` (mirrors the cycle-3 fixture's
                    // span-pin idiom and the
                    // `effect_stmt_to_declared_but_failed_kernel`
                    // discrimination — without it, a regression that
                    // emitted a cascade-error at a downstream span
                    // could still match `ConstDivByZero(N)` if it
                    // re-used the name).
                    let div_at = src.find("1 / 0").expect("`1 / 0` in source");
                    let expected = offset_to_line_col(&src, div_at);
                    let actual_span = only.span.clone().unwrap_or_else(|| {
                        panic!(
                            "root error must carry a span — \
                             cascade_kind={cascade_kind:?} \
                             trigger={trigger:?} K={k} L={l}"
                        )
                    });
                    let actual = offset_to_line_col(&src, actual_span.start);
                    assert_eq!(
                        actual, expected,
                        "cascade_kind={cascade_kind:?} \
                         trigger={trigger:?} K={k} L={l}: root error \
                         span must point at the `1 / 0` of N, not a \
                         downstream cascade — actual {actual:?} \
                         expected {expected:?}"
                    );

                    // Anti-leak guard: NO error of any of the four
                    // cascade-suppressible kinds may survive. This is
                    // the structural assertion the orchestrator
                    // requested — even though len==1 + kind-match
                    // already pins this, the explicit anti-leak
                    // pattern documents *which* leak each cell
                    // defends against and is the
                    // pattern-matchable invariant for a reviewer.
                    let leaked: Vec<_> = errs
                        .errors()
                        .iter()
                        .filter(|e| is_cascade_suppressible(&e.kind))
                        .map(|e| e.kind.clone())
                        .collect();
                    assert!(
                        leaked.is_empty(),
                        "cascade_kind={cascade_kind:?} \
                         trigger={trigger:?} K={k} L={l}: no \
                         cascade-suppressible variant (UnknownIdent / \
                         AssignmentTargetNotData / ConstRefersToNonConst / \
                         ShapeRefersToNonConst) may leak — found \
                         {leaked:?} — source:\n{src}"
                    );
                }
            }
        }
    }
}

// --------------------------------------------------------------------
// TASK-0206: cascade-aware duplicate detection.
//
// PRE-EXISTING latent gap surfaced during the TASK-0092 cycle-3
// review. Before this task, `DuplicateConst` / `DuplicateData` /
// `DuplicateKernel` consulted only `ir.consts` / `ir.data` /
// `ir.kernels` — the *successful* symbol tables. A second decl of a
// name whose first decl FAILED (so the name was in `failed_decls`,
// not `ir.X`) silently passed the duplicate check and was treated as
// the first valid decl. Net: the second decl could ITSELF fail (e.g.
// re-evaluating a bad RHS) or succeed and leave a stale symbol that
// shadowed the user's intent — either way, the duplicate violation
// was lost.
//
// Decision (TASK-0206 notes): FIX. PRD §6.2.1's single-assignment
// keyed by symbol name, combined with §6.2.3's one-namespace rule,
// implies the source-text re-use of the name is the violation —
// independent of whether the first decl evaluated. The cascade-poison
// of the original stays in `failed_decls`, so downstream references
// remain cascade-suppressed; the duplicate detection now consults the
// UNION of (successful symbol tables, `failed_decls`).
//
// Counting contract update (see `lower_algo` docstring):
//   K poisoned roots × M duplicate-of-failed re-decls → K + K*M errors.
//
// The fixture below pins this across K ∈ {1,2,3,5}, M ∈ {1,2,3} and
// all three decl-kinds (const / data / kernel) — the same multi-axis
// discipline that closed TASK-0092 cycle-3.
// --------------------------------------------------------------------

/// AC#1/#2/#3 — duplicate-of-failed-decl detection is **cascade-aware
/// AND size-parametric**. K poisoned root decls, each re-declared M
/// times after the failed first, yield EXACTLY `K + K*M` errors: K
/// root failures + K*M `DuplicateX` for the re-decls. No cascade
/// suppression of `DuplicateX` (it is an independent violation of
/// name-uniqueness, NOT a reference-resolution cascade). No leak of
/// cascade-suppressible variants on dependents of the poisoned name.
///
/// **Three cascade-decl kinds** (the decl whose first attempt fails):
/// - const-via-div-by-zero — `const N_i : usize = 1 / 0;` (independent
///   root; not via another const, so case-1 transitive-poison is not
///   involved — keeps the fixture focused on the duplicate-of-failed
///   axis without entangling with TASK-0092 transitive logic).
/// - data-via-bad-shape — `data D_i : f32[BAD];` where BAD is an
///   undeclared name (`ShapeRefersToNonConst` on the data name; the
///   data name itself goes into `failed_decls` because the error is
///   NOT a cascade of another failed decl).
/// - kernel-via-bad-signature — `kernel K_i : (f32[BAD]) -> ()
///   effectful;` (analogous: kernel name into `failed_decls`).
///
/// **M duplicate re-decls per root** — each re-decl uses the SAME
/// kind (const re-decl of const, data re-decl of data, kernel re-decl
/// of kernel) with a *valid* RHS that would lower cleanly on its own
/// (so the cascade-aware duplicate detection is the ONLY thing
/// catching it). The valid RHS is the discriminating choice: if the
/// re-decl ALSO had a bad RHS, the test would not distinguish "fix
/// fired" from "second decl also independently fails".
///
/// **Discrimination strength.** Each cell asserts:
/// - `errors().len() == K + K*M` exactly,
/// - K errors are the root-kind (`ConstDivByZero` /
///   `ShapeRefersToNonConst` on the root name),
/// - K*M errors are `DuplicateX` for the matching kind, naming the
///   root identifier,
/// - the order is source order (root-fail at position i precedes its
///   M `DuplicateX` re-decls, all in source order).
///
/// **Negative-control.** A separate cell with the same K roots but
/// ZERO duplicate re-decls yields exactly K errors (the roots only)
/// — this catches a regression where `is_failed_decl` over-fires on a
/// fresh second name (false-positive duplicate).
#[test]
fn duplicate_of_failed_decl_fires_for_any_k_m() {
    #[derive(Clone, Copy, Debug)]
    enum DeclKind {
        Const,
        Data,
        Kernel,
    }

    /// Render the FAILED first decl of name `n_i` of the given kind.
    /// The failure is *independent* (no `failed_decls` entry consulted
    /// for the failure itself), so the name is poisoned via case-3 of
    /// `Accum::record_decl_failure`, not case-1 — keeping this fixture
    /// orthogonal to the TASK-0092 transitive-poison axis.
    fn render_failed_first(kind: DeclKind, i: usize) -> String {
        match kind {
            DeclKind::Const => format!("const n{i} : usize = 1 / 0;\n"),
            // Data with a shape referencing an undeclared `BAD_i`.
            DeclKind::Data => format!("data n{i} : f32[BAD_{i}];\n"),
            DeclKind::Kernel => {
                format!("kernel n{i} : (f32[BAD_{i}]) -> () effectful;\n")
            }
        }
    }

    /// Render the j-th valid-RHS duplicate re-decl of `n_i`. The RHS
    /// is well-formed in isolation; the ONLY reason it errors is the
    /// cascade-aware duplicate check (TASK-0206).
    fn render_valid_redecl(kind: DeclKind, i: usize, j: usize) -> String {
        // `j` is unused for the content (every re-decl is structurally
        // the same valid form), but rendered into the source for
        // textual uniqueness — keeps any future dedup heuristic from
        // collapsing rows. Add as a trailing `// j=...` comment so the
        // parser ignores it.
        match kind {
            // Distinct integer values so a regression that silently
            // inserts the LAST re-decl into `ir.consts` would be
            // catchable downstream (we don't assert it here; the
            // duplicate-error count is the load-bearing assertion).
            DeclKind::Const => format!("const n{i} : usize = {};\n", 1 + j),
            DeclKind::Data => format!("data n{i} : f32[4];\n"),
            DeclKind::Kernel => format!("kernel n{i} : () -> () effectful;\n"),
        }
    }

    fn root_kind_matches(kind: DeclKind, lk: &LowerErrorKind, name: &str) -> bool {
        match kind {
            DeclKind::Const => matches!(
                lk,
                LowerErrorKind::ConstDivByZero { in_const } if in_const == name
            ),
            DeclKind::Data => matches!(
                lk,
                LowerErrorKind::ShapeRefersToNonConst { decl, .. } if decl == name
            ),
            DeclKind::Kernel => matches!(
                lk,
                LowerErrorKind::ShapeRefersToNonConst { decl, .. } if decl == name
            ),
        }
    }

    fn dup_kind_matches(kind: DeclKind, lk: &LowerErrorKind, name: &str) -> bool {
        match kind {
            DeclKind::Const => matches!(
                lk,
                LowerErrorKind::DuplicateConst(n) if n == name
            ),
            DeclKind::Data => matches!(
                lk,
                LowerErrorKind::DuplicateData(n) if n == name
            ),
            DeclKind::Kernel => matches!(
                lk,
                LowerErrorKind::DuplicateKernel(n) if n == name
            ),
        }
    }

    let kinds = [DeclKind::Const, DeclKind::Data, DeclKind::Kernel];
    let ks = [1usize, 2, 3, 5];
    let ms = [0usize, 1, 2, 3];

    for kind in kinds {
        for k in ks {
            for m in ms {
                let mut src = String::new();
                // K poisoned roots followed by M re-decls each, in
                // source order so the assertion can index them.
                for i in 0..k {
                    src.push_str(&render_failed_first(kind, i));
                    for j in 0..m {
                        src.push_str(&render_valid_redecl(kind, i, j));
                    }
                }

                let errs =
                    lower_str(&src).expect_err("a poisoned root must produce at least one error");

                let expected = k + k * m;
                assert_eq!(
                    errs.errors().len(),
                    expected,
                    "kind={kind:?} K={k} M={m}: expected {expected} errors \
                     (K roots + K*M DuplicateX), got {} — kinds={:?} — \
                     source:\n{src}",
                    errs.errors().len(),
                    errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>(),
                );

                // Source-order discrimination: for each root i, the
                // i-th block of (1 + M) errors is (root, dup_0,
                // dup_1, ..., dup_{M-1}).
                let all = errs.errors();
                for i in 0..k {
                    let block_start = i * (1 + m);
                    let root_name = format!("n{i}");

                    // Root error.
                    let root_err = &all[block_start];
                    assert!(
                        root_kind_matches(kind, &root_err.kind, &root_name),
                        "kind={kind:?} K={k} M={m} i={i}: root error at \
                         position {block_start} must be the failed-first \
                         decl of `{root_name}`, got {:?} — source:\n{src}",
                        root_err.kind
                    );

                    // M duplicate re-decls.
                    for j in 0..m {
                        let pos = block_start + 1 + j;
                        let dup_err = &all[pos];
                        assert!(
                            dup_kind_matches(kind, &dup_err.kind, &root_name),
                            "kind={kind:?} K={k} M={m} i={i} j={j}: \
                             duplicate at position {pos} must be \
                             Duplicate*({root_name}), got {:?} — \
                             source:\n{src}",
                            dup_err.kind
                        );
                    }
                }

                // Anti-leak: no cascade-suppressible kind on a
                // dependent of the poisoned name. (This fixture has
                // no dependents beyond the re-decls themselves; the
                // assertion is a structural guard against a future
                // regression that accidentally re-classifies a
                // duplicate as a cascade-suppressible.)
                //
                // Exception: for the Data and Kernel root-failure
                // shapes, the root error itself IS a
                // `ShapeRefersToNonConst` — that is the ROOT error,
                // not a cascade leak. We exclude root errors from
                // the leak count by name: leaked errors are
                // ShapeRefersToNonConst entries whose `decl` field
                // does NOT match any `n{i}` root name.
                let root_names: std::collections::HashSet<String> =
                    (0..k).map(|i| format!("n{i}")).collect();
                let leaked: Vec<_> = all
                    .iter()
                    .filter(|e| match &e.kind {
                        LowerErrorKind::UnknownIdent(_)
                        | LowerErrorKind::AssignmentTargetNotData(_)
                        | LowerErrorKind::ConstRefersToNonConst { .. } => true,
                        LowerErrorKind::ShapeRefersToNonConst { decl, .. } => {
                            !root_names.contains(decl)
                        }
                        _ => false,
                    })
                    .map(|e| e.kind.clone())
                    .collect();
                assert!(
                    leaked.is_empty(),
                    "kind={kind:?} K={k} M={m}: no cascade-suppressible \
                     variant may leak past the duplicate-detection arms — \
                     found {leaked:?} — source:\n{src}"
                );
            }
        }
    }
}

/// AC#3 narrow fixtures — the exact two source strings called out in
/// the task description. Pinned separately from the parametric sweep
/// so a regression on the headline cases produces an immediately
/// readable diagnostic.
#[test]
fn duplicate_const_after_failed_first_const_fires_exactly_two_errors() {
    // The headline case from TASK-0206: `const N = 1/0; const N = 7;`
    // must produce EXACTLY two errors — the DivByZero AND the
    // DuplicateConst — not just the DivByZero.
    let src = "\
const N : usize = 1 / 0;
const N : usize = 7;
";
    let errs = lower_str(src).expect_err("must error");
    assert_eq!(
        errs.errors().len(),
        2,
        "expected exactly 2 errors (DivByZero + DuplicateConst), got {} kinds={:?}",
        errs.errors().len(),
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>(),
    );
    assert!(
        matches!(
            &errs.errors()[0].kind,
            LowerErrorKind::ConstDivByZero { in_const } if in_const == "N"
        ),
        "first error must be ConstDivByZero(N); got {:?}",
        errs.errors()[0].kind
    );
    assert!(
        matches!(
            &errs.errors()[1].kind,
            LowerErrorKind::DuplicateConst(n) if n == "N"
        ),
        "second error must be DuplicateConst(N); got {:?}",
        errs.errors()[1].kind
    );
}

#[test]
fn duplicate_data_after_failed_first_data_fires_exactly_two_errors() {
    // The companion data-shape headline case from TASK-0206.
    let src = "\
data x : f32[BAD];
data x : f32[4];
";
    let errs = lower_str(src).expect_err("must error");
    assert_eq!(
        errs.errors().len(),
        2,
        "expected exactly 2 errors (ShapeRefersToNonConst + DuplicateData), got {} kinds={:?}",
        errs.errors().len(),
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>(),
    );
    assert!(
        matches!(
            &errs.errors()[0].kind,
            LowerErrorKind::ShapeRefersToNonConst { decl, unknown_ident }
                if decl == "x" && unknown_ident == "BAD"
        ),
        "first error must be ShapeRefersToNonConst{{decl=x, unknown_ident=BAD}}; got {:?}",
        errs.errors()[0].kind
    );
    assert!(
        matches!(
            &errs.errors()[1].kind,
            LowerErrorKind::DuplicateData(n) if n == "x"
        ),
        "second error must be DuplicateData(x); got {:?}",
        errs.errors()[1].kind
    );
}

/// AC#3 second clause — "cascade chains downstream of the now-redeclared
/// name still suppress correctly (no new cascade-class regression)".
/// A statement that bare-call-reads the re-declared name must STILL be
/// suppressed: the second decl errored at the duplicate check before
/// reaching evaluation, so `ir.X` is unchanged and the name remains in
/// `failed_decls`. Downstream references continue to cascade-suppress
/// against the original root failure.
#[test]
fn redecl_of_failed_does_not_unpoison_downstream_cascade() {
    // `const N = 1/0;` poisons N. A re-decl `const N = 7;` fires
    // DuplicateConst but does NOT unpoison N. Then `data x : f32[N];`
    // would, absent suppression, emit ShapeRefersToNonConst{x, N} —
    // but it must be cascade-suppressed against the original poison.
    let src = "\
const N : usize = 1 / 0;
const N : usize = 7;
data x : f32[N];
";
    let errs = lower_str(src).expect_err("must error");
    // Expect EXACTLY 2: ConstDivByZero(N) + DuplicateConst(N). The
    // ShapeRefersToNonConst on `data x` must NOT leak.
    assert_eq!(
        errs.errors().len(),
        2,
        "expected exactly 2 errors (root + duplicate; downstream `data x` \
         cascade-suppressed); got {} kinds={:?}",
        errs.errors().len(),
        errs.errors().iter().map(|e| &e.kind).collect::<Vec<_>>(),
    );
    let leaked: Vec<_> = errs
        .errors()
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                LowerErrorKind::ShapeRefersToNonConst { .. }
                    | LowerErrorKind::UnknownIdent(_)
                    | LowerErrorKind::ConstRefersToNonConst { .. }
                    | LowerErrorKind::AssignmentTargetNotData(_)
            )
        })
        .map(|e| e.kind.clone())
        .collect();
    assert!(
        leaked.is_empty(),
        "downstream cascade must remain suppressed after a duplicate-of-failed re-decl; \
         found leaked {leaked:?}"
    );
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
// effectful load/capture kernel on the RHS of `<--`. The spec decision
// is recorded as `backlog/decisions/decision-0004` (grammar §2 note 5
// is canonical, unidirectional). The positive test
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
/// See TASK-0089 onboarding notes and decision-0004 for the spec context.
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
    assert_eq!(
        expected,
        (4, 1),
        "sanity: bare-call `pure_k` is at line 4 col 1"
    );
    let span = err
        .span
        .clone()
        .expect("EffectCalleeNotEffectful carries a span");
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
///
/// Assertion-strength (TASK-0202): the per-violation `(line, col)` is
/// pinned via `offset_to_line_col`, analogous to the singular-violation
/// template `located_effect_purity_error_has_correct_line_col`. A
/// regression placing both spans at DISTINCT-BUT-WRONG positions (the
/// "wrong-token-on-the-line" bug class) would pass the older
/// `spans[0] != spans[1]` blanket but bites here.
///
/// Why each violation lands at column 1: the `Stmt::Effect` arm in
/// `algo/lower.rs` (commit 6e77fce) emits `EffectCalleeNotEffectful`
/// with `call.callee.span` — i.e. the span of the callee `Ident` in
/// the parsed `Call`, which starts at the identifier's first
/// character. Effect-statement syntax is `<callee>(<args>);` with no
/// leading token, so for an un-indented effect statement the callee
/// start coincides with the statement start, i.e. column 1.
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
    // Each error carries its own callee span. Pin the EXACT
    // (line, col) per violation via `offset_to_line_col` against the
    // source — not a guessed constant — so the test bites against a
    // "wrong token on the line" regression that would still produce
    // two distinct spans (defeating the older `spans[0] != spans[1]`
    // assertion). TASK-0202.
    let spans: Vec<_> = errs
        .errors()
        .iter()
        .filter_map(|e| e.span.clone())
        .collect();
    assert_eq!(spans.len(), 2, "both errors must carry spans");

    // `pure_a` appears three times in the source: line 5 (kernel
    // decl), line 8 (dataflow RHS — legal), line 9 (effect-stmt —
    // violation). The violation is the THIRD occurrence (index 2).
    let pure_a_violation_off = src
        .match_indices("pure_a")
        .nth(2)
        .expect("three `pure_a` occurrences in source")
        .0;
    let expected_a = offset_to_line_col(src, pure_a_violation_off);
    assert_eq!(
        expected_a,
        (9, 1),
        "sanity: bare-call `pure_a` is at line 9 col 1"
    );

    // `pure_b` appears twice: line 6 (kernel decl), line 10
    // (effect-stmt — violation). The violation is the SECOND
    // occurrence (index 1).
    let pure_b_violation_off = src
        .match_indices("pure_b")
        .nth(1)
        .expect("two `pure_b` occurrences in source")
        .0;
    let expected_b = offset_to_line_col(src, pure_b_violation_off);
    assert_eq!(
        expected_b,
        (10, 1),
        "sanity: bare-call `pure_b` is at line 10 col 1"
    );

    // The two expected positions are distinct (defence-in-depth: if
    // the source layout drifts so both violations land on the same
    // line, the per-site assertion below is no longer discriminating).
    assert_ne!(
        expected_a, expected_b,
        "the two expected violation positions must differ \
         (else the per-site assertions below collapse)"
    );

    assert_eq!(
        offset_to_line_col(src, spans[0].start),
        expected_a,
        "first violation (`pure_a`) must point at the bare-call callee \
         at line 9 col 1"
    );
    assert_eq!(
        offset_to_line_col(src, spans[1].start),
        expected_b,
        "second violation (`pure_b`) must point at the bare-call callee \
         at line 10 col 1"
    );
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

    let errs = lower_str(src).expect_err("the failed const must produce its root error");

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
        expected,
        (1, 27),
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
    let leaked_unknown_ident = errs
        .errors()
        .iter()
        .any(|e| matches!(&e.kind, LowerErrorKind::UnknownIdent(n) if n == "bad_kernel"));
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

// --------------------------------------------------------------------
// TASK-0205: for-body independent-error preservation under
// cascade-poisoned bounds.
//
// Surfaced during TASK-0092 cycle-3 review (qa-test-runner finding #1).
// In the pre-TASK-0205 code, `lower_stmt` for `Stmt::For` evaluated
// `lo` and `hi` with `?`-propagation. When either bound referenced a
// cascade-poisoned name (`const BAD = 1 / 0; const X = BAD + 1; for i :
// 0 .. X { … }`), the for-statement returned `Err` **before** the body
// was visited. The bound-error was itself a cascade and got
// cascade-suppressed at the top level — but **any GENUINELY-independent
// error inside the body** (a never-declared kernel call, a separate
// div-by-zero, …) was never reached and therefore silently lost.
//
// This is an **undercount** class, NOT a cascade-class regression: the
// TASK-0092 K + K*L counting contract did not claim to cover
// "independent errors inside a body whose for has a poisoned bound".
// But it is adjacent enough that reviewers may misclassify it as a 6th
// cascade-class recurrence; the cycle-3 docstring rewrite did not call
// it out as a documented exception.
//
// **Decision: FIX** (TASK-0205 AC#1). The TASK-0092 cycle-3 invariant
// is "independent errors must STILL be reported"; the for-body case
// must respect it. Implementation: `lower_for_into` (in
// `algo/lower.rs`) always descends into the body with the iter-var in
// scope, accumulating body-statement errors through
// `Accum::record_stmt_error` (which preserves cascade-suppression for
// references to poisoned names). The for-statement emits an
// `IrStmt::For` only if everything (bounds + body) succeeded; on any
// failure the partial IR is dropped.
//
// **Counting contract addendum (load-bearing — extends the lower_algo
// docstring's K + K*L rule):**
//
//   - 1 cascade-poisoned const root + K cascade-scoped for-loops each
//     with M genuinely-independent body errors →
//     EXACTLY 1 + K*M errors (1 root + K*M independents).
//
// The pre-TASK-0205 measurement was 1 (root only) for every K, M.
// --------------------------------------------------------------------

/// TASK-0205 AC#1/#2/#3 — independent body errors survive a
/// cascade-poisoned for-bound, **size-parametric** over K (number of
/// cascade-scoped for-loops) and M (number of independent body errors
/// per loop).
///
/// **Reproducer fidelity.** The canonical case from the task brief is
/// the (K=1, M=1) cell:
///
/// ```text
/// const BAD : usize = 1 / 0;
/// const X : usize = BAD + 1;
/// data y : f32[10];
/// for i_0 : 0 .. X {
///   never_k_0_0(y);
/// }
/// ```
///
/// Pre-TASK-0205 emitted 1 error (the root `ConstDivByZero(BAD)`).
/// Post-TASK-0205 emits exactly 2 (the root + the independent
/// `UnknownIdent("never_k_0_0")`).
///
/// **Why parametric in BOTH K and M (single-shape masking is the
/// recurrence the project keeps re-learning).** Memory
/// `feedback-comment-doc-lie-recurring` and the TASK-0092 cycle-3/4/5/6
/// methodology demand size-parametric pinning over BOTH axes of the
/// defect class: a fixed-K-M fixture could mask an off-by-one (one
/// independent dropped per loop), a per-loop short-circuit
/// (independents-per-loop > 1 lost), or a per-cascade-bound conflation
/// (multiple cascade-poisoned for-loops collapsed to one). The K × M
/// sweep below catches all three.
///
/// **Iter-var poisoning.** The brief calls out "do NOT emit cascade
/// errors from references to the dead iter-var". Implementation note:
/// when bound-eval fails, `lower_for_into` still calls
/// `scope.push_loop(var)` before descending, so iter-var references
/// resolve cleanly in the body (`IrExpr::Ident(name)`) — there are no
/// errors to suppress; the natural scoping rule handles it. The
/// `iter_var_use_in_body_of_cascade_scoped_loop_is_clean` cell below
/// pins this: a body that uses the iter-var as an index emits ONLY the
/// root error, not a spurious `IterVarOutOfScope` / `UnknownIdent` on
/// `i`.
///
/// **Discrimination strength.** Each (K, M) cell asserts:
/// - `errors().len() == 1 + K*M` (exact equality, not `>= 1`),
/// - the first error is the root `ConstDivByZero(BAD)` (source order),
/// - the next K*M errors are `UnknownIdent("never_k_{f}_{m}")` for
///   each (f ∈ 0..K, m ∈ 0..M), in source order — no collapsing, no
///   reordering, no off-by-one.
/// - NO error of any cascade-suppressible kind for `BAD` or `X` leaks
///   (the bound-evaluation cascade must stay suppressed).
///
/// **Negative-control (M=0).** A body with **zero** independent errors
/// (just an iter-var use, or an empty body) emits EXACTLY 1 error
/// (the root only). This catches a regression that would emit a phantom
/// error per cascade-poisoned for-loop even when its body is clean.
#[test]
fn for_body_independents_survive_cascade_poisoned_bound_for_any_k_m() {
    /// Render a source program with one cascade-poisoned root (`const
    /// BAD = 1 / 0; const X = BAD + 1;`), one shared data symbol, and
    /// `K` for-loops each with `M` independent never-declared-kernel
    /// calls in its body. Loop names are `i_0 .. i_{K-1}`; body errors
    /// are `never_k_{f}_{m}` so the assertion can pin source order.
    fn render(k: usize, m: usize) -> String {
        let mut src = String::from(
            "const BAD : usize = 1 / 0;\n\
             const X : usize = BAD + 1;\n\
             data y : f32[10];\n",
        );
        for f in 0..k {
            src.push_str(&format!("for i_{f} : 0 .. X {{\n"));
            for mj in 0..m {
                src.push_str(&format!("  never_k_{f}_{mj}(y);\n"));
            }
            src.push_str("}\n");
        }
        src
    }

    /// Anti-leak: no cascade-suppressible variant naming `BAD` or `X`
    /// may survive the cascade discipline. The bound-evaluation cascade
    /// (`ConstRefersToNonConst { unknown_ident: "BAD" }` or
    /// `UnknownIdent("X")`) must stay suppressed.
    fn no_bound_cascade_leaks(errs: &[LowerError]) -> bool {
        errs.iter().all(|e| match &e.kind {
            LowerErrorKind::UnknownIdent(n) => n != "BAD" && n != "X",
            LowerErrorKind::AssignmentTargetNotData(n) => n != "BAD" && n != "X",
            LowerErrorKind::ConstRefersToNonConst { unknown_ident, .. } => {
                unknown_ident != "BAD" && unknown_ident != "X"
            }
            LowerErrorKind::ShapeRefersToNonConst { unknown_ident, .. } => {
                unknown_ident != "BAD" && unknown_ident != "X"
            }
            _ => true,
        })
    }

    // K ∈ {1, 2, 3} cascade-poisoned for-loops, M ∈ {0, 1, 2, 3}
    // independent body errors per loop. M=0 is the negative control
    // (clean body → root error only). The pre-TASK-0205 fixed cell
    // would always measure 1 here; the K × M sweep makes the 1+K*M rule
    // measurement-backed across the defect's two dimensions.
    for k in [1usize, 2, 3] {
        for m in [0usize, 1, 2, 3] {
            let src = render(k, m);
            let errs = lower_str(&src).expect_err("the root failed const must produce its error");
            let all = errs.errors();

            let expected = 1 + k * m;
            assert_eq!(
                all.len(),
                expected,
                "K={k} M={m}: expected {expected} errors (1 root + K*M \
                 independent body errors), got {} — kinds={:?} — \
                 source:\n{src}",
                all.len(),
                all.iter().map(|e| &e.kind).collect::<Vec<_>>(),
            );

            // Position 0: the root.
            assert!(
                matches!(
                    &all[0].kind,
                    LowerErrorKind::ConstDivByZero { in_const } if in_const == "BAD"
                ),
                "K={k} M={m}: first error must be the root \
                 ConstDivByZero(BAD) in source order, got {:?}",
                all[0].kind
            );

            // Positions 1..=K*M: the K*M body independents, in source
            // order. The source-order layout is `for_0 body_0..M-1,
            // for_1 body_0..M-1, ...` so the (f * M + mj + 1)-th error
            // is `never_k_{f}_{mj}`.
            for f in 0..k {
                for mj in 0..m {
                    let pos = 1 + f * m + mj;
                    let want = format!("never_k_{f}_{mj}");
                    let got = &all[pos];
                    assert!(
                        matches!(
                            &got.kind,
                            LowerErrorKind::UnknownIdent(n) if n == &want
                        ),
                        "K={k} M={m} f={f} mj={mj}: error at position {pos} \
                         must be UnknownIdent({want:?}) in source order, \
                         got {:?} — source:\n{src}",
                        got.kind,
                    );
                }
            }

            // Bound-cascade anti-leak: nothing referencing BAD or X
            // (the poisoned chain) may survive. The bound-evaluation
            // error is itself a cascade and must be suppressed; only
            // the root and the body-independents may appear.
            assert!(
                no_bound_cascade_leaks(all),
                "K={k} M={m}: no cascade-suppressible variant naming \
                 `BAD` or `X` may leak — kinds={:?} — source:\n{src}",
                all.iter().map(|e| &e.kind).collect::<Vec<_>>(),
            );
        }
    }
}

/// TASK-0205 — iter-var poisoning interaction. A body that **uses the
/// iter-var** of a cascade-scoped for-loop (as an index into a data
/// symbol) must emit no spurious diagnostic on the iter-var itself —
/// uses of `i` inside the body resolve cleanly via the lexical scope
/// rule (PRD §6.2.3) regardless of whether the loop bounds evaluated.
///
/// **What this pins.** Without the FIX, the for-statement returned Err
/// before `push_loop` ran, so the body was never visited and the
/// question "does iter-var-use-in-body emit a spurious error?" did not
/// arise. With the FIX, the body IS visited; this fixture pins that
/// iter-var references stay clean. The only error must be the cascade
/// root.
///
/// **PRD §6.2.3 verbatim (lines 318-323):**
/// > **Name resolution.** Iteration variables and data variables share
/// > one namespace. Iteration variables shadow at their loop and go out
/// > of scope at the loop's end. A name `y` inside a `for y : ...` body
/// > always refers to the iteration variable; outside, it refers to
/// > whatever `data y : ...` declared (or is undefined). No `@`-style
/// > prefix; the compiler disambiguates by scope.
///
/// The "always refers to the iteration variable" rule is unconditional
/// — it does not depend on whether the bounds evaluated successfully.
/// The FIX honors this by `push_loop`-ing regardless of bound success.
#[test]
fn iter_var_use_in_body_of_cascade_scoped_loop_is_clean() {
    // Body uses iter-var `i` as an index into data `y`; this would
    // produce a clean `IrStmt::Dataflow` if the for-statement lowered
    // — and produces NO error on `i` even when the for is doomed.
    // The `dump` kernel is declared with a clean scalar signature so
    // its decl does not itself fail.
    let src = "\
const BAD : usize = 1 / 0;
const X : usize = BAD + 1;
data y : f32[10];
kernel dump : (f32) -> () effectful;
for i : 0 .. X {
  dump(y[i]);
}
";
    let errs = lower_str(src).expect_err("the root failed const must error");
    let all = errs.errors();

    // EXACTLY 1 — the root. The body's `dump(y[i])` is lowerable in
    // isolation (no never-declared idents, no double-assignment), so
    // it contributes ZERO errors. The bound cascade is suppressed.
    assert_eq!(
        all.len(),
        1,
        "iter-var-use-in-body must contribute no errors; expected 1 \
         (root only), got {} — kinds={:?} — source:\n{src}",
        all.len(),
        all.iter().map(|e| &e.kind).collect::<Vec<_>>(),
    );
    assert!(
        matches!(
            &all[0].kind,
            LowerErrorKind::ConstDivByZero { in_const } if in_const == "BAD"
        ),
        "the sole error must be the root ConstDivByZero(BAD), got {:?}",
        all[0].kind,
    );

    // Discriminating anti-leak: NO IterVarOutOfScope, NO
    // UnknownIdent("i"), NO ShapeRefersToNonConst on the iter-var.
    // These are the variants a regression that didn't push_loop would
    // emit.
    let iter_var_leaked = all.iter().any(|e| match &e.kind {
        LowerErrorKind::IterVarOutOfScope(n) => n == "i",
        LowerErrorKind::UnknownIdent(n) => n == "i",
        _ => false,
    });
    assert!(
        !iter_var_leaked,
        "no error referencing the iter-var `i` may leak — the FIX \
         must push_loop regardless of bound-eval success — kinds={:?}",
        all.iter().map(|e| &e.kind).collect::<Vec<_>>(),
    );
}

/// TASK-0205 — nested for, **inner** for has cascade-poisoned bound.
/// Outer for has clean bounds; inner for's bound references the
/// poisoned name. The inner body has K=1 independent error.
///
/// **What this pins.** The FIX must work for arbitrarily-nested
/// for-statements, not just top-level. The recursive
/// `lower_stmt_into` dispatch routes every nested For through
/// `lower_for_into`, so the body-descent invariant holds at any
/// nesting depth.
#[test]
fn nested_for_inner_cascade_bound_still_surfaces_inner_body_independents() {
    let src = "\
const BAD : usize = 1 / 0;
const X : usize = BAD + 1;
const N : usize = 4;
data y : f32[10];
for outer_i : 0 .. N {
  for inner_j : 0 .. X {
    never_k(y);
  }
}
";
    let errs = lower_str(src).expect_err("the root failed const must error");
    let all = errs.errors();

    // Expected: 2 — the root + the inner body's `never_k`.
    assert_eq!(
        all.len(),
        2,
        "nested for with cascade-poisoned INNER bound must still \
         surface the inner body's independent error — expected 2 \
         (root + 1 independent), got {} — kinds={:?} — source:\n{src}",
        all.len(),
        all.iter().map(|e| &e.kind).collect::<Vec<_>>(),
    );
    assert!(
        matches!(
            &all[0].kind,
            LowerErrorKind::ConstDivByZero { in_const } if in_const == "BAD"
        ),
        "first error must be ConstDivByZero(BAD); got {:?}",
        all[0].kind,
    );
    assert!(
        matches!(
            &all[1].kind,
            LowerErrorKind::UnknownIdent(n) if n == "never_k"
        ),
        "second error must be UnknownIdent(never_k); got {:?}",
        all[1].kind,
    );
}
