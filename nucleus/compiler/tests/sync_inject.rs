//! Integration tests for the sync-injection pass (TASK-0017).
//!
//! Strategy:
//!
//! - **Synthetic positive cases**: hand-built tiny ACFGs that
//!   exercise each rule from `passes::sync_inject` in isolation:
//!     1. Sequence boundary with cross-worker write -> read.
//!     2. Repeat entry from a different worker.
//!     3. Repeat exit with cross-worker writes inside.
//!     4. Single-worker elision.
//!
//!   Synthetic ACFGs are used because the real examples are either
//!   single-worker (example 1, 13-naive, 14-naive) or do not have
//!   the precise cross-worker pattern each rule targets in
//!   isolation. The synthetic cases let us test rules independently
//!   without depending on which example file happens to trigger
//!   them.
//!
//! - **Negative-of-positive**: a single-worker ACFG should produce
//!   zero syncs no matter what its structure is.
//!
//! - **Idempotence**: calling `inject_syncs` twice yields the same
//!   ACFG as calling it once. This is asserted both on the
//!   synthetic cases and on the real examples.
//!
//! - **End-to-end against real examples**: build the ACFG for each
//!   schedule of examples 1, 13, 14 and assert structural
//!   properties about sync count and participant sets, then assert
//!   idempotence.
//!
//! What this file does NOT test:
//! - Snapshot of the full tree. Like `tests/acfg.rs`, structural
//!   assertions are preferred over full-tree snapshots.
//! - Negative paths inside `inject_syncs` — the pass is total and
//!   has no error returns.

use std::collections::BTreeSet;
use std::ops::Range;

use compiler::acfg::{build_acfg, ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
use compiler::algo::{lower_algo, parse_algo};
use compiler::event::{DataId, IterVar, KernelId, WorkerId};
use compiler::link;
use compiler::passes::sync_inject::inject_syncs;
use compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Synthetic-ACFG helpers
// --------------------------------------------------------------------

fn ws(ids: &[u64]) -> BTreeSet<WorkerId> {
    ids.iter().copied().map(WorkerId).collect()
}

/// Build a synthetic Operation with the given worker set, kernel id,
/// optional data_in and data_out. data_in being non-empty makes this
/// op a *reader* under the sync-injection rules; data_out being
/// `Some` makes it a *writer*.
fn op(workers: &[u64], kernel: u64, data_in: Vec<u64>, data_out: Option<u64>) -> ACFGNode {
    let kid = KernelId(kernel);
    ACFGNode::Operation(Operation {
        kernel: kid,
        workers: ws(workers),
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(
                data_in.into_iter().map(DataId).collect(),
                kid,
                data_out.map(DataId),
            )],
        },
    })
}

fn empty_acfg(root: ACFGNode) -> ACFG {
    ACFG {
        root,
        name_kernels: Default::default(),
        name_data: Default::default(),
        name_workers: Default::default(),
        name_iter_vars: Default::default(),
        inner_block_iter_vars: Default::default(),
    }
}

fn repeat(body: Vec<ACFGNode>) -> ACFGNode {
    ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: Range { start: 0, end: 1 },
        body: Box::new(ACFGNode::Sequence(body)),
        block_tag: None,
    }
}

/// Walk all Sync nodes in the tree and collect their participant sets.
fn collect_sync_participants(node: &ACFGNode, out: &mut Vec<BTreeSet<WorkerId>>) {
    match node {
        ACFGNode::Sync(s) => out.push(s.participants.clone()),
        ACFGNode::Repeat { body, .. } => collect_sync_participants(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_sync_participants(c, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Xfer(_) => {}
    }
}

// --------------------------------------------------------------------
// Rule 1: Sequence boundary — writer on W1, reader on W2, W1 != W2
// --------------------------------------------------------------------

#[test]
fn sequence_boundary_injects_sync_between_cross_worker_writer_reader() {
    // op_w0 writes data 0, op_w1 reads data 0. Different workers.
    // Expect ONE sync between them with participants {w0, w1}.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),  // writer on w0
        op(&[1], 101, vec![0], Some(1)), // reader on w1
    ]);
    let result = inject_syncs(empty_acfg(root));

    assert_eq!(
        result.sync_count(),
        1,
        "exactly one sync between the two ops"
    );

    let mut parts = Vec::new();
    collect_sync_participants(&result.root, &mut parts);
    assert_eq!(parts, vec![ws(&[0, 1])]);

    // Structure: Sequence(op, sync, op).
    if let ACFGNode::Sequence(children) = &result.root {
        assert_eq!(children.len(), 3);
        assert!(matches!(children[0], ACFGNode::Operation(_)));
        assert!(matches!(children[1], ACFGNode::Sync(_)));
        assert!(matches!(children[2], ACFGNode::Operation(_)));
    } else {
        panic!("expected top-level Sequence");
    }
}

