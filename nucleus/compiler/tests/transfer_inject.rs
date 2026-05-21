//! Integration tests for the transfer-injection pass (TASK-0018).
//!
//! Strategy:
//!
//! - **Synthetic positive cases**: hand-built tiny ACFGs paired with
//!   minimal `LinkedIR`s that exercise the core invariants (matched
//!   Push/Wait pair, unique SeqTag, fresh policy attached).
//!
//! - **Policy combinations**: four schedule snippets covering sync,
//!   async, async+buffer=2, async+buffer=2+notify=event. The
//!   resulting [`TransferPolicy`] on every `XferPlaceholder` must
//!   reflect the schedule.
//!
//! - **Idempotence**: calling `inject_transfers` twice yields the
//!   same ACFG as calling it once.
//!
//! - **End-to-end against real examples**: build the ACFG from
//!   example 1 (no cross-worker dataflow -> 0 xfers), example 13
//!   naive (no cross-worker -> 0), and example 14 naive (no
//!   cross-worker -> 0). Structural assertions only — no snapshots
//!   of the full tree.
//!
//! What this file does NOT test:
//! - Snapshot of the full tree — structural assertions are preferred.
//! - Capability mismatch errors — by task spec, capability checks are
//!   deferred to codegen-time (TASK-0019+).

use std::collections::{BTreeMap, BTreeSet};

use compiler::acfg::{
    build_acfg, ACFGNode, DataflowDag, DataflowEdge, NotifyMode, Operation, TransferPolicy,
    XferPlaceholder, XferRole, ACFG,
};
use compiler::algo::{lower_algo, parse_algo};
use compiler::event::{DataId, KernelId, SeqTag, WorkerId};
use compiler::link::{self, LinkedIR, WorkerEntity};
use compiler::passes::transfer_inject::inject_transfers;
use compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Synthetic-ACFG helpers
// --------------------------------------------------------------------

fn ws(ids: &[u64]) -> BTreeSet<WorkerId> {
    ids.iter().copied().map(WorkerId).collect()
}

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

/// Build a synthetic `LinkedIR` carrying just enough info for
/// `inject_transfers` to consult:
/// - `data_producers`: map data symbol name -> single-worker entity.
/// - `sched.transfers`: per-data transfer policy in textual schedule form.
///
/// Other LinkedIR fields are left empty/default — the transfer-inject
/// pass does not look at them.
fn synthetic_linked_ir(
    name_data: &BTreeMap<String, DataId>,
    name_workers: &BTreeMap<String, WorkerId>,
    producers: &[(&str, &[&str])],
    transfers_src: &str,
) -> LinkedIR {
    let _ = name_data;
    let _ = name_workers;

    let mut data_producers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    for (data_name, worker_names) in producers {
        let entity = WorkerEntity(worker_names.iter().map(|s| (*s).to_string()).collect());
        data_producers.insert((*data_name).to_string(), entity);
    }

    // We embed the transfer directives by parsing a minimal schedule
    // file that declares one worker and the transfers. Hand-writing
    // SchedIR fields is brittle; using the real parser keeps us
    // honest to the surface syntax.
    let sched_src = format!(
        r#"schedule for "../prog.algo.nuc" {{
    workers = {{ host }};
    {transfers_src}
}}"#
    );
    let sched_ast = parse_sched(&sched_src).expect("synthetic sched parses");
    let sched = lower_sched(&sched_ast).expect("synthetic sched lowers");

    LinkedIR {
        algo: Default::default(),
        sched,
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers,
        data_consumers: Default::default(),
    }
}

fn synthetic_acfg(
    root: ACFGNode,
    name_data_pairs: &[(&str, u64)],
    name_workers_pairs: &[(&str, u64)],
) -> ACFG {
    let name_data: BTreeMap<String, DataId> = name_data_pairs
        .iter()
        .map(|(n, i)| ((*n).to_string(), DataId(*i)))
        .collect();
    let name_workers: BTreeMap<String, WorkerId> = name_workers_pairs
        .iter()
        .map(|(n, i)| ((*n).to_string(), WorkerId(*i)))
        .collect();
    ACFG {
        root,
        name_kernels: Default::default(),
        name_data,
        name_workers,
        name_iter_vars: Default::default(),
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: Default::default(),
    }
}

