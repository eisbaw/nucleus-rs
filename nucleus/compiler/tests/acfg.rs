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
//! Excluded examples mirror `tests/link.rs`:
//! - `14-hearing-aid/embedded_multimcu.sched.nuc` (TASK-0079: parse
//!   failure).
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

use compiler::acfg::{build_acfg, ACFGNode};
use compiler::algo::{lower_algo, parse_algo};
use compiler::link;
use compiler::sched::{lower_sched, parse_sched};

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

fn linked_from_paths(algo_rel: &str, sched_rel: &str) -> compiler::LinkedIR {
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
    let acfg = build_acfg(&linked);

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
    let acfg = build_acfg(&linked);

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
    let acfg = build_acfg(&linked);

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
    let acfg = build_acfg(&linked);

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
    // Hearing-aid, naive (everything on host). One top-level for-loop
    // with six statements inside:
    //   mic_in <-- fe_capture()
    //   bt_in  <-- rf_receive()
    //   bt_out <-- denoise(mic_in)
    //   rf_transmit(bt_out)
    //   spk_out <-- denoise(mix2(...))
    //   fe_emit(spk_out)
    // -> 6 operations, all inside one Repeat.
    let linked = linked_from_paths(
        "14-hearing-aid/prog.algo.nuc",
        "14-hearing-aid/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked);

    assert_eq!(acfg.operation_count(), 6);
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
    let acfg = build_acfg(&linked);

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
    let a = build_acfg(&linked);
    let b = build_acfg(&linked);
    assert_eq!(a, b, "ACFG construction must be deterministic");
}

#[test]
fn acfg_is_deterministic_example_13_batch_parallel() {
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    );
    let a = build_acfg(&linked);
    let b = build_acfg(&linked);
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
    let acfg = build_acfg(&linked);
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
    let acfg = build_acfg(&linked);
    let mut ranges = Vec::new();
    collect_ranges(&acfg.root, &mut ranges);
    assert_eq!(ranges, vec![0..16]);
}

// --------------------------------------------------------------------
// Tree-walking helpers used by the assertions above
// --------------------------------------------------------------------

fn visit_operations(node: &ACFGNode, f: &mut impl FnMut(&compiler::acfg::Operation)) {
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