#[test]
fn sequence_boundary_same_worker_injects_nothing() {
    // Both ops on worker 0 -> no sync (rule wants W1 != W2).
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[0], 101, vec![0], Some(1)),
    ]);
    let result = inject_syncs(empty_acfg(root));
    assert_eq!(result.sync_count(), 0);
}

#[test]
fn sequence_boundary_writer_without_reader_injects_nothing() {
    // First op writes on w0; second op writes on w1 but does NOT
    // read anything (no data_in). The rule needs a reader on the
    // second side; absent that, no sync.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![], Some(1)),
    ]);
    let result = inject_syncs(empty_acfg(root));
    assert_eq!(result.sync_count(), 0);
}

#[test]
fn sequence_boundary_three_ops_two_syncs() {
    // w0 writer -> w1 reader -> w2 reader. Two boundaries, two
    // syncs.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![0], Some(1)),
        op(&[2], 102, vec![1], Some(2)),
    ]);
    let result = inject_syncs(empty_acfg(root));
    assert_eq!(result.sync_count(), 2);

    let mut parts = Vec::new();
    collect_sync_participants(&result.root, &mut parts);
    assert_eq!(parts, vec![ws(&[0, 1]), ws(&[1, 2])]);
}

// --------------------------------------------------------------------
// Rule 2: Repeat entry — body's workers differ from prior writer
// --------------------------------------------------------------------

#[test]
fn repeat_entry_injects_sync_when_workers_differ() {
    // Prior op writes on w0; Repeat body is one op on w1. Inject
    // sync at body entry with participants {w0, w1}.
    //
    // (The Sequence boundary rule may ALSO fire here because the
    // Repeat's reads/writes propagate up. That's expected
    // over-syncing — we assert both presences explicitly.)
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        repeat(vec![op(&[1], 101, vec![0], Some(1))]),
    ]);
    let result = inject_syncs(empty_acfg(root));

    // We expect 2 syncs: one outside the Repeat (Sequence rule
    // because the Repeat reads data 0 on w1 while w0 wrote it) and
    // one at the entry of the Repeat body (Repeat entry rule).
    assert_eq!(
        result.sync_count(),
        2,
        "one outer Sequence boundary sync + one Repeat-entry sync"
    );

    // The first sync (the outer one) lives directly in the top
    // Sequence; the second lives inside the Repeat's body Sequence.
    if let ACFGNode::Sequence(children) = &result.root {
        // Expect [Op, Sync, Repeat].
        assert_eq!(children.len(), 3);
        assert!(matches!(children[1], ACFGNode::Sync(_)));
        if let ACFGNode::Repeat { body, .. } = &children[2] {
            if let ACFGNode::Sequence(body_kids) = body.as_ref() {
                assert!(
                    matches!(body_kids.first(), Some(ACFGNode::Sync(_))),
                    "Repeat body must start with an entry Sync"
                );
            } else {
                panic!("Repeat body must be Sequence");
            }
        } else {
            panic!("expected Repeat at index 2");
        }
    } else {
        panic!("expected top-level Sequence");
    }
}

// --------------------------------------------------------------------
// Rule 3: Repeat exit — cross-worker writes inside the body
// --------------------------------------------------------------------