// --------------------------------------------------------------------
// Core: two-worker producer/consumer
// --------------------------------------------------------------------

#[test]
fn two_worker_producer_consumer_yields_matched_push_wait() {
    // op_p on worker w0 produces data 0 (named "d").
    // op_c on worker w1 reads data 0.
    // Expect: ONE Push after op_p, ONE Wait before op_c, both
    // carrying the same SeqTag, src=w0, dst=w1, data=0, sync policy.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),  // producer on w0
        op(&[1], 101, vec![0], Some(1)), // consumer on w1
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["w0"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg);

    // Exactly one Push and one Wait.
    assert_eq!(result.push_count(), 1, "exactly one Push");
    assert_eq!(result.wait_count(), 1, "exactly one Wait");

    // Both share the same seq tag.
    let xfers = result.root.collect_xfers();
    assert_eq!(xfers.len(), 2);
    assert_eq!(xfers[0].seq, xfers[1].seq, "matched Push/Wait share seq");

    // Push appears before Wait in source order (producer Op comes
    // before consumer Op).
    assert_eq!(xfers[0].role, XferRole::Push);
    assert_eq!(xfers[1].role, XferRole::Wait);

    // Endpoints make sense.
    for x in &xfers {
        assert_eq!(x.src, WorkerId(0));
        assert_eq!(x.dst, WorkerId(1));
        assert_eq!(x.data, DataId(0));
    }

    // Sequence structure: [op_p, Push, op_c_wait_then_op] ->
    // [op_p, Push, Wait, op_c].
    if let ACFGNode::Sequence(children) = &result.root {
        assert_eq!(children.len(), 4);
        assert!(matches!(children[0], ACFGNode::Operation(_)));
        assert!(matches!(&children[1], ACFGNode::Xfer(x) if x.role == XferRole::Push));
        assert!(matches!(&children[2], ACFGNode::Xfer(x) if x.role == XferRole::Wait));
        assert!(matches!(children[3], ACFGNode::Operation(_)));
    } else {
        panic!("expected top-level Sequence");
    }
}

#[test]
fn same_worker_producer_consumer_yields_no_transfers() {
    // Both ops on worker 0 -> no cross-worker edge -> no transfer.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[0], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0)]);
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["w0"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg);
    assert_eq!(result.xfer_count(), 0);
}

#[test]
fn seq_tags_unique_per_pair() {
    // Three cross-worker edges in sequence; expect 3 Push and 3 Wait,
    // all six with distinct sequence values within {Push} and {Wait}
    // and the same seq within each matched pair.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),  // produces d0 on w0
        op(&[1], 101, vec![0], Some(1)), // reads d0 on w1; produces d1
        op(&[2], 102, vec![1], Some(2)), // reads d1 on w2; produces d2
        op(&[0], 103, vec![2], None),    // reads d2 on w0
    ]);
    let acfg = synthetic_acfg(
        root,
        &[("d0", 0), ("d1", 1), ("d2", 2)],
        &[("w0", 0), ("w1", 1), ("w2", 2)],
    );
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d0", &["w0"]), ("d1", &["w1"]), ("d2", &["w2"])],
        "transfer d0 : sync; transfer d1 : sync; transfer d2 : sync;",
    );
    let result = inject_transfers(&linked, acfg);

    assert_eq!(result.push_count(), 3);
    assert_eq!(result.wait_count(), 3);

    let xfers = result.root.collect_xfers();
    let seqs: Vec<SeqTag> = xfers.iter().map(|x| x.seq).collect();
    let unique: BTreeSet<SeqTag> = seqs.iter().copied().collect();
    // Three matched pairs -> 3 distinct seq values, each appearing
    // twice.
    assert_eq!(unique.len(), 3, "three distinct sequence tags");

    // Verify every Push has a matching Wait by seq.
    let pushes: BTreeSet<SeqTag> = xfers
        .iter()
        .filter(|x| x.role == XferRole::Push)
        .map(|x| x.seq)
        .collect();
    let waits: BTreeSet<SeqTag> = xfers
        .iter()
        .filter(|x| x.role == XferRole::Wait)
        .map(|x| x.seq)
        .collect();
    assert_eq!(pushes, waits, "Push and Wait seq sets match");
}

