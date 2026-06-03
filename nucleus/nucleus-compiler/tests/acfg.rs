//! Integration tests for the ACFG construction pass (TASK-0016).
//!
//! Strategy:
//!
//! - **Positive**: build an ACFG for every existing (algorithm,
//!   schedule) pair that the link tests already cover. Assert
//!   structural properties (operation count, repeat depth, top-level
//!   shape) rather than snapshotting the whole tree — snapshots rot
//!   fast and the task spec explicitly asks for structural
//!   properties.
//!
//! - **Determinism**: build the ACFG twice from the same input and
//!   assert `==`. The deterministic name -> ID mapping (see
//!   `crate::acfg` docs) must produce identical IDs across runs.
//!
//! M11 admission (mirrors `tests/link.rs`):
//! - `14-hearing-aid/embedded_multimcu.sched.nuc` (the M11 multi-MCU
//!   schedule, built against the per-frame `prog.embedded.algo.nuc`)
//!   was ADMITTED into the ACFG matrix at TASK-0192 — see
//!   `acfg_example_14_embedded_multimcu`.
//!
//! 05-stencil was historically a parse failure (legacy 2013 syntax);
//! TASK-0078 / TASK-0031 rewrote it into v2 form. Adding a dedicated
//! ACFG smoke test for example 05 is filed as a low-priority
//! follow-up — the link-test pinning (links_05_stencil_*) already
//! covers the upstream pipeline and the e2e cell covers the
//! downstream pipeline end-to-end.
//!
//! What this file does NOT cover (filed as follow-ups in the task
//! self-report):
//! - The full data-flow contents of each `Operation`. The structural
//!   count is enough for M1; equivalence-by-hashing tests live with
//!   the richer-DAG follow-up.
//! - Error paths. `build_acfg` panics on link-pass-invariant
//!   violations (unplaced kernel, non-const loop bound); those are
//!   invariants of the input, not user errors, and have no negative
//!   test path here.

use nucleus_compiler::acfg::{build_acfg, ACFGNode};
use nucleus_compiler::algo::{lower_algo, parse_algo, IrBinOp, IrExpr};
use nucleus_compiler::link;
use nucleus_compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Helpers (mirror tests/link.rs)
// --------------------------------------------------------------------

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

fn linked_from_paths(algo_rel: &str, sched_rel: &str) -> nucleus_compiler::LinkedIR {
    let algo_ast = parse_algo(&read_example(algo_rel)).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(sched_rel)).expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    link(algo, sched).expect("link must succeed")
}

// --------------------------------------------------------------------
// Positive: example × schedule pairs build a sensible ACFG
// --------------------------------------------------------------------