#[test]
fn repeat_exit_injects_sync_when_body_has_cross_worker_writes() {
    // Body has two ops, one on w0 writing data 0, the next on w1
    // writing data 1 (no data_in). The Sequence boundary rule
    // inside the body does NOT fire (second op is not a reader),
    // but the Repeat-exit rule DOES fire because writers span
    // {w0, w1}.
    let root = repeat(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![], Some(1)),
    ]);
    let result = inject_syncs(empty_acfg(root));

    // Expect exactly one Sync (the body-exit one). prior_writes at
    // the program root is empty, so no entry sync, and no
    // Sequence-boundary sync between the two body ops because the
    // second op has no data_in.
    assert_eq!(result.sync_count(), 1);

    let mut parts = Vec::new();
    collect_sync_participants(&result.root, &mut parts);
    assert_eq!(parts, vec![ws(&[0, 1])]);

    // Structure: Repeat( Sequence(op_w0, op_w1, Sync) ).
    if let ACFGNode::Repeat { body, .. } = &result.root {
        if let ACFGNode::Sequence(kids) = body.as_ref() {
            assert_eq!(kids.len(), 3);
            assert!(matches!(kids[2], ACFGNode::Sync(_)));
        } else {
            panic!("Repeat body must be Sequence");
        }
    } else {
        panic!("expected Repeat at root");
    }
}

#[test]
fn repeat_exit_no_sync_if_only_one_worker_writes_inside() {
    // Body has two ops both on w0. No cross-worker writes. No
    // sync should be injected at exit.
    let root = repeat(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[0], 101, vec![0], Some(1)),
    ]);
    let result = inject_syncs(empty_acfg(root));
    assert_eq!(result.sync_count(), 0);
}

// --------------------------------------------------------------------
// Rule 4: Elision — single-worker participant
// --------------------------------------------------------------------

#[test]
fn single_worker_acfg_produces_no_syncs() {
    // All ops on one worker, including inside a loop. No syncs
    // anywhere.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        repeat(vec![
            op(&[0], 101, vec![0], Some(1)),
            op(&[0], 102, vec![1], Some(2)),
        ]),
        op(&[0], 103, vec![2], None),
    ]);
    let result = inject_syncs(empty_acfg(root));
    assert_eq!(
        result.sync_count(),
        0,
        "single-worker programs never need syncs"
    );
}

// --------------------------------------------------------------------
// Idempotence
// --------------------------------------------------------------------

#[test]
fn idempotent_on_synthetic_sequence_case() {
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![0], Some(1)),
        op(&[2], 102, vec![1], Some(2)),
    ]);
    let once = inject_syncs(empty_acfg(root));
    let twice = inject_syncs(once.clone());
    assert_eq!(once, twice, "inject_syncs must be idempotent");
}

#[test]
fn idempotent_on_synthetic_repeat_case() {
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        repeat(vec![
            op(&[1], 101, vec![0], Some(1)),
            op(&[2], 102, vec![1], Some(2)),
        ]),
    ]);
    let once = inject_syncs(empty_acfg(root));
    let twice = inject_syncs(once.clone());
    assert_eq!(once, twice, "inject_syncs must be idempotent");
}

// --------------------------------------------------------------------
// End-to-end: real examples
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

fn acfg_with_syncs(algo_rel: &str, sched_rel: &str) -> ACFG {
    let linked = linked_from_paths(algo_rel, sched_rel);
    inject_syncs(build_acfg(&linked).expect("build_acfg"))
}

