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

use compiler::algo::{
    parse_algo, AlgoAst, Item, KernelDecl, ParseError, ParseErrorKind, ParseErrors, Purity,
    ScalarType, Stmt,
};
use compiler::algo::span::Spanned;

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
    // CARGO_MANIFEST_DIR is the path to `nucleus/compiler/`. Examples
    // live at `<repo>/nuc-nucleus/examples/...`. Walk up two levels.
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
    let src = read_example("14-hearing-aid/prog.algo.nuc");
    let ast = parse_algo(&src).expect("14-hearing-aid must parse");

    assert_eq!(ast.count_consts(), 2, "expected 2 const decls");
    assert_eq!(ast.count_data(), 4, "expected 4 data decls");
    assert_eq!(ast.count_kernels(), 6, "expected 6 kernel decls");
    // Only the `for frame : ...` loop at top level.
    assert_eq!(ast.count_stmts(), 1, "expected 1 top-level statement");

    // Inside the loop body we expect 6 statements (2 captures, 2
    // outbound, 2 inbound).
    let for_body = first_for_body(&ast);
    assert_eq!(for_body.len(), 6, "for body should have 6 statements");

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

    // rf_transmit returns unit.
    let rf_transmit = ast
        .items
        .iter()
        .find_map(|i| match &i.node {
            Item::Kernel(k) if k.name.node == "rf_transmit" => Some(k),
            _ => None,
        })
        .expect("missing rf_transmit kernel");
    assert!(rf_transmit.sig.ret.is_none(), "rf_transmit returns ()");
    assert_eq!(rf_transmit.purity, Purity::Effectful);
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
    // The parser sees `block` as an ident followed by `=`, which is
    // not a legal statement start.
    let src = "\
const H : usize = 600;
for y : 1 .. H {
    block = 64;
}
";
    let err = expect_err(src);
    // The `=` is on line 3.
    assert_eq!(err.line, 3, "{:?}", err);
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
    parse_algo(src).expect_err("expected parse error").first().clone()
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
    use compiler::error::offset_to_line_col;

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
        compiler::algo::Expr::Call(c) => c,
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
// for{} body cascade — parametric over-n measurement (TASK-0207)
// --------------------------------------------------------------------

/// TASK-0207: parametric over-n measurement of the algo for{}-body
/// recovery shape. This is the algo sibling of TASK-0087 cycle-4's
/// sched parametric fixtures
/// (`sched_parser.rs::nested_brace_body_error_surfaces_n_plus_two_parametric_{worker_class,memory_region}`).
///
/// **HONEST DISCREPANCY FROM SCHED** (this is the load-bearing finding
/// of TASK-0207, surfaced to the recurring undercount-honesty class):
/// the algo for{}-body case is **NOT** an `n+2` cascade. Measured
/// empirically on `parse_algo` (deterministic across two runs at every
/// probed `n`), the count is the **constant `2`** regardless of `n` —
/// the primary error plus a single structural `Unexpected` follow-on
/// pointing at the line *after* the for{}-body close-`}`, which is the
/// first token the OUTER program-level `skip_until([';'])` recovery
/// reaches without a leading `;` to consume.
///
/// **Root cause of the algo/sched divergence** (structural, not a bug
/// in either):
/// - **Sched** parses brace bodies (`worker_class IDENT { field;
///   field; }`) with an INNER `;`-anchored
///   `field.recover_with(skip_until(...)).repeated()`, so each valid
///   inner-`;` after the primary fires its own field-level recovery
///   error → `n` per-field cascade entries → `n + 2` total (primary +
///   n inner-field cascades + structural close-`}` follow-on).
/// - **Algo** parses for{}-body statements with a bare
///   `stmt.clone().repeated()` (no inner recovery; see
///   `algo/parser.rs::stmt_parser` for-arm). On the primary failure
///   the entire `for_stmt` alternative bails to the OUTER program-
///   level recovery, which `skip_until([';'])` straight through the
///   whole body — eating *every* inner `;` (valid and invalid alike)
///   — and stops at the bare body-close `}` (a non-item token), where
///   it emits one structural `Unexpected`. The body's `n` valid
///   trailing `;`s contribute **zero** per-field cascade entries
///   because the algo grammar has no per-statement recovery layer at
///   this depth.
///
/// **Implication for TASK-0199**: when the keyword-anchored sync set
/// fix lands, both algo and sched will collapse to **`== 1`** (the
/// primary alone). The mechanical edit in this fixture is the same as
/// the sched siblings — replace `2` with `1` in the `expected` literal
/// (one line). For TASK-0199 AC#7 the algo and sched fixtures now
/// share the same flip-to-1 shape; the pre-fix counts diverge
/// (`2` algo, `n + 2` sched) but the post-fix count is identical.
///
/// **Probed dimensions** (n = number of valid trailing for-body
/// statements after the primary `@`-typo): `{0, 1, 2, 5}`, matching
/// the sched sibling fixture exactly. Out-of-fixture probes in this
/// cycle's measurement (`{3, 8, 12}`) also returned `len() == 2`,
/// confirming the constant-2 plateau is not a small-n artefact and
/// the masking-defect class (single-n fixture cannot tell constant-2
/// apart from 2/2/3/4 or 2/3/4/5) is closed at the algo layer
/// independently of the closure at the sched layer.
///
/// Primary error position pinned at line 5 column 14 (the `@`) for
/// all probed `n` (line 5 is the first body line; `    x[i] <-- @;`
/// — 4 spaces + `x[i] <-- ` = 13 chars, so `@` is column 14). The
/// structural follow-on sits at line `6 + n` column 1 (the line of
/// the body-close `}`; with the body-close on the line immediately
/// after the last trailing valid `inc(i);`).
#[test]
fn for_body_error_surfaces_constant_two_parametric() {
    for n in [0usize, 1, 2, 5] {
        // Lines:
        //   1: `const N : usize = 4;`
        //   2: `data x : f32[N];`
        //   3: `kernel inc : (f32) -> f32 pure;`
        //   4: `for i : 0 .. N {`
        //   5: `    x[i] <-- @;`              (PRIMARY, @ at col 14)
        //   6..5+n: `    x[i] <-- inc(i);`    (n valid trailing stmts)
        //   6+n: `}`                          (body-close, col 1 follow-on)
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

        // CORE ASSERTION: constant 2 across all probed n (NOT n+2 —
        // see test-level docstring for the structural why).
        let expected = 2;
        assert_eq!(
            es.len(),
            expected,
            "n={n}: expected exactly 2 errors (primary + structural \
             follow-on at body-close `}}`), got {} — algo for-body \
             count is CONSTANT in n (unlike sched n+2). \
             source:\n{src}\nerrors: {es:?}",
            es.len()
        );

        // Primary at line 5 column 14 (the `@`), regardless of n.
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

        // Structural follow-on at line (6 + n), column 1 — the
        // body-close `}` line (a non-item token the outer
        // `skip_until([';'])` cannot consume past). This pins the
        // OUTER-recovery sync exit point, which is what would shift
        // if the recovery sync set changed.
        let close_line = 6 + n;
        assert_eq!(
            (es[1].line, es[1].column),
            (close_line, 1),
            "n={n}: structural follow-on must be at (L{close_line}, C1) \
             — the body-close `}}` line; got {es:?}"
        );
        assert_eq!(
            es[1].kind,
            ParseErrorKind::Unexpected,
            "n={n}: structural follow-on kind: {es:?}"
        );
    }
}