// --------------------------------------------------------------------
// Schedule policy combinations
// --------------------------------------------------------------------

fn policy_after_inject(transfers_src: &str) -> TransferPolicy {
    // Same two-op shape as the very first test; we just vary the
    // schedule's transfer directive and read the policy back off
    // the placeholders.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["w0"])],
        transfers_src,
    );
    let result = inject_transfers(&linked, acfg);

    let xfers = result.root.collect_xfers();
    assert!(!xfers.is_empty(), "expected at least one transfer");
    let p = xfers[0].policy;
    // All endpoints of a pair must agree on the policy.
    for x in &xfers {
        assert_eq!(x.policy, p);
    }
    p
}

#[test]
fn policy_sync() {
    let p = policy_after_inject("transfer d : sync;");
    assert_eq!(
        p,
        TransferPolicy {
            synchronous: true,
            buffer: 1,
            notify: NotifyMode::Default,
        }
    );
}

#[test]
fn policy_async() {
    let p = policy_after_inject("transfer d : async;");
    assert_eq!(
        p,
        TransferPolicy {
            synchronous: false,
            buffer: 1,
            notify: NotifyMode::Default,
        }
    );
}

#[test]
fn policy_async_buffer_2() {
    let p = policy_after_inject("transfer d : async, buffer=2;");
    assert_eq!(
        p,
        TransferPolicy {
            synchronous: false,
            buffer: 2,
            notify: NotifyMode::Default,
        }
    );
}

#[test]
fn policy_async_buffer_2_notify_event() {
    let p = policy_after_inject("transfer d : async, buffer=2, notify=event;");
    assert_eq!(
        p,
        TransferPolicy {
            synchronous: false,
            buffer: 2,
            notify: NotifyMode::Event,
        }
    );
}

// --------------------------------------------------------------------
// Idempotence
// --------------------------------------------------------------------

#[test]
fn idempotent_on_synthetic_two_worker_case() {
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["w0"])],
        "transfer d : async, buffer=2, notify=event;",
    );
    let once = inject_transfers(&linked, acfg.clone());
    let twice = inject_transfers(&linked, once.clone());

    // Same structure: same Push/Wait counts, same positions, same
    // (data, src, dst, tile, policy) tuples. We do NOT require seq
    // equality on re-run (the second pass leaves placeholders intact
    // by structural skip, so seq is preserved).
    assert_eq!(once.push_count(), twice.push_count());
    assert_eq!(once.wait_count(), twice.wait_count());

    let xs1 = once.root.collect_xfers();
    let xs2 = twice.root.collect_xfers();
    assert_eq!(xs1.len(), xs2.len());
    for (a, b) in xs1.iter().zip(xs2.iter()) {
        assert_eq!(a.role, b.role);
        assert_eq!(a.src, b.src);
        assert_eq!(a.dst, b.dst);
        assert_eq!(a.data, b.data);
        assert_eq!(a.tile, b.tile);
        assert_eq!(a.policy, b.policy);
        assert_eq!(a.seq, b.seq);
    }

    // Full ACFG equality: tree structure preserved (no extra
    // placeholders).
    assert_eq!(once, twice, "inject_transfers must be idempotent");
}

// --------------------------------------------------------------------
// End-to-end against real examples
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

