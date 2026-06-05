//! Integration tests for the algorithm-sublanguage parser.
//!
//! Test strategy (hand-rolled assertions, no `insta`):
//! - For each existing example `prog.algo.nuc`, assert structural
//!   counts (consts, data, kernels, top-level statements). Snapshotting
//!   the full AST would be brittle as we evolve formatting / AST shape.
//! - Negative tests: hand-written invalid sources must return an `Err`
//!   with the expected `ParseErrorKind`.
//!
//! `05-stencil/prog.algo.nuc` historically used the legacy 2013-style
//! `where pure {{ ... }}` substitution syntax — TASK-0078 rewrote it
//! into the v2 form (signature-only kernels, bodies in `kernels.rs`).
//! It now parses; see `parses_example_05_stencil` below.
//!
//! Stable across edits: we assert *counts*, not exact AST trees. If
//! the example files grow or shrink, update the counts here.

use nucleus_compiler::algo::span::Spanned;
use nucleus_compiler::algo::{
    parse_algo, AlgoAst, BinOp, CmpOp, CombineOp, Expr, IndexedLValue, Item, KernelDecl, ParseError,
    ParseErrorKind, ParseErrors, Purity, ScalarType, Stmt,
};

/// The body of the first top-level `for` loop. Spans (TASK-0082) mean
/// items/statements are `Spanned<_>`; this projects through `.node` so
/// the structural assertions below stay readable. (`Spanned`'s
/// equality forwards to the node, so structural comparisons are
/// unaffected by source position — see `span.rs`.)
fn first_for_body(ast: &AlgoAst) -> &[Spanned<Stmt>] {
    ast.items
        .iter()
        .find_map(|i| match &i.node {
            Item::Stmt(s) => match &s.node {
                Stmt::For { body, .. } => Some(body.as_slice()),
                _ => None,
            },
            _ => None,
        })
        .expect("expected a for-loop at top level")
}

/// All kernel declarations, in source order.
fn kernels(ast: &AlgoAst) -> Vec<&KernelDecl> {
    ast.items
        .iter()
        .filter_map(|i| match &i.node {
            Item::Kernel(k) => Some(k),
            _ => None,
        })
        .collect()
}