#[test]
fn acfg_example_1_naive() {
    // Example 1: load_input, load_input_b, then `for i : 0..256 { c[i] <-- add(a[i], b[i]); }`,
    // then save_output. That's 3 top-level Operations + 1 Repeat
    // containing 1 Operation = 4 Operations total, 1 Repeat.
    let linked = linked_from_paths(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    // Top-level shape is a Sequence.
    assert!(
        matches!(acfg.root, ACFGNode::Sequence(_)),
        "top-level must be Sequence, got {:?}",
        acfg.root
    );

    assert_eq!(
        acfg.operation_count(),
        4,
        "ops = load + load_b + add(in loop) + save"
    );
    assert_eq!(acfg.repeat_count(), 1, "one outer for-loop");
    assert_eq!(acfg.max_repeat_depth(), 1, "no nested loops");

    // Name table sanity.
    assert!(acfg.name_kernels.contains_key("add"));
    assert!(acfg.name_kernels.contains_key("load_input"));
    assert!(acfg.name_kernels.contains_key("save_output"));
    assert!(acfg.name_iter_vars.contains_key("i"));
    assert!(acfg.name_workers.contains_key("host"));
}

#[test]
fn acfg_example_13_naive() {
    // CNN inference, naive schedule (everything on host).
    // load_input (top-level), for n { conv1, conv2, classifier }, save_output.
    // 1 + 3-in-loop + 1 = 5 operations; 1 Repeat; depth 1.
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    assert_eq!(acfg.operation_count(), 5);
    assert_eq!(acfg.repeat_count(), 1);
    assert_eq!(acfg.max_repeat_depth(), 1);
}

#[test]
fn acfg_example_13_batch_parallel() {
    // Same algorithm so the structural counts are identical to naive;
    // what differs is the worker set inside each Operation. Spot-check
    // that distributed placement collapses into a multi-element
    // workers set.
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    assert_eq!(acfg.operation_count(), 5);
    assert_eq!(acfg.repeat_count(), 1);

    // Find an Operation for conv_block_1 and assert it's placed on
    // {w0,w1,w2,w3} — four workers.
    let conv1_id = acfg.name_kernels["conv_block_1"];
    let mut found_distributed = false;
    visit_operations(&acfg.root, &mut |op| {
        if op.kernel == conv1_id {
            assert_eq!(
                op.workers.len(),
                4,
                "conv_block_1 distributed over 4 workers"
            );
            found_distributed = true;
        }
    });
    assert!(
        found_distributed,
        "expected to see conv_block_1 in the tree"
    );
}

#[test]
fn acfg_example_13_pipeline_parallel() {
    // Same algorithm shape; the pipeline schedule places each layer
    // on its own singleton worker. Structural counts unchanged;
    // every Operation has exactly one worker.
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    assert_eq!(acfg.operation_count(), 5);
    assert_eq!(acfg.repeat_count(), 1);

    let mut max_set = 0usize;
    visit_operations(&acfg.root, &mut |op| {
        max_set = max_set.max(op.workers.len());
    });
    assert_eq!(max_set, 1, "pipeline schedule: every kernel on one worker");
}

#[test]
fn acfg_example_14_naive() {
    // Hearing-aid, naive (everything on host). Cycle 201 rewrite
    // (TASK-0054 reopen): bulk-IO + intermediate `mixed` symbol so
    // the for-loop body has no nested kernel call. Structure:
    //   mic_in   <-- load_mic()                      (top-level Op)
    //   bt_in    <-- load_bt()                       (top-level Op)
    //   for frame {
    //       bt_out[frame]  <-- denoise(mic_in[frame])
    //       mixed[frame]   <-- mix2(mic_in[frame], bt_in[frame])
    //       spk_out[frame] <-- denoise(mixed[frame])
    //   }                                             (3 ops in Repeat)
    //   save_spk(spk_out)                            (top-level Op)
    //   save_bt_out(bt_out)                          (top-level Op)
    // -> 7 operations total, 1 Repeat, max depth 1.
    let linked = linked_from_paths(
        "14-hearing-aid/prog.algo.nuc",
        "14-hearing-aid/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    assert_eq!(acfg.operation_count(), 7);
    assert_eq!(acfg.repeat_count(), 1);
    assert_eq!(acfg.max_repeat_depth(), 1);
}

/// TASK-0192 (M11 lowering-admission de-risk): build an ACFG for the
/// M11 multi-MCU `embedded_multimcu` schedule × the per-frame
/// `prog.embedded.algo.nuc`. This is the ACFG arm of the de-risk
/// (sched_lower + link arms live in `tests/sched_lower.rs` /
/// `tests/link.rs`).
///
/// Structure (per-frame IO, unlike the naive bulk-IO shape above):
///   for frame {
///       mic_in[frame]  <-- fe_capture()            (Op)
///       bt_in[frame]   <-- rf_receive()            (Op)
///       bt_out[frame]  <-- denoise(mic_in[frame])  (Op)
///       rf_transmit(bt_out[frame])                 (Op, effect)
///       mixed[frame]   <-- mix2(mic_in, bt_in)     (Op)
///       spk_out[frame] <-- denoise(mixed[frame])   (Op)
///       fe_emit(spk_out[frame])                    (Op, effect)
///   }
/// -> 7 operations total, ALL inside 1 Repeat, max depth 1. (The naive
/// shape also has 7 ops / 1 repeat, but split 4 top-level + 3 in the
/// repeat; here all 7 are per-frame, so all 7 live in the repeat.)
#[test]
fn acfg_example_14_embedded_multimcu() {
    let linked = linked_from_paths(
        "14-hearing-aid/prog.embedded.algo.nuc",
        "14-hearing-aid/schedules/embedded_multimcu.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    assert_eq!(acfg.operation_count(), 7);
    assert_eq!(acfg.repeat_count(), 1);
    assert_eq!(acfg.max_repeat_depth(), 1);
}

#[test]
fn acfg_example_7_naive() {
    // TASK-0032: matmul, naive schedule. Top level:
    //   a <-- load_a();
    //   b <-- load_b();
    //   for i { for j { for k { c[i][j] <-- madd(...); } } }
    //   save_c(c);
    // -> 2 top-level Operations + 1-in-triple-loop + 1 = 4 Operations
    // total, 3 Repeats, max depth 3.
    let linked = linked_from_paths(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    assert!(
        matches!(acfg.root, ACFGNode::Sequence(_)),
        "top-level must be Sequence, got {:?}",
        acfg.root
    );

    assert_eq!(
        acfg.operation_count(),
        4,
        "ops = load_a + load_b + madd(in innermost loop) + save_c"
    );
    assert_eq!(acfg.repeat_count(), 3, "three nested for-loops");
    assert_eq!(acfg.max_repeat_depth(), 3, "i / j / k nested");

    assert!(acfg.name_kernels.contains_key("madd"));
    assert!(acfg.name_kernels.contains_key("load_a"));
    assert!(acfg.name_kernels.contains_key("load_b"));
    assert!(acfg.name_kernels.contains_key("save_c"));
    for v in ["i", "j", "k"] {
        assert!(
            acfg.name_iter_vars.contains_key(v),
            "missing iter var `{}`",
            v
        );
    }
}

// --------------------------------------------------------------------
// Determinism: rebuilding the ACFG yields the same value
// --------------------------------------------------------------------

#[test]
fn acfg_is_deterministic_example_1() {
    let linked = linked_from_paths(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let a = build_acfg(&linked).expect("build_acfg");
    let b = build_acfg(&linked).expect("build_acfg");
    assert_eq!(a, b, "ACFG construction must be deterministic");
}

#[test]
fn acfg_is_deterministic_example_13_batch_parallel() {
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    );
    let a = build_acfg(&linked).expect("build_acfg");
    let b = build_acfg(&linked).expect("build_acfg");
    assert_eq!(a, b, "ACFG construction must be deterministic");
}

// --------------------------------------------------------------------
// Range carried by Repeat is resolved correctly
// --------------------------------------------------------------------

#[test]
fn acfg_loop_range_resolved_to_consts() {
    // Example 1: N=256, loop bounds 0..N -> 0..256.
    let linked = linked_from_paths(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");
    let mut ranges: Vec<std::ops::Range<i64>> = Vec::new();
    collect_ranges(&acfg.root, &mut ranges);
    assert_eq!(ranges, vec![0..256]);
}

#[test]
fn acfg_loop_range_resolved_for_cnn() {
    // CNN: B=16, loop bounds 0..B -> 0..16.
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");
    let mut ranges = Vec::new();
    collect_ranges(&acfg.root, &mut ranges);
    assert_eq!(ranges, vec![0..16]);
}

// --------------------------------------------------------------------
// Tree-walking helpers used by the assertions above
// --------------------------------------------------------------------

fn visit_operations(node: &ACFGNode, f: &mut impl FnMut(&nucleus_compiler::acfg::Operation)) {
    match node {
        ACFGNode::Operation(op) => f(op),
        ACFGNode::Repeat { body, .. } => visit_operations(body, f),
        ACFGNode::Sequence(children) => {
            for c in children {
                visit_operations(c, f);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

// --------------------------------------------------------------------
// TASK-0150 — per-firing index expressions on DataflowEdge.
// --------------------------------------------------------------------

/// `ident - intlit` shaped IrExpr, the common stencil offset form.
fn ident_minus(name: &str, k: i64) -> IrExpr {
    IrExpr::BinOp(
        IrBinOp::Sub,
        Box::new(IrExpr::Ident(name.to_string())),
        Box::new(IrExpr::IntLit(k)),
    )
}

/// `ident + intlit`.
fn ident_plus(name: &str, k: i64) -> IrExpr {
    IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(IrExpr::Ident(name.to_string())),
        Box::new(IrExpr::IntLit(k)),
    )
}

/// The 3x3 stencil `blur3` firing must record all nine `img_in`
/// reads, in argument order, each with its own two-axis index
/// expression list — including the duplicate symbol (`img_in`
/// appears nine times). The LHS `img_out[y][x]` write must be
/// captured in `data_out_access`.
#[test]
fn dataflow_edge_carries_stencil_index_expressions() {
    let linked = linked_from_paths(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    let mut found_blur = false;
    visit_operations(&acfg.root, &mut |op| {
        let edge = op
            .dataflow
            .edges
            .first()
            .expect("operation has at least one edge");
        // The blur3 firing is the one with nine inputs.
        if edge.data_in.len() != 9 {
            // single-assignment invariant: data_in mirrors access.
            assert_eq!(
                edge.data_in.len(),
                edge.data_in_access.len(),
                "data_in and data_in_access must be the same length"
            );
            for (id, acc) in edge.data_in.iter().zip(&edge.data_in_access) {
                assert_eq!(*id, acc.data, "data_in[i] must match data_in_access[i]");
            }
            // data_out_access mirrors data_out.
            assert_eq!(
                edge.data_out,
                edge.data_out_access.as_ref().map(|a| a.data),
                "data_out_access.data must mirror data_out"
            );
            return;
        }
        found_blur = true;

        // All nine reads name the same DataId (img_in) and the
        // symbol-only view (data_in) is exactly that DataId nine
        // times — proves data_in is derived from the access list.
        let img_in = edge.data_in[0];
        assert!(
            edge.data_in.iter().all(|d| *d == img_in),
            "all nine stencil reads are img_in"
        );
        assert_eq!(edge.data_in_access.len(), 9);
        assert!(
            edge.data_in_access.iter().all(|a| a.data == img_in),
            "access list agrees on the symbol"
        );

        // Argument order is preserved: the source reads, in order,
        //   img_in[y-1][x-1], img_in[y-1][x], img_in[y-1][x+1],
        //   img_in[y  ][x-1], img_in[y  ][x], img_in[y  ][x+1],
        //   img_in[y+1][x-1], img_in[y+1][x], img_in[y+1][x+1]
        let y = "y";
        let x = "x";
        let expect: Vec<Vec<IrExpr>> = vec![
            vec![ident_minus(y, 1), ident_minus(x, 1)],
            vec![ident_minus(y, 1), IrExpr::Ident(x.into())],
            vec![ident_minus(y, 1), ident_plus(x, 1)],
            vec![IrExpr::Ident(y.into()), ident_minus(x, 1)],
            vec![IrExpr::Ident(y.into()), IrExpr::Ident(x.into())],
            vec![IrExpr::Ident(y.into()), ident_plus(x, 1)],
            vec![ident_plus(y, 1), ident_minus(x, 1)],
            vec![ident_plus(y, 1), IrExpr::Ident(x.into())],
            vec![ident_plus(y, 1), ident_plus(x, 1)],
        ];
        let got: Vec<Vec<IrExpr>> = edge
            .data_in_access
            .iter()
            .map(|a| a.indices.clone())
            .collect();
        assert_eq!(
            got, expect,
            "stencil index expressions must survive verbatim, in argument order, \
             with duplicates kept"
        );

        // Output write: img_out[y][x].
        let out = edge
            .data_out_access
            .as_ref()
            .expect("dataflow statement has an output access");
        assert_eq!(Some(out.data), edge.data_out);
        assert_eq!(
            out.indices,
            vec![IrExpr::Ident(y.into()), IrExpr::Ident(x.into())],
            "LHS index expressions captured"
        );
    });
    assert!(found_blur, "expected to see the nine-input blur3 firing");
}

/// An effect statement (`save_image(img_out)`) reads a whole array
/// with no indices; the access must still be recorded, with an
/// empty `indices` list and `data_out_access == None`.
#[test]
fn effect_statement_records_whole_array_read_with_no_indices() {
    let linked = linked_from_paths(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");

    // The save_image firing: one input, no output. (load_image has
    // zero inputs and an output; blur3 has nine inputs.)
    let mut saw_save = false;
    visit_operations(&acfg.root, &mut |op| {
        let edge = op.dataflow.edges.first().expect("edge");
        if edge.data_in.len() == 1 && edge.data_out.is_none() {
            saw_save = true;
            assert_eq!(edge.data_in_access.len(), 1);
            assert_eq!(edge.data_in[0], edge.data_in_access[0].data);
            assert!(
                edge.data_in_access[0].indices.is_empty(),
                "whole-array read carries no index expressions"
            );
            assert!(edge.data_out_access.is_none());
        }
    });
    assert!(saw_save, "expected to see the save_image effect firing");
}

/// Global structural invariant across every example/schedule used in
/// the bit-identical e2e set: `data_in`/`data_in_access` stay aligned
/// and `data_out_access` mirrors `data_out`. This pins the
/// single-source-of-truth contract (data_in derived from access).
#[test]
fn access_lists_mirror_bare_lists_for_all_e2e_examples() {
    for (algo, sched) in [
        (
            "01-elementwise-add/prog.algo.nuc",
            "01-elementwise-add/schedules/naive.sched.nuc",
        ),
        (
            "02-split-add/prog.algo.nuc",
            "02-split-add/schedules/split.sched.nuc",
        ),
        (
            "03-reduction/prog.algo.nuc",
            "03-reduction/schedules/naive.sched.nuc",
        ),
        (
            "05-stencil/prog.algo.nuc",
            "05-stencil/schedules/naive.sched.nuc",
        ),
        (
            "07-matmul/prog.algo.nuc",
            "07-matmul/schedules/naive.sched.nuc",
        ),
    ] {
        let linked = linked_from_paths(algo, sched);
        let acfg = build_acfg(&linked).expect("build_acfg");
        visit_operations(&acfg.root, &mut |op| {
            for edge in &op.dataflow.edges {
                assert_eq!(
                    edge.data_in.len(),
                    edge.data_in_access.len(),
                    "{algo}: data_in / data_in_access length mismatch"
                );
                for (id, acc) in edge.data_in.iter().zip(&edge.data_in_access) {
                    assert_eq!(*id, acc.data, "{algo}: per-index symbol mismatch");
                }
                assert_eq!(
                    edge.data_out,
                    edge.data_out_access.as_ref().map(|a| a.data),
                    "{algo}: data_out / data_out_access mismatch"
                );
            }
        });
    }
}

fn collect_ranges(node: &ACFGNode, out: &mut Vec<std::ops::Range<i64>>) {
    match node {
        ACFGNode::Repeat { range, body, .. } => {
            out.push(range.clone());
            collect_ranges(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_ranges(c, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

// --------------------------------------------------------------------
// Negative: a non-const loop bound is a TYPED error, not a panic
// (TASK-0179). Mirrors TASK-0170's
// `sidecar_same_name_loop_differing_bounds_is_typed_error_not_panic`:
// pin the recurring "panic! on representable user input" defect class
// shut at this site.
//
// The witness is a *triangular* loop `for j : 0 .. i { ... }`. The
// algorithm grammar admits an enclosing iteration variable in a loop
// bound (`algo::lower::lower_index_expr` -> `resolve_ident` resolves
// in-scope iter vars), so this program parses, lowers AND links
// cleanly — it is valid, representable user input. `build_acfg`'s
// `eval_const` only resolves declared `const`s, so the bound `i`
// cannot be folded to a concrete `Range<i64>`. Before TASK-0179 that
// `panic!`ed; it must now return `BuildAcfgError::NonConstLoopBound`,
// which the driver surfaces as a clean diagnostic.
// --------------------------------------------------------------------

fn linked_from_inline_src(algo_src: &str, sched_src: &str) -> nucleus_compiler::LinkedIR {
    let algo = lower_algo(&parse_algo(algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(sched_src).expect("sched parse")).expect("sched lower");
    link(algo, sched).expect("link must succeed")
}

#[test]
fn build_acfg_non_const_loop_bound_is_typed_error_not_panic() {
    use nucleus_compiler::acfg::{BuildAcfgError, LoopBoundEnd};

    // A triangular nested loop: the inner bound `i` is the *outer*
    // loop variable, not a declared const. Single-assignment holds
    // (one write per data symbol), so this is accepted end-to-end by
    // parse/lower/link — exactly the "representable, otherwise-valid
    // input" the recurring-defect-class memo flags.
    let algo = r#"
const N : usize = 8;

data a : i32[N];
data b : i32[N];

kernel load_input  : ()    -> i32[N] effectful;
kernel id          : (i32) -> i32    pure;
kernel save_output : (i32[N]) -> ()  effectful;

a <-- load_input();

for i : 0 .. N {
    for j : 0 .. i {
        b[j] <-- id(a[j]);
    }
}

save_output(b);
"#;
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place id          on host;
    place save_output on host;
}
"#;

    let linked = linked_from_inline_src(algo, sched);

    // The contract: a typed error, NOT a panic (this call would
    // `panic!` before TASK-0179).
    let err = build_acfg(&linked).expect_err("non-const loop bound must be a typed error");

    match &err {
        BuildAcfgError::NonConstLoopBound { var, end, expr } => {
            // The offending loop is the inner `j`, whose upper bound
            // is the non-const expression `i`.
            assert_eq!(var, "j", "offending loop variable");
            assert_eq!(*end, LoopBoundEnd::Upper, "upper bound `i` is non-const");
            assert_eq!(
                *expr,
                IrExpr::Ident("i".into()),
                "offending expr is the enclosing iter var `i`, carried verbatim"
            );
        }
        // TASK-0398: a genuinely non-const bound must route to
        // NonConstLoopBound, NOT the new OverflowingLoopBound.
        other => panic!("expected NonConstLoopBound, got {other:?}"),
    }

    // The Display string carries an actionable, source-free
    // diagnostic (fail-fast AND verbose).
    let msg = err.to_string();
    assert!(msg.contains("loop `j`"), "msg: {msg}");
    assert!(msg.contains("non-constant"), "msg: {msg}");
    assert!(msg.contains("compile-time"), "msg: {msg}");
}

// --------------------------------------------------------------------
// TASK-0398: an OVERFLOWING constant loop bound is a DISTINCT diagnostic
// from a non-const one. Before TASK-0398, build.rs's `eval_const`
// returned `Option<i64>` and collapsed "not a constant" with "constant
// that overflows i64 / divides by zero", so an overflowing *constant*
// bound was mis-reported as `NonConstLoopBound` ("use a constant bound"
// — which the user already did). These pin the split: overflow and
// div-by-zero now surface as `OverflowingLoopBound`, while the genuine
// non-const case above still surfaces as `NonConstLoopBound`.
// --------------------------------------------------------------------

/// Inline algo with a single `for j` loop whose upper bound is the given
/// constant expression, plus a host-only schedule. Mirrors the
/// non-const-bound test's shape so the only variable is the bound expr.
fn linked_with_loop_bound(bound_src: &str) -> nucleus_compiler::LinkedIR {
    let algo = format!(
        r#"
const N : usize = 8;
data a : i32[N];
data b : i32[N];
kernel load_input  : ()    -> i32[N] effectful;
kernel id          : (i32) -> i32    pure;
kernel save_output : (i32[N]) -> ()  effectful;
a <-- load_input();
for j : 0 .. {bound_src} {{
    b[j] <-- id(a[j]);
}}
save_output(b);
"#
    );
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place id          on host;
    place save_output on host;
}
"#;
    linked_from_inline_src(&algo, sched)
}

#[test]
fn build_acfg_overflowing_const_loop_bound_is_overflow_error_not_nonconst() {
    use nucleus_compiler::acfg::{build_acfg, BuildAcfgError, LoopBoundEnd};

    // `-(0 - i64::MAX - 1)` is a pure constant expression that evaluates
    // to i64::MIN, whose negation overflows. It IS a constant — so it
    // must NOT be reported as a non-const bound.
    let linked = linked_with_loop_bound("-(0 - 9223372036854775807 - 1)");
    let err = build_acfg(&linked).expect_err("overflowing const bound must be a typed error");

    match &err {
        BuildAcfgError::OverflowingLoopBound {
            var, end, detail, ..
        } => {
            assert_eq!(var, "j", "offending loop variable");
            assert_eq!(*end, LoopBoundEnd::Upper);
            assert!(
                detail.contains("overflow"),
                "detail should name the overflow: {detail}"
            );
        }
        other => panic!("expected OverflowingLoopBound, got {other:?}"),
    }

    // The Display must NOT mis-advise "non-constant" — it must say the
    // bound is a constant that overflows.
    let msg = err.to_string();
    assert!(msg.contains("loop `j`"), "msg: {msg}");
    assert!(
        msg.contains("constant expression but cannot be folded"),
        "msg must frame it as a foldable-failure, not non-const: {msg}"
    );
    assert!(!msg.contains("non-constant"), "must not mis-advise: {msg}");
}

#[test]
fn build_acfg_divide_by_zero_const_loop_bound_is_overflow_error() {
    use nucleus_compiler::acfg::{build_acfg, BuildAcfgError};

    // `8 / 0` is a constant expression with a division by zero — also a
    // fold failure, not a non-const bound.
    let linked = linked_with_loop_bound("8 / 0");
    let err = build_acfg(&linked).expect_err("div-by-zero const bound must be a typed error");

    match &err {
        BuildAcfgError::OverflowingLoopBound { var, detail, .. } => {
            assert_eq!(var, "j");
            assert!(
                detail.contains("division by zero"),
                "detail should name div-by-zero: {detail}"
            );
        }
        other => panic!("expected OverflowingLoopBound, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// TASK-0360: kernel-less dataflow RHS is a LOUD typed error, not a
// silent ACFG drop. (Design-slice decision: option (c) — keep the
// explicit-kernel surface, decline the kernel-optional refactor, but
// make the unsupported bare-LValue form fail loud.)
// --------------------------------------------------------------------

/// A SAME-WORKER bare-LValue identity copy `c <-- a` (no kernel). Every
/// kernel is on `host`, so link's `MissingCrossWorkerTransfer`
/// (cross-worker only) does NOT fire — pre-TASK-0360 this compiled to a
/// silent drop (`build_acfg` returned Ok with the copy missing, so `c`
/// stayed at its allocation default — a silent wrong answer). The guard
/// must now reject it with `KernelLessDataflowRhs`.
#[test]
fn build_acfg_same_worker_bare_lvalue_copy_is_loud_error() {
    use nucleus_compiler::acfg::{build_acfg, BuildAcfgError};

    let algo = r#"
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()       -> i32[N] effectful;
kernel save_output : (i32[N]) -> ()     effectful;
a <-- load_input();
c <-- a;
save_output(c);
"#;
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place save_output on host;
}
"#;
    let linked = linked_from_inline_src(algo, sched);
    let err = build_acfg(&linked).expect_err("bare-LValue copy must be a loud typed error");

    match &err {
        BuildAcfgError::KernelLessDataflowRhs { lhs, .. } => {
            assert_eq!(lhs, "c", "the offending LHS data symbol");
        }
        other => panic!("expected KernelLessDataflowRhs, got {other:?}"),
    }

    // The diagnostic must point at the explicit-kernel workaround so the
    // user can act on it without reading the source.
    let msg = err.to_string();
    assert!(msg.contains("`c <--"), "msg names the offending stmt: {msg}");
    assert!(
        msg.contains("must go through a kernel"),
        "msg explains the v2 kernel requirement: {msg}"
    );
    assert!(
        msg.contains("identity passthrough") && msg.contains("xpose"),
        "msg points at the explicit-kernel workaround: {msg}"
    );
}

/// An arithmetic (kernel-less) RHS `c[i] <-- a[i] + a[i]` hits the same
/// guard: the rule is "dataflow RHS must be a kernel call", not
/// specifically "identity copy".
#[test]
fn build_acfg_arithmetic_kernel_less_rhs_is_loud_error() {
    use nucleus_compiler::acfg::{build_acfg, BuildAcfgError};

    let algo = r#"
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()       -> i32[N] effectful;
kernel save_output : (i32[N]) -> ()     effectful;
a <-- load_input();
for i : 0 .. N {
    c[i] <-- a[i] + a[i];
}
save_output(c);
"#;
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place save_output on host;
}
"#;
    let linked = linked_from_inline_src(algo, sched);
    let err = build_acfg(&linked).expect_err("arithmetic kernel-less RHS must be a loud error");
    assert!(
        matches!(&err, BuildAcfgError::KernelLessDataflowRhs { lhs, .. } if lhs == "c"),
        "expected KernelLessDataflowRhs for `c`, got {err:?}"
    );
}

/// Positive control: the canonical kernel-call form still builds an
/// Operation (the guard does not over-reach). `c[i] <-- id(a[i])` is a
/// Call RHS — exactly the 15-transpose `xpose` workaround shape.
#[test]
fn build_acfg_kernel_call_dataflow_still_builds_operation() {
    use nucleus_compiler::acfg::build_acfg;

    let algo = r#"
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()       -> i32[N] effectful;
kernel id          : (i32)    -> i32    pure;
kernel save_output : (i32[N]) -> ()     effectful;
a <-- load_input();
for i : 0 .. N {
    c[i] <-- id(a[i]);
}
save_output(c);
"#;
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place id          on host;
    place save_output on host;
}
"#;
    let linked = linked_from_inline_src(algo, sched);
    let acfg = build_acfg(&linked).expect("kernel-call dataflow must build");
    // load_input + id(in loop) + save_output = 3 operations.
    assert_eq!(acfg.operation_count(), 3, "load + id + save");
}

// --------------------------------------------------------------------
// TASK-0341.02.01.05.01 / epic S4: a `for … until COND { … }` bounded
// early-exit loop is now NON-INERT — it LOWERS to an `ACFGNode::Repeat`
// over the compile-time cap range carrying the halt predicate in
// `break_cond: Some(IrExpr::Compare(...))`. This SUPERSEDES the epic-S1
// inert behaviour (which rejected the loop at build_acfg with
// `BuildAcfgError::UntilLoopUnsupported`); the S1 reject was the opacity
// gate this slice LIFTS (architect S1 fold-forward item 4 — do not leave
// it rejecting a now-supported shape). The runtime break EMIT is still
// deferred (TASK-0341.02.01.05.04); this asserts the codegen-free
// lowering only.
// --------------------------------------------------------------------

#[test]
fn build_acfg_for_until_compare_cond_lowers_to_repeat_with_break_cond() {
    use nucleus_compiler::acfg::ACFGNode;
    use nucleus_compiler::algo::{IrCmpOp, IrExpr};

    // A capped early-exit loop. The COND (`a[0] <= a[0]`) is a bool
    // comparison over a pre-existing data symbol — decoupled from any
    // reduction (epic S3), exercising the loop-control surface alone.
    let algo = r#"
const N : usize = 8;

data a : i32[N];
data b : i32[N];

kernel load_input  : ()    -> i32[N] effectful;
kernel id          : (i32) -> i32    pure;
kernel save_output : (i32[N]) -> ()  effectful;

a <-- load_input();

for i : 0 .. N until a[0] <= a[0] {
    b[i] <-- id(a[i]);
}

save_output(b);
"#;
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place id          on host;
    place save_output on host;
}
"#;

    let linked = linked_from_inline_src(algo, sched);

    // S4: build_acfg now SUCCEEDS — the until-loop lowers to a capped
    // Repeat carrying the break predicate.
    let acfg = build_acfg(&linked).expect("a Compare-COND until-loop must lower cleanly");

    // Find the lone Repeat in the root and inspect its `break_cond`.
    fn find_repeat(node: &ACFGNode) -> Option<&ACFGNode> {
        match node {
            ACFGNode::Repeat { .. } => Some(node),
            ACFGNode::Sequence(cs) => cs.iter().find_map(find_repeat),
            _ => None,
        }
    }
    let repeat = find_repeat(&acfg.root).expect("the for..until lowered to a Repeat");
    let ACFGNode::Repeat {
        range, break_cond, ..
    } = repeat
    else {
        unreachable!("find_repeat returns a Repeat");
    };

    // Cap N kept: the Repeat iterates the full compile-time range 0..N.
    assert_eq!(*range, 0..8, "the compile-time cap N=8 is the Repeat range");

    // The halt predicate survives to the ACFG layer as a bool Compare.
    match break_cond {
        Some(IrExpr::Compare(IrCmpOp::Le, _, _)) => {}
        other => panic!("expected break_cond = Some(Compare(Le, ..)), got {other:?}"),
    }
}

// TASK-0341.02.01.05.01 / epic S4: bool-context gate. The `until COND`
// must be a relational comparison (`IrExpr::Compare`, the only
// bool-valued IrExpr in v2). A non-Compare COND — here a plain data read
// `a[0]` (i32, not bool) — is rejected with the typed
// `BuildAcfgError::UntilCondNotComparison` (NOT a panic, NOT a silent
// accept). This closes the gap S1 left: `lower_rvalue` accepts a plain
// int rvalue as COND with no bool gate; S4 adds the bool-context check at
// the ACFG boundary.
// --------------------------------------------------------------------

#[test]
fn build_acfg_for_until_non_compare_cond_is_typed_error() {
    use nucleus_compiler::acfg::BuildAcfgError;

    // The COND `a[0]` is a plain i32 data read — NOT a bool comparison.
    let algo = r#"
const N : usize = 8;

data a : i32[N];
data b : i32[N];

kernel load_input  : ()    -> i32[N] effectful;
kernel id          : (i32) -> i32    pure;
kernel save_output : (i32[N]) -> ()  effectful;

a <-- load_input();

for i : 0 .. N until a[0] {
    b[i] <-- id(a[i]);
}

save_output(b);
"#;
    let sched = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place id          on host;
    place save_output on host;
}
"#;

    let linked = linked_from_inline_src(algo, sched);

    let err =
        build_acfg(&linked).expect_err("a non-Compare until-COND must be a typed error, not Ok");

    match &err {
        BuildAcfgError::UntilCondNotComparison { var, .. } => {
            assert_eq!(var, "i", "offending loop variable is `i`");
        }
        other => panic!("expected UntilCondNotComparison, got {other:?}"),
    }

    // The Display string is actionable + source-free.
    let msg = err.to_string();
    assert!(msg.contains("loop `i`"), "msg: {msg}");
    assert!(msg.contains("relational comparison"), "msg: {msg}");
}
