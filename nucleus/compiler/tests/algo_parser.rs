//! Integration tests for the algorithm-sublanguage parser.
//!
//! Test strategy (hand-rolled assertions, no `insta`):
//! - For each existing example `prog.algo.nuc`, assert structural
//!   counts (consts, data, kernels, top-level statements). Snapshotting
//!   the full AST would be brittle as we evolve formatting / AST shape.
//! - Negative tests: hand-written invalid sources must return an `Err`
//!   with the expected `ParseErrorKind`.
//!
//! Known-failing example: `05-stencil/prog.algo.nuc` uses the legacy
//! `where pure {{ ... }}` syntax (PRD §6.2.2 deprecates it). The
//! parser MUST reject it — this is asserted as a negative test rather
//! than skipped. See TASK-0078 for the rewrite.
//!
//! Stable across edits: we assert *counts*, not exact AST trees. If
//! the example files grow or shrink, update the counts here.

use compiler::algo::{parse_algo, Item, ParseError, Purity, ScalarType, Stmt};

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
    let for_body = ast
        .items
        .iter()
        .find_map(|i| match i {
            Item::Stmt(Stmt::For { body, .. }) => Some(body),
            _ => None,
        })
        .expect("expected a for-loop at top level");
    assert_eq!(for_body.len(), 1, "for body should have 1 statement");

    // Spot-check purities and that `add` is the scalar kernel.
    let kernels: Vec<_> = ast
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Kernel(k) => Some(k),
            _ => None,
        })
        .collect();
    let by_name = |name: &str| {
        kernels
            .iter()
            .find(|k| k.name == name)
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
    let kernels: Vec<_> = ast
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Kernel(k) => Some(k),
            _ => None,
        })
        .collect();
    let by_name = |name: &str| {
        kernels
            .iter()
            .find(|k| k.name == name)
            .unwrap_or_else(|| panic!("missing kernel {}", name))
    };
    assert_eq!(by_name("load_input").purity, Purity::Effectful);
    assert_eq!(by_name("save_output").purity, Purity::Effectful);
    assert_eq!(by_name("conv_block_1").purity, Purity::Pure);
    assert_eq!(by_name("classifier").purity, Purity::Pure);

    // The for-loop body should contain 3 dataflow statements.
    let for_body = ast
        .items
        .iter()
        .find_map(|i| match i {
            Item::Stmt(Stmt::For { body, .. }) => Some(body),
            _ => None,
        })
        .expect("expected a for-loop at top level");
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
    let for_body = ast
        .items
        .iter()
        .find_map(|i| match i {
            Item::Stmt(Stmt::For { body, .. }) => Some(body),
            _ => None,
        })
        .expect("expected a for-loop at top level");
    assert_eq!(for_body.len(), 6, "for body should have 6 statements");

    // mix2 has 2 params -> assert structurally.
    let mix2 = ast
        .items
        .iter()
        .find_map(|i| match i {
            Item::Kernel(k) if k.name == "mix2" => Some(k),
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
        .find_map(|i| match i {
            Item::Kernel(k) if k.name == "rf_transmit" => Some(k),
            _ => None,
        })
        .expect("missing rf_transmit kernel");
    assert!(rf_transmit.sig.ret.is_none(), "rf_transmit returns ()");
    assert_eq!(rf_transmit.purity, Purity::Effectful);
}

/// `05-stencil/prog.algo.nuc` uses the retired 2013-style
/// `kernel foo(a,b) -> out where pure {{ ... }};` syntax. The v2
/// grammar (PRD §6.2.2) requires kernel bodies to live in Rust source,
/// not inlined with `{{ ... }}` substitution. The parser MUST reject
/// this file. The rewrite is tracked by TASK-0078.
#[test]
fn rejects_legacy_05_stencil() {
    let src = read_example("05-stencil/prog.algo.nuc");
    let err = parse_algo(&src).expect_err(
        "05-stencil uses the legacy `where pure {{...}}` syntax and must be rejected. \
         TODO: TASK-0078 rewrites the example into v2 form.",
    );
    // We don't pin the exact kind here because the input mismatch can
    // surface in several places; just assert that a position is set.
    assert!(err.line >= 1);
    assert!(err.column >= 1);
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
    let d = match &ast.items[1] {
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

fn expect_err(src: &str) -> ParseError {
    parse_algo(src).expect_err("expected parse error")
}