/// Reads a source file at a workspace-relative path. Panics on IO
/// failure — these tests are environment-dependent by design, and
/// silent skips would hide regressions.
fn read_example(relpath: &str) -> String {
    // CARGO_MANIFEST_DIR is the path to `nucleus/nucleus-compiler/`.
    // Examples live at `<repo>/nuc-nucleus/examples/...`. Walk up two
    // levels.
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

#[test]
fn parses_example_01_elementwise_add() {
    // TASK-0013: the first concrete end-to-end example. Smallest
    // possible algorithm — one scalar kernel, two effectful I/O
    // kernels, one for-loop. If this fails the example file drifted
    // out of grammar; see TASK-0013 implementation notes for the
    // intended shape.
    let src = read_example("01-elementwise-add/prog.algo.nuc");
    let ast = parse_algo(&src).expect("01-elementwise-add must parse");

    // 1 const (N), 3 data (a, b, c), 4 kernels (add, load_input,
    // load_input_b, save_output).
    assert_eq!(ast.count_consts(), 1, "expected 1 const decl");
    assert_eq!(ast.count_data(), 3, "expected 3 data decls");
    assert_eq!(ast.count_kernels(), 4, "expected 4 kernel decls");

    // Top-level statements: `a <-- load_input();`,
    // `b <-- load_input_b();`, `for i : 0..N { ... }`,
    // `save_output(c);` -> 4.
    assert_eq!(ast.count_stmts(), 4, "expected 4 top-level statements");

    // The for-loop body holds a single dataflow statement.
    let for_body = first_for_body(&ast);
    assert_eq!(for_body.len(), 1, "for body should have 1 statement");

    // Spot-check purities and that `add` is the scalar kernel.
    let kernels = kernels(&ast);
    let by_name = |name: &str| {
        *kernels
            .iter()
            .find(|k| k.name.node == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    let add = by_name("add");
    assert_eq!(add.purity, Purity::Pure);
    assert_eq!(add.sig.params.len(), 2, "add takes two scalars");
    assert!(add.sig.ret.is_some(), "add returns a scalar");

    assert_eq!(by_name("load_input").purity, Purity::Effectful);
    assert_eq!(by_name("load_input_b").purity, Purity::Effectful);
    assert_eq!(by_name("save_output").purity, Purity::Effectful);
    assert!(
        by_name("save_output").sig.ret.is_none(),
        "save_output returns ()"
    );
}

#[test]
fn parses_example_02_split_add() {
    // TASK-0021: example 02 has the SAME algorithm shape as example
    // 01 (load_input + load_input_b + for-loop with one body stmt +
    // save_output). The point of the example is the multi-worker
    // *schedule*, not a new algorithm surface.
    //
    // If counts diverge from example 01, either (a) the algorithm
    // grew a feature it shouldn't have, or (b) example 01 changed
    // and this example needs to follow. Investigate before touching
    // the counts.
    let src = read_example("02-split-add/prog.algo.nuc");
    let ast = parse_algo(&src).expect("02-split-add must parse");

    // 1 const (N), 3 data (a, b, c), 4 kernels (add, load_input,
    // load_input_b, save_output).
    assert_eq!(ast.count_consts(), 1, "expected 1 const decl");
    assert_eq!(ast.count_data(), 3, "expected 3 data decls");
    assert_eq!(ast.count_kernels(), 4, "expected 4 kernel decls");

    // Top-level statements: `a <-- load_input();`,
    // `b <-- load_input_b();`, `for i : 0..N { ... }`,
    // `save_output(c);` -> 4.
    assert_eq!(ast.count_stmts(), 4, "expected 4 top-level statements");

    // The for-loop body holds a single dataflow statement.
    let for_body = first_for_body(&ast);
    assert_eq!(for_body.len(), 1, "for body should have 1 statement");

    // Spot-check purities.
    let kernels = kernels(&ast);
    let by_name = |name: &str| {
        *kernels
            .iter()
            .find(|k| k.name.node == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    let add = by_name("add");
    assert_eq!(add.purity, Purity::Pure);
    assert_eq!(add.sig.params.len(), 2, "add takes two scalars");
    assert!(add.sig.ret.is_some(), "add returns a scalar");

    assert_eq!(by_name("load_input").purity, Purity::Effectful);
    assert_eq!(by_name("load_input_b").purity, Purity::Effectful);
    assert_eq!(by_name("save_output").purity, Purity::Effectful);
}

#[test]
fn parses_example_03_reduction() {
    // TASK-0022: reduction example. The algorithm declares N,
    // NUM_WORKERS, PARTITION_SIZE (3 consts), 5 data (a, partials,
    // half1, half2, result), 4 kernels (load_input, save_output,
    // accumulate, combine).
    //
    // Top-level statements: `a <-- load_input();`, the phase-1
    // outer `for w` loop, then three phase-2 dataflow statements
    // (half1, half2, result), then `save_output(result);`. Total = 6.
    //
    // The phase-1 outer for-loop holds exactly one statement (the
    // inner `for i` loop). The inner for-loop holds exactly one
    // statement (the `partials[w] <-- accumulate(...)` dataflow).
    let src = read_example("03-reduction/prog.algo.nuc");
    let ast = parse_algo(&src).expect("03-reduction must parse");

    assert_eq!(ast.count_consts(), 3, "expected 3 const decls");
    assert_eq!(ast.count_data(), 5, "expected 5 data decls");
    assert_eq!(ast.count_kernels(), 4, "expected 4 kernel decls");
    assert_eq!(ast.count_stmts(), 6, "expected 6 top-level statements");

    // Spot-check kernel purities.
    let kernels = kernels(&ast);
    let by_name = |name: &str| {
        *kernels
            .iter()
            .find(|k| k.name.node == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    assert_eq!(by_name("accumulate").purity, Purity::Pure);
    assert_eq!(by_name("combine").purity, Purity::Pure);
    assert_eq!(by_name("load_input").purity, Purity::Effectful);
    assert_eq!(by_name("save_output").purity, Purity::Effectful);

    // TASK-0343.01.01 GRAMMAR GOTCHA: `combine` is a legal kernel NAME
    // here, NOT the (positional, after-purity) combine attribute. No
    // kernel in 03-reduction declares a `combine = <op>` attribute, so
    // every kernel's `combine` field must be `None` — including the one
    // literally named `combine`. If the contextual keyword had been
    // promoted to a reserved word, `kernel combine : ...` would have
    // failed to parse (the `.expect("03-reduction must parse")` above
    // would have fired). This asserts the attribute is also not
    // mis-attached.
    assert_eq!(
        by_name("combine").combine,
        None,
        "the kernel literally named `combine` must carry NO combine \
         attribute — `combine` is a contextual keyword, not reserved"
    );
    assert!(
        kernels.iter().all(|k| k.combine.is_none()),
        "no 03-reduction kernel declares a combine attribute"
    );

    // Both step kernels are binary scalar.
    assert_eq!(by_name("accumulate").sig.params.len(), 2);
    assert_eq!(by_name("combine").sig.params.len(), 2);
    assert!(by_name("save_output").sig.ret.is_none());

    // The outer for-loop body has exactly one nested for-loop.
    let outer_body = first_for_body(&ast);
    assert_eq!(outer_body.len(), 1, "outer-for body should have 1 stmt");
    let inner_body = match &outer_body[0].node {
        Stmt::For { body, .. } => body,
        other => panic!("expected inner for-loop; got {:?}", other),
    };
    assert_eq!(inner_body.len(), 1, "inner-for body should have 1 stmt");
}

#[test]
fn parses_example_13_cnn_inference() {
    let src = read_example("13-cnn-inference/prog.algo.nuc");
    let ast = parse_algo(&src).expect("13-cnn-inference must parse");

    // Counts from a hand-walk of the file. If the example changes,
    // update here.
    assert_eq!(ast.count_consts(), 7, "expected 7 const decls");
    assert_eq!(ast.count_data(), 4, "expected 4 data decls");
    assert_eq!(ast.count_kernels(), 5, "expected 5 kernel decls");

    // Top-level statements: `input <-- load_input();`,
    // `for n : 0 .. B { ... }`, `save_output(output);` -> 3.
    assert_eq!(ast.count_stmts(), 3, "expected 3 top-level statements");

    // Spot-check kernel purities.
    let kernels = kernels(&ast);
    let by_name = |name: &str| {
        *kernels
            .iter()
            .find(|k| k.name.node == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    assert_eq!(by_name("load_input").purity, Purity::Effectful);
    assert_eq!(by_name("save_output").purity, Purity::Effectful);
    assert_eq!(by_name("conv_block_1").purity, Purity::Pure);
    assert_eq!(by_name("classifier").purity, Purity::Pure);

    // The for-loop body should contain 3 dataflow statements.
    let for_body = first_for_body(&ast);
    assert_eq!(for_body.len(), 3, "for body should have 3 statements");
}

#[test]
fn parses_example_14_hearing_aid() {
    // Cycle 201 (TASK-0054 reopen): example 14 rewritten from f32 +
    // per-frame stateful peripheral kernels (fe_capture/rf_receive/
    // fe_emit/rf_transmit) to i32 + bulk IO (load_mic/load_bt/
    // save_spk/save_bt_out) + explicit `mixed` intermediate symbol
    // (v2 codegen rejects nested kernel calls inside argument
    // expressions). This test was rewritten to pin the new structure.
    let src = read_example("14-hearing-aid/prog.algo.nuc");
    let ast = parse_algo(&src).expect("14-hearing-aid must parse");

    assert_eq!(ast.count_consts(), 2, "expected 2 const decls");
    // 5 data symbols: mic_in, bt_in, spk_out, bt_out, mixed.
    assert_eq!(ast.count_data(), 5, "expected 5 data decls");
    // 6 kernels: load_mic, load_bt, save_spk, save_bt_out, mix2, denoise.
    assert_eq!(ast.count_kernels(), 6, "expected 6 kernel decls");
    // 5 top-level statements: mic_in <-- load_mic, bt_in <-- load_bt,
    // for frame { ... }, save_spk(spk_out), save_bt_out(bt_out).
    assert_eq!(ast.count_stmts(), 5, "expected 5 top-level statements");

    // Inside the for-loop body, 3 dataflow statements (bt_out <--
    // denoise, mixed <-- mix2, spk_out <-- denoise).
    let for_body = first_for_body(&ast);
    assert_eq!(for_body.len(), 3, "for body should have 3 statements");

    // mix2 has 2 params -> assert structurally.
    let mix2 = ast
        .items
        .iter()
        .find_map(|i| match &i.node {
            Item::Kernel(k) if k.name.node == "mix2" => Some(k),
            _ => None,
        })
        .expect("missing mix2 kernel");
    assert_eq!(mix2.sig.params.len(), 2);
    assert!(mix2.sig.ret.is_some(), "mix2 returns a typed value");
    assert_eq!(mix2.purity, Purity::Pure);

    // save_bt_out returns unit (bulk-IO sink kernel, replaces the
    // old per-frame rf_transmit).
    let save_bt_out = ast
        .items
        .iter()
        .find_map(|i| match &i.node {
            Item::Kernel(k) if k.name.node == "save_bt_out" => Some(k),
            _ => None,
        })
        .expect("missing save_bt_out kernel");
    assert!(save_bt_out.sig.ret.is_none(), "save_bt_out returns ()");
    assert_eq!(save_bt_out.purity, Purity::Effectful);
}

/// TASK-0054.01 (M11 entry): example 14 grew a SECOND algorithm shape,
/// `prog.embedded.algo.nuc`, for the M11 multi-MCU `embedded_multimcu`
/// schedule. It declares the per-frame peripheral kernels
/// (fe_capture / rf_receive / fe_emit / rf_transmit) that the tier-1
/// bulk-IO `prog.algo.nuc` does NOT, while reusing mix2/denoise. This
/// test is the AC#4 "parses in isolation" evidence; the tier-1 file's
/// own test (`parses_example_14_hearing_aid`) stays untouched.
#[test]
fn parses_example_14_hearing_aid_embedded() {
    let src = read_example("14-hearing-aid/prog.embedded.algo.nuc");
    let ast = parse_algo(&src).expect("14-hearing-aid/embedded must parse");

    // Same 2 consts (N_FRAMES, SAMPLES_PER_FRAME) and 5 data symbols
    // (mic_in, bt_in, spk_out, bt_out, mixed) as the tier-1 file.
    assert_eq!(ast.count_consts(), 2, "expected 2 const decls");
    assert_eq!(ast.count_data(), 5, "expected 5 data decls");

    // 6 kernels, but a DIFFERENT set from tier-1: the 4 per-frame
    // peripherals (fe_capture/rf_receive/fe_emit/rf_transmit) replace
    // the 4 bulk-IO kernels (load_mic/load_bt/save_spk/save_bt_out);
    // mix2/denoise are shared.
    assert_eq!(ast.count_kernels(), 6, "expected 6 kernel decls");

    // Top level is a single `for frame` loop (no bulk load/save
    // bookends, unlike tier-1 which has 5 top-level statements).
    assert_eq!(ast.count_stmts(), 1, "expected 1 top-level statement");

    // The 4 per-frame peripheral kernels must be present with the
    // signatures the AC pins:
    //   fe_capture/rf_receive : ()                       -> i32[SPF] effectful
    //   fe_emit/rf_transmit   : (i32[SPF])               -> ()       effectful
    let ks = kernels(&ast);
    let by_name = |n: &str| {
        ks.iter()
            .find(|k| k.name.node == n)
            .unwrap_or_else(|| panic!("missing kernel {n}"))
    };
    for n in ["fe_capture", "rf_receive"] {
        let k = by_name(n);
        assert_eq!(k.sig.params.len(), 0, "{n} takes no args");
        assert!(k.sig.ret.is_some(), "{n} returns a frame buffer");
        assert_eq!(k.purity, Purity::Effectful, "{n} is effectful");
    }
    for n in ["fe_emit", "rf_transmit"] {
        let k = by_name(n);
        assert_eq!(k.sig.params.len(), 1, "{n} takes one frame buffer");
        assert!(k.sig.ret.is_none(), "{n} returns ()");
        assert_eq!(k.purity, Purity::Effectful, "{n} is effectful");
    }

    // The for-body has 7 statements (2 captures + outbound denoise +
    // rf_transmit + mix2 + inbound denoise + fe_emit). Critically the
    // inbound path uses the explicit `mixed` intermediate (AC#2: NO
    // nested kernel call inside an argument expression).
    let for_body = first_for_body(&ast);
    assert_eq!(for_body.len(), 7, "for body should have 7 statements");
}

/// `05-stencil/prog.algo.nuc` was historically the legacy 2013-style
/// `kernel foo(a,b) -> out where pure {{ ... }};` syntax. TASK-0078 /
/// TASK-0031 rewrote it into the v2 form (signature-only kernels,
/// bodies in adjacent `kernels.rs`). This test pins that the v2
/// surface now parses cleanly. The "legacy syntax rejected" guarantee
/// stays alive via the `negative_legacy_inline_kernel_body` test
/// below, which uses a distilled fragment so the rejection invariant
/// no longer depends on this example file's contents.
#[test]
fn parses_example_05_stencil() {
    let src = read_example("05-stencil/prog.algo.nuc");
    let ast = parse_algo(&src).expect("05-stencil must parse");

    // 2 consts (H, W), 2 data (img_in, img_out), 3 kernels
    // (blur3, load_image, save_image).
    assert_eq!(ast.count_consts(), 2, "expected 2 const decls");
    assert_eq!(ast.count_data(), 2, "expected 2 data decls");
    assert_eq!(ast.count_kernels(), 3, "expected 3 kernel decls");

    // Top-level statements: `img_in <-- load_image();`, the outer
    // `for y` loop, `save_image(img_out);` -> 3.
    assert_eq!(ast.count_stmts(), 3, "expected 3 top-level statements");

    // Spot-check the kernel purities.
    let kernels = kernels(&ast);
    let by_name = |name: &str| {
        *kernels
            .iter()
            .find(|k| k.name.node == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    let blur3 = by_name("blur3");
    assert_eq!(blur3.purity, Purity::Pure);
    assert_eq!(blur3.sig.params.len(), 9, "blur3 takes nine scalars");
    assert!(blur3.sig.ret.is_some(), "blur3 returns a scalar");

    assert_eq!(by_name("load_image").purity, Purity::Effectful);
    assert_eq!(by_name("save_image").purity, Purity::Effectful);
    assert!(
        by_name("save_image").sig.ret.is_none(),
        "save_image returns ()"
    );

    // The outer for-loop body holds exactly one statement (the inner
    // `for x` loop).
    let outer_body = first_for_body(&ast);
    assert_eq!(outer_body.len(), 1, "outer-for body should have 1 stmt");
    let inner_body = match &outer_body[0].node {
        Stmt::For { body, .. } => body,
        other => panic!("expected inner for-loop; got {:?}", other),
    };
    assert_eq!(inner_body.len(), 1, "inner-for body should have 1 stmt");
}

/// `07-matmul/prog.algo.nuc` (TASK-0032). Triple-nested loop with a
/// reduction on the innermost axis; the LHS `c[i][j]` appears on the
/// RHS of the same dataflow statement. Counts: 1 const (N), 3 data
/// (a, b, c), 4 kernels (madd, load_a, load_b, save_c), 4 top-level
/// statements (two load dataflows, the outer for-i, the save_c
/// effect).
#[test]
fn parses_example_07_matmul() {
    let src = read_example("07-matmul/prog.algo.nuc");
    let ast = parse_algo(&src).expect("07-matmul must parse");

    assert_eq!(ast.count_consts(), 1, "expected 1 const decl (N)");
    assert_eq!(ast.count_data(), 3, "expected 3 data decls (a, b, c)");
    assert_eq!(ast.count_kernels(), 4, "expected 4 kernel decls");
    assert_eq!(ast.count_stmts(), 4, "expected 4 top-level statements");

    let kernels = kernels(&ast);
    let by_name = |name: &str| {
        *kernels
            .iter()
            .find(|k| k.name.node == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    let madd = by_name("madd");
    assert_eq!(madd.purity, Purity::Pure);
    assert_eq!(madd.sig.params.len(), 3, "madd takes three scalars");
    assert!(madd.sig.ret.is_some(), "madd returns a scalar");

    assert_eq!(by_name("load_a").purity, Purity::Effectful);
    assert_eq!(by_name("load_b").purity, Purity::Effectful);
    assert_eq!(by_name("save_c").purity, Purity::Effectful);
    assert!(by_name("save_c").sig.ret.is_none(), "save_c returns ()");

    // The outer for-i body holds exactly one statement (the inner
    // for-j loop), whose body holds one statement (the for-k loop),
    // whose body holds one statement (the madd dataflow).
    let outer_body = first_for_body(&ast);
    assert_eq!(outer_body.len(), 1, "outer-for body should have 1 stmt");
    let middle_body = match &outer_body[0].node {
        Stmt::For { body, .. } => body,
        other => panic!("expected middle for-loop; got {:?}", other),
    };
    assert_eq!(middle_body.len(), 1, "middle-for body should have 1 stmt");
    let inner_body = match &middle_body[0].node {
        Stmt::For { body, .. } => body,
        other => panic!("expected inner for-loop; got {:?}", other),
    };
    assert_eq!(inner_body.len(), 1, "inner-for body should have 1 stmt");
}

// --------------------------------------------------------------------
// Negative tests
// --------------------------------------------------------------------

#[test]
fn negative_missing_semicolon() {
    // No `;` after the const declaration.
    let src = "const N : usize = 16\ndata x : f32[N];\n";
    let err = expect_err(src);
    // The missing `;` is at end of line 1; the parser reports the
    // failure at the first unexpected token, which is `d` of `data`
    // on line 2 (whitespace and the newline are consumed by `pad`).
    assert_eq!(err.line, 2, "{:?}", err);
    assert_eq!(err.column, 1, "{:?}", err);
}

#[test]
fn negative_unknown_keyword_in_algorithm() {
    // `block = 64;` is a schedule directive, not algorithm syntax.
    // TASK-0083: the parser now detects the `<sched_kw> =` shape and
    // emits a tailored hint message pointing at the offending
    // keyword instead of the generic "unexpected `=`".
    let src = "\
const H : usize = 600;
for y : 1 .. H {
    block = 64;
}
";
    let err = expect_err(src);
    // Hint underlines `block` (line 3, col 5), NOT `=`.
    assert_eq!(err.line, 3, "{:?}", err);
    assert_eq!(err.column, 5, "{:?}", err);
    assert!(
        err.message.contains("`block` is a schedule directive"),
        "expected schedule-directive hint, got: {}",
        err.message
    );
    assert!(
        err.message.contains("*.sched.nuc"),
        "expected `*.sched.nuc` reference in hint, got: {}",
        err.message
    );
}

// --------------------------------------------------------------------
// TASK-0343.01.01: contextual `combine = <op>` kernel attribute
// --------------------------------------------------------------------

/// Parse a one-kernel program and return the kernel's `combine` field.
fn parse_one_kernel_combine(decl: &str) -> Option<CombineOp> {
    let src = format!("{decl}\n");
    let ast = parse_algo(&src).expect("kernel decl must parse");
    let ks = kernels(&ast);
    assert_eq!(ks.len(), 1, "expected exactly one kernel decl");
    ks[0].combine
}

#[test]
fn combine_attribute_sum_or_xor_parse() {
    assert_eq!(
        parse_one_kernel_combine("kernel k : (i32, i32) -> i32 pure combine=sum;"),
        Some(CombineOp::Sum)
    );
    assert_eq!(
        parse_one_kernel_combine("kernel k : (i32, i32) -> i32 pure combine = or ;"),
        Some(CombineOp::Or),
        "whitespace around `=` and the op must be tolerated"
    );
    assert_eq!(
        parse_one_kernel_combine("kernel k : (i32, i32) -> i32 pure combine=xor;"),
        Some(CombineOp::Xor)
    );
    // No attribute => None.
    assert_eq!(
        parse_one_kernel_combine("kernel k : (i32, i32) -> i32 pure;"),
        None
    );
}

#[test]
fn combine_named_kernel_still_legal_without_attribute() {
    // The grammar gotcha: `combine` as a kernel NAME (right after
    // `kernel`) must stay legal even though it is also the attribute
    // keyword (after purity). 03-reduction / 23-dot-product rely on this.
    assert_eq!(
        parse_one_kernel_combine("kernel combine : (i32, i32) -> i32 pure;"),
        None,
        "kernel named `combine` with no attribute => None combine field"
    );
    // And a kernel named `combine` that ALSO declares a combine attribute
    // parses (name position vs attribute position never overlap).
    assert_eq!(
        parse_one_kernel_combine("kernel combine : (i32, i32) -> i32 pure combine=xor;"),
        Some(CombineOp::Xor),
        "kernel named `combine` may also carry a `combine=<op>` attribute"
    );
}

#[test]
fn combine_attribute_min_max_and_rejected_pointing_at_followup() {
    // min/max/and (non-zero identity, identity-aware init) are OUT OF
    // SCOPE for TASK-0343.01.01 — rejected with a typed error pointing
    // at the deferral task TASK-0343.01.02. No silent fallthrough.
    for op in ["min", "max", "and"] {
        let src = format!("kernel k : (i32, i32) -> i32 pure combine={op};\n");
        let err = expect_err(&src);
        assert!(
            err.message.contains("TASK-0343.01.02"),
            "`combine={op}` reject must point at the deferral task \
             TASK-0343.01.02; got: {}",
            err.message
        );
    }
}

#[test]
fn combine_attribute_unknown_op_rejected() {
    let err = expect_err("kernel k : (i32, i32) -> i32 pure combine=product;\n");
    assert!(
        err.message.contains("combine") && err.message.contains("sum"),
        "unknown combine op must name the expected set (sum/or/xor); got: {}",
        err.message
    );
}

/// TASK-0433: a DSL identifier that collides with a Rust keyword is
/// rejected at the `.nuc` source site, NOT silently admitted to be
/// emitted as un-compilable generated Rust (`let mut in = …`). The
/// diagnostic must underline the identifier and name the codegen
/// reason. Covers the concrete TASK-0431 trigger (`data in`), a
/// kernel-name collision (`match`), and the raw-identifier-
/// INCOMPATIBLE keywords (`crate`, `self`) that even an `r#`-escape
/// strategy could not have rescued (AC#1, AC#2, AC#3).
#[test]
fn rust_keyword_identifier_rejected_at_source_site() {
    // (src, offending-keyword, (expected line, expected col)).
    let cases: &[(&str, &str, (usize, usize))] = &[
        // The original TASK-0431 trigger.
        ("data in : i32[4];\n", "in", (1, 6)),
        // A kernel name colliding with a Rust keyword.
        (
            "data x : i32[4];\nkernel match : (i32) -> i32 pure;\n",
            "match",
            (2, 8),
        ),
        // Raw-identifier-incompatible: `r#crate` / `r#self` are
        // rejected by rustc, so these MUST be source-site rejects.
        ("data crate : i32[4];\n", "crate", (1, 6)),
        ("data self : i32[4];\n", "self", (1, 6)),
    ];
    for (src, kw, (line, col)) in cases {
        let err = expect_err(src);
        assert_eq!(err.line, *line, "kw `{kw}`: wrong line in {err:?}");
        assert_eq!(err.column, *col, "kw `{kw}`: wrong col in {err:?}");
        assert!(
            err.message.contains(&format!("`{kw}`")),
            "kw `{kw}`: diagnostic must quote the identifier, got: {}",
            err.message
        );
        assert!(
            err.message.contains("Rust reserved word"),
            "kw `{kw}`: diagnostic must name the codegen-collision \
             reason (not the grammar-keyword message), got: {}",
            err.message
        );
        // It must be a source-site PARSE error, never a generic
        // grammar-keyword message that conflates the two concepts.
        assert!(
            !err.message.contains("expected identifier, found keyword"),
            "kw `{kw}`: Rust-reserved collision must NOT reuse the \
             grammar-keyword message, got: {}",
            err.message
        );
    }
}

/// TASK-0434: the algo for-loop VARIABLE position now anchors its
/// keyword-collision diagnostic AT the offending `VAR`, not at the
/// downstream `{`.
///
/// Background: TASK-0433 guaranteed correctness (a reserved `VAR` is
/// rejected and never reaches codegen) but the diagnostic pointed at
/// the trailing `{` — chumsky 0.9 merges simultaneous alternative
/// errors by furthest input position, and the `for_stmt` branch dying
/// at `VAR` let the more-consuming `{`-mismatch win. TASK-0434's
/// `for_loop_var()` parser consumes through the `{` on a collision so
/// its error out-reaches the brace, while pinning the *display span*
/// at `VAR`.
///
/// We assert the IMPROVED anchoring for BOTH reserved classes:
/// - a Rust-reserved `VAR` (`loop`) reports the codegen-collision
///   message (`Rust reserved word`) at `loop`;
/// - a grammar-keyword `VAR` (`const`) reports the grammar message
///   (`found keyword`) at `const`;
/// - both anchor at the VAR token's (line, col) — line 2, col 5 (the
///   char right after `for `), NOT the `{` on the same line.
///
/// The messages are the SAME ones the data/kernel/worker positions
/// emit for the same word (shared `ident_collision_message`), so the
/// for-var diagnostic stays consistent with the rest of the front-end.
#[test]
fn for_loop_var_keyword_collision_is_anchored_at_the_variable_token() {
    // (src, offending-VAR, expected (line, col), message-substring).
    let cases: &[(&str, &str, (usize, usize), &str)] = &[
        (
            "const N : usize = 4;\nfor loop : 0 .. N {\n}\n",
            "loop",
            (2, 5),
            "Rust reserved word",
        ),
        (
            "const N : usize = 4;\nfor const : 0 .. N {\n}\n",
            "const",
            (2, 5),
            "expected identifier, found keyword",
        ),
    ];
    for (src, var, (line, col), msg_substr) in cases {
        // Correctness (TASK-0433 invariant): still rejected, so the
        // reserved word never reaches the AST / codegen.
        assert!(
            parse_algo(src).is_err(),
            "a reserved for-loop variable `{var}` must be rejected"
        );
        let err = expect_err(src);
        // Anchored AT the VAR token, NOT the trailing `{`.
        assert_eq!(
            err.line, *line,
            "for-var `{var}`: diagnostic must anchor at the VAR line, got {err:?}"
        );
        assert_eq!(
            err.column, *col,
            "for-var `{var}`: diagnostic must anchor at the VAR column \
             (right after `for `), not the trailing `{{`, got {err:?}"
        );
        // The diagnostic must quote the offending word and carry the
        // class-appropriate (shared) message.
        assert!(
            err.message.contains(&format!("`{var}`")),
            "for-var `{var}`: diagnostic must quote the identifier, got: {}",
            err.message
        );
        assert!(
            err.message.contains(msg_substr),
            "for-var `{var}`: diagnostic must contain {msg_substr:?}, got: {}",
            err.message
        );
        // It must NOT be the old weak downstream `{`-mismatch message.
        assert!(
            !err.message.contains("found \"{\""),
            "for-var `{var}`: must no longer surface the downstream \
             `{{`-mismatch, got: {}",
            err.message
        );
    }
}

/// TASK-0434.01: the VAR-anchored keyword-collision diagnostic must
/// also fire on TRUNCATED input with no opening brace (`for VAR : lo ..
/// hi` at EOF). The `for_loop_var` collision path `take_until`s the
/// block `{` to out-reach the chumsky furthest-position merge; before
/// TASK-0434.01 the bare `just('{')` terminator ran to EOF and FAILED
/// on brace-less input, so the user got a generic "found end of input"
/// instead of the VAR-anchored message. The `end()` terminator
/// alternative fixes it. Correctness was never affected (the reserved
/// VAR is rejected either way); this pins the diagnostic QUALITY.
#[test]
fn for_loop_var_keyword_collision_anchored_on_truncated_braceless_input() {
    // No `{ … }` body, EOF right after the range. (src, VAR, (line,col),
    // message-substring) — mirrors the braced sibling test's shape.
    let cases: &[(&str, &str, (usize, usize), &str)] = &[
        (
            "const N : usize = 4;\nfor loop : 0 .. N",
            "loop",
            (2, 5),
            "Rust reserved word",
        ),
        (
            "const N : usize = 4;\nfor const : 0 .. N",
            "const",
            (2, 5),
            "expected identifier, found keyword",
        ),
    ];
    for (src, var, (line, col), msg_substr) in cases {
        assert!(
            parse_algo(src).is_err(),
            "a reserved for-loop variable `{var}` must be rejected (truncated input)"
        );
        let err = expect_err(src);
        assert_eq!(
            err.line, *line,
            "for-var `{var}` (truncated): diagnostic must anchor at the VAR line, got {err:?}"
        );
        assert_eq!(
            err.column, *col,
            "for-var `{var}` (truncated): diagnostic must anchor at the VAR column, \
             not end-of-input, got {err:?}"
        );
        assert!(
            err.message.contains(&format!("`{var}`")),
            "for-var `{var}` (truncated): diagnostic must quote the identifier, got: {}",
            err.message
        );
        assert!(
            err.message.contains(msg_substr),
            "for-var `{var}` (truncated): diagnostic must contain {msg_substr:?}, got: {}",
            err.message
        );
        // Must NOT degrade to the generic EOF message the bare-`{`
        // terminator produced pre-TASK-0434.01.
        assert!(
            !err.message.contains("end of input"),
            "for-var `{var}` (truncated): must not surface the generic end-of-input \
             message, got: {}",
            err.message
        );
    }
}

/// TASK-0433 (positive / over-fire guard): an identifier that merely
/// CONTAINS a Rust keyword as a prefix/substring is still accepted.
/// The reject must bite on exact equality only, mirroring the grammar
/// `KEYWORDS` prefix guard (`in_` is not `in`).
#[test]
fn rust_keyword_prefix_identifier_still_accepted() {
    let src = "\
const N : usize = 4;
data in_ : i32[N];
data match_thing : i32[N];
data crater : i32[N];
for loops : 0 .. N {
    in_[loops] <-- inc(loops);
}
";
    let ast = parse_algo(src).expect("near-miss identifiers must parse");
    assert_eq!(ast.count_data(), 3, "in_/match_thing/crater are valid data");
}

/// TASK-0083: schedule directives in algorithm files get a tailored
/// hint, not a generic "unexpected `=`" / "unexpected ident" error.
///
/// Covers every keyword shape promised by `docs/grammar-algo.md` §3:
/// the `<kw> =` family, plus `place IDENT`, `place_data IDENT`,
/// `check loop`. Each fixture is a tiny `for { … }` body containing
/// exactly the offending directive, so the assertion is on the
/// `(keyword, hint-message)` pair alone.
#[test]
fn sched_directive_hint_fires_for_each_keyword() {
    let cases: &[(&str, &str)] = &[
        ("block = 64;", "block"),
        ("buffer = 16;", "buffer"),
        ("notify = once;", "notify"),
        ("partition = 4;", "partition"),
        ("pipeline = 2;", "pipeline"),
        ("transfer = sync;", "transfer"),
        ("unroll = 8;", "unroll"),
        // `vectorize = 4;` removed 2026-05-25 (TASK-0292) — vectorize
        // is no longer a schedule directive; the algo parser no
        // longer emits a hint for it.
        ("place k on host;", "place"),
        ("place_data x to mem;", "place_data"),
        ("check loop y : latency_max = 10;", "check"),
    ];
    for (directive, kw) in cases {
        let src = format!("for y : 1 .. 10 {{\n    {directive}\n}}\n");
        let err = expect_err(&src);
        assert!(
            err.message
                .contains(&format!("`{kw}` is a schedule directive")),
            "directive `{directive}`: expected hint mentioning `{kw}`, got: {}",
            err.message
        );
        assert!(
            err.message.contains("*.sched.nuc"),
            "directive `{directive}`: expected `*.sched.nuc` in hint, got: {}",
            err.message
        );
        // Hint underlines the keyword itself (col 5 inside the
        // 4-space-indented body), not the trailing `=`/IDENT.
        assert_eq!(err.line, 2, "directive `{directive}`: line {:?}", err);
        assert_eq!(err.column, 5, "directive `{directive}`: col {:?}", err);
    }
}

/// TASK-0083 disambiguation: a schedule-directive keyword used as a
/// PLAIN identifier in a valid algorithm statement (e.g. as an LValue
/// in dataflow or a callee name) must NOT fire the hint. These
/// keywords are NOT in the algorithm `KEYWORDS` set; they only fire
/// the hint in the unambiguous `<kw> =` / `place IDENT` / etc shapes.
#[test]
fn sched_directive_hint_does_not_break_keyword_as_plain_ident() {
    // `block` used as a data name + dataflow LValue.
    let src = "\
data block : f32[16];
data src : f32[16];
block <-- src;
";
    parse_algo(src).expect("plain `block` ident must still parse");
}

#[test]
fn negative_dangling_kernel_arg() {
    // Trailing comma is allowed (allow_trailing); a *dangling* arg
    // here means an unmatched `(` with no `)`.
    let src = "\
kernel k : (f32[16], -> () pure;
";
    let err = expect_err(src);
    assert_eq!(err.line, 1, "{:?}", err);
}

#[test]
fn negative_legacy_inline_kernel_body() {
    // Distilled from 05-stencil: legacy syntax must be rejected.
    let src = "kernel blur3(a, b) -> out where pure {{ ${out} = ${a}; }};\n";
    let err = expect_err(src);
    assert_eq!(err.line, 1, "{:?}", err);
}

#[test]
fn parses_minimal_program() {
    // Tiny smoke test: a single dataflow statement.
    let src = "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel id : (f32[N]) -> f32[N] pure;
y <-- id(x);
";
    let ast = parse_algo(src).expect("must parse");
    assert_eq!(ast.count_consts(), 1);
    assert_eq!(ast.count_data(), 2);
    assert_eq!(ast.count_kernels(), 1);
    assert_eq!(ast.count_stmts(), 1);
}

#[test]
fn parses_const_expr_in_shape() {
    // f32[H/2] requires `ConstExpr` inside `DimList`.
    let src = "\
const H : usize = 8;
data x : f32[H/2];
";
    let ast = parse_algo(src).expect("must parse");
    let d = match &ast.items[1].node {
        Item::Data(d) => d,
        _ => panic!("expected DataDecl"),
    };
    assert_eq!(d.ty.scalar, ScalarType::F32);
    assert_eq!(d.ty.dims.len(), 1);
}

#[test]
fn parser_error_carries_line_and_column() {
    // Force a parse error at a known location and check coordinates.
    // The `?` is illegal at the very start.
    let src = "?";
    let err = expect_err(src);
    assert_eq!(err.line, 1);
    assert_eq!(err.column, 1);
}

// --------------------------------------------------------------------
// Multi-error reporting + recovery (TASK-0080 / TASK-0081)
// --------------------------------------------------------------------

/// Helper: all errors, in deterministic positional order.
fn expect_errs(src: &str) -> ParseErrors {
    parse_algo(src).expect_err("expected parse error(s)")
}

/// TASK-0080 AC#1 / TASK-0081 AC#1: a single source with TWO
/// independent syntax errors in DIFFERENT items must surface BOTH in
/// one pass, each with its own correct 1-based `(line, column)`. The
/// first item's `?` is illegal; the parser recovers at the next `;`
/// and reports the second item's bad token too.
#[test]
fn multi_error_two_independent_errors_both_reported() {
    // Line 1: a valid const so recovery has a clean prefix.
    // Line 2: `data x : f32[?];` — `?` is not a valid dim expression.
    // Line 3: a valid data decl.
    // Line 4: `kernel k : (f32[4]) => () pure;` — `=>` is not `->`.
    let src = "\
const N : usize = 4;
data x : f32[?];
data y : f32[N];
kernel k : (f32[4]) => () pure;
";
    let errs = expect_errs(src);
    assert!(
        errs.errors().len() >= 2,
        "expected >=2 distinct errors, got {:?}",
        errs.errors()
    );

    // Errors are positional: earliest source offset first. The first
    // must point at the `?` on line 2; a later one at the `=>` region
    // on line 4. We validate the (line, column) against the source by
    // reconstructing the offending character.
    let first = &errs.errors()[0];
    assert_eq!(first.line, 2, "first error on line 2: {first:?}");
    // Column of `?` in `data x : f32[?];` — `data x : f32[` is 13
    // chars, so `?` is column 14.
    assert_eq!(first.column, 14, "first error at the `?`: {first:?}");

    // Some later error must be located on line 4 (the `=>`). We do
    // NOT pin its exact column (chumsky's reported offset for an
    // operator-shape mismatch is an implementation detail); pinning
    // the line proves recovery resumed past line 2 and kept going —
    // that is the locatedness-per-error AC.
    assert!(
        errs.errors().iter().any(|e| e.line == 4),
        "a later error must be reported on line 4 (recovery resumed): {:?}",
        errs.errors()
    );
}

/// TASK-0081 AC#1: error mid-program; a later, fully valid item is
/// still parsed/reported (recovery resumed) — and the run is
/// deterministic across repeated parses (AC#2).
#[test]
fn recovery_resumes_and_is_deterministic() {
    let src = "\
data a : f32[%];
const M : usize = 8;
kernel k : (f32[8]) -> f32[8] pure;
";
    let e1 = expect_errs(src);
    let e2 = expect_errs(src);
    // Same source -> identical error set AND order (reproducibility
    // gate; no HashMap/HashSet in the error path).
    assert_eq!(
        e1, e2,
        "parse errors must be a deterministic function of the source"
    );
    // The bad `%` is on line 1; recovery skips to the `;` and the
    // following valid items do not add errors.
    assert_eq!(e1.errors()[0].line, 1, "{:?}", e1.errors());
}

/// TASK-0080 AC#2 (no loosened assertion) / over-aggressive-recovery
/// guard: a source with EXACTLY ONE syntax error must report EXACTLY
/// ONE error — recovery must not manufacture a spurious cascade. (The
/// `expect_err` helper enforces this for every single-error negative
/// test too; this is the explicit, named pin.)
#[test]
fn single_error_input_yields_exactly_one_error_no_cascade() {
    let src = "\
const N : usize = 4;
data x : f32[N] @;
data y : f32[N];
kernel k : (f32[N]) -> f32[N] pure;
y <-- k(x);
";
    let errs = expect_errs(src);
    assert_eq!(
        errs.errors().len(),
        1,
        "exactly one error expected; recovery must not cascade: {:?}",
        errs.errors()
    );
    assert_eq!(errs.errors()[0].kind, ParseErrorKind::Unexpected);
}

/// TASK-0406: bite test for the `UnexpectedEof` arm of
/// [`ParseErrorKind`]. The classifier (`error.rs`, the
/// `SimpleReason::Unexpected if err.found().is_none() => UnexpectedEof`
/// site) maps an EOF-while-more-was-expected failure to `UnexpectedEof`;
/// every OTHER `.kind` assertion in this suite pins `Unexpected`, so
/// this was the lone production-reachable `ParseErrorKind` variant with
/// no bite test (cycle-236 TASK-0406 audit; the architect refuted the
/// "single error type / per-variant N/A" premise). A construct
/// truncated exactly at EOF — `const N : usize =`, where the value
/// expression is required but the input ends — drives the parser to
/// fail with `found() == None`.
#[test]
fn negative_unexpected_eof_kind_on_truncated_input() {
    let err = expect_err("const N : usize =");
    assert_eq!(
        err.kind,
        ParseErrorKind::UnexpectedEof,
        "a construct truncated at EOF must classify as UnexpectedEof, \
         not Unexpected: {err:?}"
    );
}

/// TASK-0081 AC#2: recovery is BOUNDED. A pathological, deeply
/// malformed input must TERMINATE (no infinite skip-then-retry) and
/// yield a finite, deterministic error set whose size grows at most
/// *linearly* with the input — not hang, not blow up super-linearly.
/// The test completing at all is the termination evidence; the
/// linear-ceiling assertion is the no-unbounded-cascade evidence.
#[test]
fn pathological_input_terminates_bounded_and_deterministic() {
    // A wall of illegal characters with scattered `;` sync points and
    // no valid item anywhere. Without bounded recovery this is the
    // infinite-retry / cascade-spam footgun.
    let line = "@@@ ;; ??? ;; %%% ;; &&& ;; ^^^ ;; ### ;; !!! ;;\n";
    let src = line.repeat(8);
    let r1 = expect_errs(&src);
    let r2 = expect_errs(&src);
    assert_eq!(r1, r2, "pathological input must parse deterministically");
    // Finite and at most linear in the input length: each recovery
    // step consumes ≥1 char, so the error count cannot exceed the
    // character count. We assert a strict *linear* ceiling (one error
    // per input char is the theoretical max; real output is well
    // under). A super-linear / unbounded cascade — the footgun this
    // test guards — would blow past this. The exact count (113 today)
    // is an implementation detail; the load-bearing invariant is
    // "finite, deterministic, ≤ O(n)".
    assert!(!r1.errors().is_empty(), "must report errors");
    assert!(
        r1.errors().len() <= src.len(),
        "error set must be at most linear in input (≤ {} chars), got {}",
        src.len(),
        r1.errors().len()
    );
    // Determinism also across a *different* repeat count (structure,
    // not a memoised constant).
    let r3a = expect_errs(&line.repeat(3));
    let r3b = expect_errs(&line.repeat(3));
    assert_eq!(r3a, r3b);
    assert!(r3a.errors().len() < r1.errors().len(), "scales with input");
}

/// Single-error negative-test helper.
///
/// Migrated for TASK-0080/0081: `parse_algo` now returns `ParseErrors`
/// (a non-empty bundle); this helper returns the **first** (earliest,
/// positional) error. This is an **assertion-strength-preserving**
/// mechanical migration: the pre-recovery tests asserted exactly
/// `parse_algo(src).expect_err()`'s `.line`/`.column`/`.kind`; that
/// single error is precisely `ParseErrors::first()`, with identical
/// discriminating power. The per-call-site `.line`/`.column`/`.kind`
/// checks are byte-for-byte unchanged — no assertion was loosened,
/// wildcarded, or removed.
///
/// Deliberately NOT also asserting `len() == 1` here: several of these
/// legacy fixtures place their sole error at/near end-of-input
/// (`const N : usize = 16` with no `;`, then EOF). `;`-anchored
/// recovery legitimately also reports the resulting structural
/// follow-on (a real `UnexpectedEof`, deterministic and bounded — the
/// program genuinely is truncated there), so a blanket exactly-one
/// assertion would be *false*, not stronger. The "single clean error
/// ⇒ exactly one error, no spurious cascade" property — the actual
/// AC, for the realistic "user has one typo, valid code after it"
/// case — is pinned precisely and separately by
/// [`single_error_input_yields_exactly_one_error_no_cascade`].
fn expect_err(src: &str) -> ParseError {
    parse_algo(src)
        .expect_err("expected parse error")
        .first()
        .clone()
}

// --------------------------------------------------------------------
// Span population (TASK-0082, AC#3)
// --------------------------------------------------------------------

/// Spans are populated and point at the exact source substring each
/// wrapped node was parsed from. We recover `&src[node.span]` and
/// compare it byte-for-byte against the expected slice, and validate
/// the start offset's `(line, column)` via the shared
/// `error::offset_to_line_col` helper (the same helper `ParseError`
/// uses), so a future diagnostic prints the right coordinates.
///
/// This is evidence, not just an assertion: the test reconstructs the
/// original token text from the span alone.
#[test]
fn spans_point_at_correct_source_substring() {
    use nucleus_compiler::error::offset_to_line_col;

    // Carefully laid-out source so offsets are predictable. Line 1 is
    // the const decl; line 2 declares data; line 3 is the for-loop
    // whose body (line 4) is a dataflow statement.
    let src = "\
const N : usize = 4;
data x : f32[N];
for i : 0 .. N {
    x[i] <-- inc(i);
}
";
    let ast = parse_algo(src).expect("must parse");

    // --- Item 0: the whole const declaration ---
    let item0 = &ast.items[0];
    assert_eq!(
        &src[item0.span.clone()],
        "const N : usize = 4;",
        "item 0 span must cover the entire const declaration"
    );
    assert_eq!(offset_to_line_col(src, item0.span.start), (1, 1));

    // The const's *name* identifier carries its own tight span: just
    // `N`, at line 1 column 7 (1-based) — what an "undeclared/duplicate
    // `N`" diagnostic would underline.
    let cn = match &item0.node {
        Item::Const(c) => &c.name,
        other => panic!("expected const; got {other:?}"),
    };
    assert_eq!(&src[cn.span.clone()], "N");
    assert_eq!(offset_to_line_col(src, cn.span.start), (1, 7));

    // The const's value expression `4` (single-token expr span).
    let cv = match &item0.node {
        Item::Const(c) => &c.value,
        _ => unreachable!(),
    };
    assert_eq!(&src[cv.span.clone()], "4");
    assert_eq!(offset_to_line_col(src, cv.span.start), (1, 19));

    // --- Item 1: data decl; its shape-dim expression `N` ---
    let data_dim = match &ast.items[1].node {
        Item::Data(d) => &d.ty.dims[0],
        other => panic!("expected data; got {other:?}"),
    };
    assert_eq!(&src[data_dim.span.clone()], "N");
    // `f32[N]` — the `N` is on line 2. Column: `data x : f32[` is 13
    // chars, so `N` is column 14.
    assert_eq!(offset_to_line_col(src, data_dim.span.start), (2, 14));

    // --- Item 2: the for-loop statement ---
    let item2 = &ast.items[2];
    // The item span covers the whole loop, including the closing `}`.
    assert_eq!(
        &src[item2.span.clone()],
        "for i : 0 .. N {\n    x[i] <-- inc(i);\n}"
    );
    assert_eq!(offset_to_line_col(src, item2.span.start), (3, 1));

    let for_stmt = match &item2.node {
        Item::Stmt(s) => s,
        other => panic!("expected stmt item; got {other:?}"),
    };
    let (var, body) = match &for_stmt.node {
        Stmt::For { var, body, .. } => (var, body),
        other => panic!("expected for; got {other:?}"),
    };
    // The loop variable `i` token.
    assert_eq!(&src[var.span.clone()], "i");
    assert_eq!(offset_to_line_col(src, var.span.start), (3, 5));

    // --- Body statement: the dataflow `x[i] <-- inc(i);` ---
    let body0 = &body[0];
    assert_eq!(&src[body0.span.clone()], "x[i] <-- inc(i);");
    assert_eq!(offset_to_line_col(src, body0.span.start), (4, 5));

    // Drill into the RHS call `inc(i)` and its callee identifier.
    let (lhs, rhs) = match &body0.node {
        Stmt::Dataflow { lhs, rhs } => (lhs, rhs),
        other => panic!("expected dataflow; got {other:?}"),
    };
    // LHS base identifier `x`.
    assert_eq!(&src[lhs.name.span.clone()], "x");
    assert_eq!(offset_to_line_col(src, lhs.name.span.start), (4, 5));
    // RHS expression `inc(i)`.
    assert_eq!(&src[rhs.span.clone()], "inc(i)");
    let call = match &rhs.node {
        nucleus_compiler::algo::Expr::Call(c) => c,
        other => panic!("expected call rhs; got {other:?}"),
    };
    // Callee identifier `inc` — column 14 on line 4: `    x[i] <-- `
    // is 13 chars (4 spaces, `x[i]`, space, `<--`, space).
    assert_eq!(&src[call.callee.span.clone()], "inc");
    assert_eq!(offset_to_line_col(src, call.callee.span.start), (4, 14));
    // The single argument `i` (an LValue-shaped bare ident — its span
    // is the identifier token).
    assert_eq!(&src[call.args[0].span.clone()], "i");

    // Spanned equality must IGNORE the span (AC#2): two structurally
    // identical idents from different source positions compare equal.
    let a = Spanned::new("z".to_string(), 0..1);
    let b = Spanned::new("z".to_string(), 99..100);
    assert_eq!(a, b, "Spanned PartialEq must exclude the span");
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut ha = DefaultHasher::new();
    let mut hb = DefaultHasher::new();
    a.hash(&mut ha);
    b.hash(&mut hb);
    assert_eq!(ha.finish(), hb.finish(), "Spanned Hash must exclude span");
}

// --------------------------------------------------------------------
// for{} body cascade — parametric over-n measurement (TASK-0207),
// flipped post-fix (TASK-0199): brace-balanced recovery collapses the
// pre-fix `constant 2` shape to `EXACTLY 1`.
// --------------------------------------------------------------------

/// TASK-0199 post-fix pin (renamed from
/// `for_body_error_surfaces_constant_two_parametric`, TASK-0207).
///
/// The algo for{}-body recovery shape now collapses to **exactly 1**
/// error (the primary alone) for any number `n` of valid trailing
/// for-body statements after the primary `@`-typo. This is the
/// post-fix counterpart of the sched parametric fixtures
/// (`sched_parser.rs::nested_brace_body_error_surfaces_single_primary_after_keyword_sync_*`)
/// — both algo and sched now share `== 1` post-fix; the pre-fix
/// counts (`constant 2` algo, `n + 2` sched) diverged for a structural
/// reason that the brace-balanced recovery makes moot.
///
/// # Mechanism (TASK-0199, verified by code-read of
/// `algo/parser.rs::brace_balanced_recovery`)
///
/// The historical `;`-only `skip_until([';'], …).consume_end()` had a
/// genuine recovery defect: when a typo fell inside a brace-delimited
/// body, the outer recovery consumed the typo's inner `;` and landed
/// mid-body, producing follow-on noise.
///
/// The fix replaces that with a brace-balanced
/// `skip_parser(brace_balanced_recovery())` that consumes one "logical
/// item span" per recovery step:
/// - a bare `;` (degenerate stray-terminator case), OR
/// - one or more outer atoms then an optional terminating `;`.
///   An outer atom is either a recursively-balanced `{ … }` block
///   (inner `;` consumed transparently as nested content) OR any
///   single char that is not `{`, `}`, or `;`.
///
/// For the algo `for i : 0 .. N { stmt; stmt; … }` shape: when a
/// stmt inside the body fails, recovery safe-char-consumes through
/// `for i : 0 .. N `, then the brace-block arm consumes the entire
/// `{ … }` balanced body in one step (including all inner `;`),
/// leaving the stream cleanly at EOF (or at the next valid item).
/// No follow-on. **1 error** regardless of `n`.
///
/// Pre-fix mechanism (the rationale that this rename supersedes):
/// after the `;`-only recovery consumed the typo's `;` and landed
/// mid-body, algo's top-level grammar accepted `Stmt` items so each
/// residual `x[i] <-- inc(i);` line parsed cleanly as an item — zero
/// re-failures, leaving only one structural close-`}` follow-on, for
/// a `constant 2` total independent of `n`. Sched's top-level grammar
/// accepted only directive-keyword-led items so each residual
/// field-keyword line re-failed the directive parser and re-triggered
/// recovery, for an `n + 2` linear cascade. Both diverged from each
/// other AND from the desirable `== 1`; the brace-balanced recovery
/// collapses both.
///
/// # Probed dimensions
///
/// `n ∈ {0, 1, 2, 5}` (matching the sched parametric fixtures
/// exactly). The pre-fix out-of-fixture probes `{3, 8, 12}` all
/// returned `2` (constant); post-fix all return `1`. Primary error
/// position remains pinned at line 5 column 14 (the `@`) regardless
/// of `n` — this is what makes the assertion meaningful (the primary
/// must still be correctly located, not just present).
///
/// # Multi-error preservation
///
/// Genuinely-independent errors in DIFFERENT items are still
/// reported separately — see
/// `multi_error_two_independent_errors_both_reported`. The recovery
/// consumes ONE item's span per invocation, so N independent errors
/// produce N errors. The brace-balanced recovery does NOT swallow
/// errors that legitimately belong to following items.
#[test]
fn for_body_error_surfaces_single_primary_after_keyword_sync() {
    for n in [0usize, 1, 2, 5] {
        // Lines:
        //   1: `const N : usize = 4;`
        //   2: `data x : f32[N];`
        //   3: `kernel inc : (f32) -> f32 pure;`
        //   4: `for i : 0 .. N {`
        //   5: `    x[i] <-- @;`              (PRIMARY, @ at col 14)
        //   6..5+n: `    x[i] <-- inc(i);`    (n valid trailing stmts —
        //                                      all swallowed by the
        //                                      brace-balanced recovery)
        //   6+n: `}`                          (body-close, also swallowed)
        //   7+n: (EOF)
        let mut src = String::from("const N : usize = 4;\n");
        src.push_str("data x : f32[N];\n");
        src.push_str("kernel inc : (f32) -> f32 pure;\n");
        src.push_str("for i : 0 .. N {\n");
        src.push_str("    x[i] <-- @;\n");
        for _ in 0..n {
            src.push_str("    x[i] <-- inc(i);\n");
        }
        src.push_str("}\n");

        let e1 = expect_errs(&src);
        let e2 = expect_errs(&src);
        assert_eq!(
            e1, e2,
            "n={n}: parametric algo for{{}}-body recovery must be \
             deterministic across two runs"
        );
        let es = e1.errors();

        // CORE ASSERTION (post-fix): exactly 1 across all probed n.
        // The brace-balanced recovery consumes the entire `for { … }`
        // block as a single recovery atom; no structural close-`}`
        // follow-on remains.
        assert_eq!(
            es.len(),
            1,
            "n={n}: expected EXACTLY 1 error (primary only); got {} — \
             TASK-0199 brace-balanced recovery should collapse the \
             pre-fix `constant 2` shape. source:\n{src}\nerrors: {es:?}",
            es.len()
        );

        // Primary at line 5 column 14 (the `@`), regardless of n.
        // Position-pinning is what makes "errors().len() == 1"
        // discriminating: it must be the GENUINE primary at the typo,
        // not some bogus error elsewhere.
        assert_eq!(
            (es[0].line, es[0].column),
            (5, 14),
            "n={n}: primary `@` must be at (L5, C14); got {es:?}"
        );
        assert_eq!(es[0].kind, ParseErrorKind::Unexpected, "n={n}: {es:?}");
        assert_eq!(
            e1.first().line,
            5,
            "n={n}: primary must be earliest in deterministic order: {es:?}"
        );
    }
}

/// TASK-0199 review-gate cycle-2 regression: a failing brace-bodied
/// item (e.g. `for { … }` with a typo inside) followed by a VALID
/// item at the top level must NOT swallow the valid item. The
/// pre-cycle-2 brace-balanced recovery's `outer_atom.repeated()`
/// greedily consumed safe chars across the brace-block boundary up
/// to the next `;`, silently swallowing the subsequent `const OK : usize = 7;`
/// from both the error set AND the AST. QA review (TASK-0199 gate)
/// caught this via PROBE 6/6a; the fix splits the recovery into a
/// `brace_block_item` arm (one brace block + optional `;`, then STOP)
/// and a `flat_item` arm (safe chars + required `;` or end()).
///
/// Without the cycle-2 split, this test produces 2 errors
/// (`Unexpected` at the `@` typo + `UnexpectedEof`), and the `const OK`
/// is silently dropped from the AST. With the split, the `for { … }`
/// recovery span stops at the closing `}`, and the subsequent
/// `const OK : usize = 7;` parses cleanly as a separate top-level
/// item — exactly 1 error (the `@` typo only).
#[test]
fn brace_bodied_item_recovery_does_not_swallow_subsequent_valid_item() {
    let src = "\
const N : usize = 5;
data x : f32[N];
kernel inc : (f32) -> f32 pure;
for i : 0 .. N {
    x[i] <-- @;
}
const OK : usize = 7;
";
    let e1 = expect_errs(src);
    let e2 = expect_errs(src);
    assert_eq!(
        e1, e2,
        "brace-bodied + valid-item recovery must be deterministic"
    );

    let es = e1.errors();
    assert_eq!(
        es.len(),
        1,
        "expected EXACTLY 1 error (the `@` typo); subsequent valid \
         `const OK` must NOT be swallowed by the for-body recovery: {es:?}"
    );
    assert_eq!(
        (es[0].line, es[0].column),
        (5, 14),
        "primary at the `@`: {es:?}"
    );
    assert_eq!(es[0].kind, ParseErrorKind::Unexpected, "{es:?}");

    // Crucially, the AST must contain `const OK` — the valid item
    // survived the brace-bodied item's recovery. We can't directly
    // inspect the AST here (parse_algo returns Err), but the
    // 1-error count is the load-bearing assertion: a failing
    // recovery that consumed `const OK` would have produced an
    // `UnexpectedEof` (2 errors total) because the recovery span
    // would terminate at the `;` after `7`.
}

/// TASK-0199 review-gate cycle-2 regression (sched analog): a
/// failing brace-bodied directive followed by a VALID directive
/// must NOT swallow the valid directive. The sched grammar bounds
/// the over-consumption via the required trailing `;` after `}`,
/// but we pin the symmetric property explicitly for cross-layer
/// confidence.
#[test]
fn brace_bodied_directive_recovery_preserves_valid_directive_at_eof() {
    // Algo-side analog: a failing flat item followed by a brace-
    // bodied item at the very end of source. The flat-item recovery
    // requires `;` OR end() as its terminator; the brace-bodied item
    // immediately following must NOT be consumed as residue of the
    // flat item's safe-char sequence.
    let src = "\
const N : usize = 5;
data x : f32[?];
for i : 0 .. N {
    x[i] <-- x[i];
}
";
    let e1 = expect_errs(src);
    let e2 = expect_errs(src);
    assert_eq!(e1, e2, "must be deterministic");

    let es = e1.errors();
    assert_eq!(
        es.len(),
        1,
        "expected EXACTLY 1 error (the `?` in data shape); the \
         subsequent valid for-loop must parse cleanly and not \
         contribute follow-on errors: {es:?}"
    );
    assert_eq!(es[0].line, 2, "primary on line 2 (the `?`): {es:?}");
}

// --------------------------------------------------------------------
// TASK-0341.03.01: nested-index (gather) parses to a nested Expr::LValue.
// --------------------------------------------------------------------

/// Return the first kernel-call argument of the first dataflow statement
/// in the first top-level `for` body (the `src[idx[i]]` arg below).
fn first_for_dataflow_first_arg(ast: &AlgoAst) -> &Expr {
    let body = first_for_body(ast);
    let rhs = body
        .iter()
        .find_map(|s| match &s.node {
            Stmt::Dataflow { rhs, .. } => Some(&rhs.node),
            _ => None,
        })
        .expect("for body must hold a dataflow statement");
    match rhs {
        Expr::Call(c) => &c.args.first().expect("call must have an arg").node,
        other => panic!("expected a kernel call rhs, got {other:?}"),
    }
}

/// AC#2 (positive): a data-dependent gather `g(src[idx[i]])` PARSES — the
/// parser admits a nested index because `index_tail` uses the full
/// recursive expression grammar (the rejection was only ever in
/// lowering, now lifted for subscript position). The arg must be an
/// `Expr::LValue` (`src`) whose FIRST index is ITSELF an indexed
/// `Expr::LValue` (`idx[i]`) — the nested-gather AST shape. Pins the
/// parser against a future tightening of `index_tail` to int/ident-only
/// that would "enforce" the stale grammar-doc and silently break gather.
#[test]
fn task0341_0301_nested_gather_index_parses_to_nested_lvalue() {
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
    let ast = parse_algo(src).expect("a gather `src[idx[i]]` must parse");
    let arg = first_for_dataflow_first_arg(&ast);
    match arg {
        Expr::LValue(IndexedLValue { name, indices }) => {
            assert_eq!(name.node, "src", "outer ref must be `src`");
            assert_eq!(indices.len(), 1, "`src` has one index");
            match &indices[0].node {
                Expr::LValue(inner) => {
                    assert_eq!(
                        inner.name.node, "idx",
                        "the index must be the data ref `idx`"
                    );
                    assert_eq!(inner.indices.len(), 1, "`idx[i]` has one (affine) index");
                }
                other => panic!(
                    "the index of `src` must be a NESTED indexed LValue `idx[i]` (the \
                     gather), got {other:?}"
                ),
            }
        }
        other => panic!("expected `src[idx[i]]` as an Expr::LValue, got {other:?}"),
    }
}

/// AC#2 (negative): an AFFINE index `g(src[i])` is NOT misclassified as a
/// gather — its single index is a BARE `Expr::LValue` (`i`, empty
/// indices), not a nested indexed ref. Guards against treating every
/// `Expr::LValue` index as data-dependent.
#[test]
fn task0341_0301_affine_index_is_not_a_nested_gather() {
    let src = "\
const N : usize = 4;
data src : i32[N];
data out : i32[N];
kernel g : (i32) -> i32 pure;
for i : 0 .. N {
    out[i] <-- g(src[i]);
}
";
    let ast = parse_algo(src).expect("an affine `src[i]` must parse");
    let arg = first_for_dataflow_first_arg(&ast);
    match arg {
        Expr::LValue(IndexedLValue { indices, .. }) => match &indices[0].node {
            Expr::LValue(inner) => assert!(
                inner.indices.is_empty(),
                "an affine index `i` must be a BARE LValue (empty indices), not a \
                 nested gather; got indices {:?}",
                inner.indices
            ),
            other => panic!("affine index `i` should be a bare LValue, got {other:?}"),
        },
        other => panic!("expected `src[i]` as an Expr::LValue, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// TASK-0341.02.01.02 / epic S2: relational (bool-valued) operators.
// Parser-level coverage: every operator parses to an `Expr::Compare`
// with the right `CmpOp`; precedence sits BELOW additive; chaining
// (`a < b < c`) does NOT parse (single, non-associative).
// --------------------------------------------------------------------

/// Project the FIRST top-level dataflow RHS expression. (S2 reaches a
/// comparison via the dataflow RHS path; see lowering notes.)
fn first_toplevel_dataflow_rhs(ast: &AlgoAst) -> &Expr {
    ast.items
        .iter()
        .find_map(|i| match &i.node {
            Item::Stmt(s) => match &s.node {
                Stmt::Dataflow { rhs, .. } => Some(&rhs.node),
                _ => None,
            },
            _ => None,
        })
        .expect("expected a top-level dataflow statement")
}

/// Build a minimal program whose single bool-typed dataflow RHS is
/// `EXPR`. `flag : bool` is the LHS; `a`,`b` are i32 data so the
/// comparison has integer operands.
fn prog_with_bool_rhs(rhs: &str) -> String {
    format!(
        "\
data a : i32;
data b : i32;
data flag : bool;
flag <-- {rhs};
"
    )
}

#[test]
fn task0341_020102_each_relational_operator_parses_to_compare() {
    for (src_op, want) in [
        ("<=", CmpOp::Le),
        ("<", CmpOp::Lt),
        ("==", CmpOp::Eq),
        ("!=", CmpOp::Ne),
        (">", CmpOp::Gt),
        (">=", CmpOp::Ge),
    ] {
        let src = prog_with_bool_rhs(&format!("a {src_op} b"));
        let ast = parse_algo(&src).unwrap_or_else(|e| panic!("`a {src_op} b` must parse: {e:?}"));
        match first_toplevel_dataflow_rhs(&ast) {
            Expr::Compare(op, lhs, rhs) => {
                assert_eq!(*op, want, "operator `{src_op}` must parse as {want:?}");
                // Operands are bare LValues `a`, `b`.
                assert!(
                    matches!(&lhs.node, Expr::LValue(lv) if lv.name.node == "a"),
                    "lhs must be `a`, got {:?}",
                    lhs.node
                );
                assert!(
                    matches!(&rhs.node, Expr::LValue(lv) if lv.name.node == "b"),
                    "rhs must be `b`, got {:?}",
                    rhs.node
                );
            }
            other => panic!("`a {src_op} b` must be Expr::Compare, got {other:?}"),
        }
    }
}

/// Precedence: relational sits BELOW additive, so `a + b <= c` parses
/// as `(a + b) <= c` — the comparison is the OUTER node and its lhs is
/// the additive subtree, NOT `a + (b <= c)`.
#[test]
fn task0341_020102_relational_is_below_additive() {
    let src = prog_with_bool_rhs("a + b <= a");
    let ast = parse_algo(&src).expect("`a + b <= a` must parse");
    match first_toplevel_dataflow_rhs(&ast) {
        Expr::Compare(CmpOp::Le, lhs, _rhs) => {
            // The comparison's LHS must be the additive `a + b`.
            assert!(
                matches!(&lhs.node, Expr::Binary(BinOp::Add, _, _)),
                "lhs of `<=` must be the additive `(a + b)` (relational below additive), got {:?}",
                lhs.node
            );
        }
        other => panic!("`a + b <= a` must be a top-level Compare(Le), got {other:?}"),
    }
}

/// Non-associative single comparison: chaining `a < b < c` must NOT
/// parse (the grammar admits at most one `(CmpOp AddExpr)?`). The
/// trailing `< c` is left unconsumed and the statement parser reports an
/// error rather than building a nested Compare.
#[test]
fn task0341_020102_chained_comparison_does_not_parse() {
    let src = prog_with_bool_rhs("a < b < b");
    let res = parse_algo(&src);
    assert!(
        res.is_err(),
        "chained `a < b < b` must NOT parse (single non-associative comparison); got {res:?}"
    );
}

// --------------------------------------------------------------------
// TASK-0341.02.01.03 / epic S1: `for IDENT : LO .. HI until COND { … }`
// bounded early-exit loop surface syntax. Parser-level coverage: the
// optional `until COND` clause parses into the new `Stmt::For.until`
// field; a plain loop leaves it `None`; a malformed `until` (no COND
// before `{`) is rejected with the diagnostic anchored at the `until`
// token (TASK-0434 VAR-anchoring precedent). INERT — rejected later at
// the ACFG boundary (covered in `algo_lower` / `acfg_build` tests).
// --------------------------------------------------------------------

/// Project the FIRST top-level `for` statement (the whole `Stmt::For`,
/// not just its body) so the `until` field can be inspected.
fn first_toplevel_for(ast: &AlgoAst) -> &Stmt {
    ast.items
        .iter()
        .find_map(|i| match &i.node {
            Item::Stmt(s) => match &s.node {
                f @ Stmt::For { .. } => Some(f),
                _ => None,
            },
            _ => None,
        })
        .expect("expected a top-level for statement")
}

/// AC#1 positive: `for i : 0 .. N until COND { … }` parses with the new
/// AST node carrying var / bounds / COND / body. The COND is a bool
/// comparison (`a <= b`), reached through the same `Expr::Compare` path
/// S2 added.
#[test]
fn task0341_020103_for_until_parses_with_cond_field() {
    let src = "\
const N : usize = 8;
data a : i32;
data b : i32;
data x : i32[N];
for i : 0 .. N until a <= b {
    x[i] <-- inc(i);
}
";
    let ast = parse_algo(src).expect("`for i : 0..N until a <= b { … }` must parse");
    match first_toplevel_for(&ast) {
        Stmt::For {
            var,
            lo,
            hi,
            until,
            body,
        } => {
            assert_eq!(var.node, "i", "loop var must be `i`");
            assert!(
                matches!(&lo.node, Expr::IntLit(0)),
                "lo bound must be `0`, got {:?}",
                lo.node
            );
            assert!(
                matches!(&hi.node, Expr::LValue(lv) if lv.name.node == "N"),
                "hi (cap) bound must be `N`, got {:?}",
                hi.node
            );
            let cond = until
                .as_ref()
                .expect("`until` clause must populate the until field");
            assert!(
                matches!(&cond.node, Expr::Compare(CmpOp::Le, _, _)),
                "until COND must parse as `a <= b` (Compare(Le)), got {:?}",
                cond.node
            );
            assert_eq!(body.len(), 1, "body must hold the single dataflow stmt");
        }
        other => panic!("expected Stmt::For, got {other:?}"),
    }
}

/// AC#1 negative-control: a PLAIN fixed-iteration loop (no `until`)
/// still parses and leaves `until: None` — the field is genuinely
/// optional and the existing loop surface is unchanged.
#[test]
fn task0341_020103_plain_for_leaves_until_none() {
    let src = "\
const N : usize = 4;
data x : i32[N];
for i : 0 .. N {
    x[i] <-- inc(i);
}
";
    let ast = parse_algo(src).expect("plain `for` must still parse");
    match first_toplevel_for(&ast) {
        Stmt::For { until, .. } => assert!(
            until.is_none(),
            "a plain for-loop must leave `until` as None, got {until:?}"
        ),
        other => panic!("expected Stmt::For, got {other:?}"),
    }
}

/// AC#3: a malformed `until` (the keyword present but NO condition
/// before the body `{`) is rejected with the diagnostic anchored AT
/// the `until` token, not at a diffuse whole-loop span or the `{`.
/// `until` is on line 3 starting at column 16 (`for i : 0 .. N ` is 15
/// chars, so `until` begins at column 16).
#[test]
fn task0341_020103_malformed_until_is_anchored_at_the_until_token() {
    let src = "\
const N : usize = 4;
data x : i32[N];
for i : 0 .. N until {
    x[i] <-- inc(i);
}
";
    assert!(
        parse_algo(src).is_err(),
        "`until` with no condition before `{{` must be rejected"
    );
    let err = expect_err(src);
    assert_eq!(
        err.line, 3,
        "malformed-until diagnostic must anchor on the `until` line, got {err:?}"
    );
    assert_eq!(
        err.column, 16,
        "malformed-until diagnostic must anchor at the `until` token \
         (column 16), not the trailing `{{`, got {err:?}"
    );
    assert!(
        err.message.contains("until") && err.message.contains("TASK-0341.02.01.03"),
        "diagnostic must name the `until` clause and the epic, got: {}",
        err.message
    );
}

/// `until` is a RESERVED word (added to algo KEYWORDS): it may not be a
/// loop variable / identifier, which is what keeps the optional-clause
/// grammar LL(1). Using it as a for-var is rejected.
#[test]
fn task0341_020103_until_is_a_reserved_word() {
    let src = "\
const N : usize = 4;
for until : 0 .. N {
}
";
    assert!(
        parse_algo(src).is_err(),
        "`until` must be reserved and rejected as a loop variable"
    );
}