fn linked_from_paths(algo_rel: &str, sched_rel: &str) -> LinkedIR {
    let algo_ast = parse_algo(&read_example(algo_rel)).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(sched_rel)).expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    link::link(algo, sched).expect("link must succeed")
}

#[test]
fn example_1_naive_has_no_transfers() {
    // Single-worker schedule -> no cross-worker edges -> no xfers.
    let linked = linked_from_paths(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");
    let result = inject_transfers(&linked, acfg);
    assert_eq!(result.xfer_count(), 0);
}

#[test]
fn example_13_naive_has_no_transfers() {
    // Naive places everything on host -> no cross-worker edges.
    let linked = linked_from_paths(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");
    let result = inject_transfers(&linked, acfg);
    assert_eq!(result.xfer_count(), 0);
}

#[test]
fn example_14_naive_has_no_transfers() {
    let linked = linked_from_paths(
        "14-hearing-aid/prog.algo.nuc",
        "14-hearing-aid/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");
    let result = inject_transfers(&linked, acfg);
    assert_eq!(result.xfer_count(), 0);
}

#[test]
fn structural_pairing_holds_for_synthetic_multi_edge() {
    // Re-use the three-hop synthetic case and assert structural
    // properties: every Push has a Wait with the same seq, src, dst,
    // data, tile, and policy.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![0], Some(1)),
        op(&[2], 102, vec![1], Some(2)),
        op(&[0], 103, vec![2], None),
    ]);
    let acfg = synthetic_acfg(
        root,
        &[("d0", 0), ("d1", 1), ("d2", 2)],
        &[("w0", 0), ("w1", 1), ("w2", 2)],
    );
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d0", &["w0"]), ("d1", &["w1"]), ("d2", &["w2"])],
        "transfer d0 : sync; transfer d1 : async, buffer=2; transfer d2 : async, buffer=2, notify=event;",
    );
    let result = inject_transfers(&linked, acfg);

    let xfers = result.root.collect_xfers();
    let mut by_seq: BTreeMap<SeqTag, Vec<&XferPlaceholder>> = BTreeMap::new();
    for x in &xfers {
        by_seq.entry(x.seq).or_default().push(x);
    }
    for (seq, group) in &by_seq {
        assert_eq!(
            group.len(),
            2,
            "seq {:?} should appear on exactly one Push and one Wait",
            seq
        );
        let push = group
            .iter()
            .find(|x| x.role == XferRole::Push)
            .expect("push present");
        let wait = group
            .iter()
            .find(|x| x.role == XferRole::Wait)
            .expect("wait present");
        assert_eq!(push.src, wait.src);
        assert_eq!(push.dst, wait.dst);
        assert_eq!(push.data, wait.data);
        assert_eq!(push.tile, wait.tile);
        assert_eq!(push.policy, wait.policy);
    }
}

#[test]
fn inject_transfers_preserves_name_tables() {
    // Sanity: name tables forwarded unchanged.
    let linked = linked_from_paths(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let before = build_acfg(&linked).expect("build_acfg");
    let after = inject_transfers(&linked, before.clone());
    assert_eq!(before.name_kernels, after.name_kernels);
    assert_eq!(before.name_data, after.name_data);
    assert_eq!(before.name_workers, after.name_workers);
    assert_eq!(before.name_iter_vars, after.name_iter_vars);
}

#[test]
fn inject_transfers_preserves_operation_count_on_real_examples() {
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
            "14-hearing-aid/prog.algo.nuc",
            "14-hearing-aid/schedules/naive.sched.nuc",
        ),
    ] {
        let linked = linked_from_paths(algo, sched);
        let before = build_acfg(&linked).expect("build_acfg");
        let after = inject_transfers(&linked, before.clone());
        assert_eq!(
            before.operation_count(),
            after.operation_count(),
            "operation count must be preserved (algo={algo}, sched={sched})"
        );
        assert_eq!(
            before.repeat_count(),
            after.repeat_count(),
            "repeat count must be preserved (algo={algo}, sched={sched})"
        );
    }
}