#[test]
fn example_1_naive_has_no_syncs() {
    // Single-worker schedule -> zero syncs.
    let acfg = acfg_with_syncs(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    assert_eq!(acfg.sync_count(), 0);
}

#[test]
fn example_13_naive_has_no_syncs() {
    // Naive schedule places everything on host -> zero syncs.
    let acfg = acfg_with_syncs(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/naive.sched.nuc",
    );
    assert_eq!(acfg.sync_count(), 0);
}

#[test]
fn example_13_batch_parallel_injects_cross_worker_syncs() {
    // host loads input, distributed workers {w0..w3} run the
    // batch-parallel CNN inside the loop, host saves output.
    // Expect:
    // - At least one Sync between `load_input` (host) and the
    //   Repeat (whose body reads/writes on {w0..w3}).
    // - A Repeat-entry Sync.
    // - A Sync between the Repeat (workers {w0..w3}) and
    //   `save_output` (host).
    let acfg = acfg_with_syncs(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    );
    assert!(
        acfg.sync_count() > 0,
        "batch-parallel schedule must inject syncs"
    );

    // All injected syncs in this schedule should mention at least
    // one of {host, w0..w3} and have >= 2 participants.
    let mut parts = Vec::new();
    collect_sync_participants(&acfg.root, &mut parts);
    for p in &parts {
        assert!(
            p.len() >= 2,
            "elision rule: sync participants must be >= 2; got {:?}",
            p
        );
    }

    // The host worker id and at least one compute worker id should
    // appear in the union of participants -- the sync injection
    // covers the host<->compute control-flow joins.
    let host_id = acfg.name_workers["host"];
    let all_participants: BTreeSet<WorkerId> = parts.iter().flatten().copied().collect();
    assert!(
        all_participants.contains(&host_id),
        "host worker must appear in at least one sync"
    );

    // Idempotence on the real example.
    let twice = inject_syncs(acfg.clone());
    assert_eq!(acfg, twice);
}

#[test]
fn example_13_pipeline_parallel_injects_cross_worker_syncs() {
    // host -> w_stage1 -> w_stage2 -> w_stage3 -> host. Every layer
    // is on a singleton worker; consecutive layers in the loop
    // body produce -> consume across workers, so the Sequence rule
    // fires inside the loop body multiple times.
    let acfg = acfg_with_syncs(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );
    assert!(
        acfg.sync_count() >= 2,
        "pipeline-parallel: at least the inner stage->stage hops produce syncs"
    );

    // All syncs have >=2 participants.
    let mut parts = Vec::new();
    collect_sync_participants(&acfg.root, &mut parts);
    for p in &parts {
        assert!(p.len() >= 2);
    }

    // Idempotence.
    let twice = inject_syncs(acfg.clone());
    assert_eq!(acfg, twice);
}

#[test]
fn example_14_naive_has_no_syncs() {
    // Everything on `host` -> no cross-worker writes/reads -> no
    // syncs.
    let acfg = acfg_with_syncs(
        "14-hearing-aid/prog.algo.nuc",
        "14-hearing-aid/schedules/naive.sched.nuc",
    );
    assert_eq!(acfg.sync_count(), 0);
}

#[test]
fn inject_syncs_preserves_name_tables_on_real_examples() {
    // Sanity: name_kernels/name_data/name_workers/name_iter_vars
    // must be forwarded unchanged. If a future refactor builds a
    // fresh ACFG inside the pass it would be wrong to drop these.
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    );
    let before = build_acfg(&linked).expect("build_acfg");
    let after = inject_syncs(before.clone());
    assert_eq!(before.name_kernels, after.name_kernels);
    assert_eq!(before.name_data, after.name_data);
    assert_eq!(before.name_workers, after.name_workers);
    assert_eq!(before.name_iter_vars, after.name_iter_vars);
}

#[test]
fn inject_syncs_preserves_operation_count_on_real_examples() {
    // Syncs are added but Operations are never removed. The
    // operation_count should be identical before/after.
    for (algo, sched) in [
        (
            "01-elementwise-add/prog.algo.nuc",
            "01-elementwise-add/schedules/naive.sched.nuc",
        ),
        (
            "13-cnn-inference/prog.algo.nuc",
            "13-cnn-inference/schedules/naive.sched.nuc",
        ),
        (
            "13-cnn-inference/prog.algo.nuc",
            "13-cnn-inference/schedules/batch_parallel.sched.nuc",
        ),
        (
            "13-cnn-inference/prog.algo.nuc",
            "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
        ),
        (
            "14-hearing-aid/prog.algo.nuc",
            "14-hearing-aid/schedules/naive.sched.nuc",
        ),
    ] {
        let linked = linked_from_paths(algo, sched);
        let before = build_acfg(&linked).expect("build_acfg");
        let after = inject_syncs(before.clone());
        assert_eq!(
            before.operation_count(),
            after.operation_count(),
            "operation count must be preserved by inject_syncs (algo={algo}, sched={sched})"
        );
        assert_eq!(
            before.repeat_count(),
            after.repeat_count(),
            "repeat count must be preserved (algo={algo}, sched={sched})"
        );
    }
}
