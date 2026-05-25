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

use nucleus_compiler::acfg::{
    build_acfg, ACFGNode, DataflowDag, DataflowEdge, NotifyMode, Operation, TransferPolicy,
    XferPlaceholder, XferRole, ACFG,
};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{DataId, IterVar, KernelId, SeqTag, WorkerId};
use nucleus_compiler::link::{self, LinkedIR, WorkerEntity};
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::{lower_sched, parse_sched};

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
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
        partition_pairs: std::collections::BTreeMap::new(),
        grid_shape_for_outer_iv: std::collections::BTreeMap::new(),
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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

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
    let once = inject_transfers(&linked, acfg.clone()).expect("inject_transfers");
    let twice = inject_transfers(&linked, once.clone()).expect("inject_transfers");

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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
    assert_eq!(result.xfer_count(), 0);
}

#[test]
fn example_14_naive_has_no_transfers() {
    let linked = linked_from_paths(
        "14-hearing-aid/prog.algo.nuc",
        "14-hearing-aid/schedules/naive.sched.nuc",
    );
    let acfg = build_acfg(&linked).expect("build_acfg");
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
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
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

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
    let after = inject_transfers(&linked, before.clone()).expect("inject_transfers");
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
        let after = inject_transfers(&linked, before.clone()).expect("inject_transfers");
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

// --------------------------------------------------------------------
// TASK-0117 fan-out
// --------------------------------------------------------------------

/// 1:N broadcast (one host producer, N compute consumers) emits N
/// Push/Wait pairs with distinct `seq` tags.
#[test]
fn fanout_one_to_n_emits_n_pairs() {
    // host produces d0; {w1,w2,w3,w4} consumes d0.
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),           // host
        op(&[1, 2, 3, 4], 101, vec![0], Some(1)), // {w1..w4}
    ]);
    let acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    // 4 destination workers -> 4 Push/Wait pairs.
    assert_eq!(result.push_count(), 4, "one Push per destination worker");
    assert_eq!(result.wait_count(), 4, "one Wait per destination worker");

    let xfers = result.root.collect_xfers();
    // Distinct seqs.
    let seqs: BTreeSet<SeqTag> = xfers.iter().map(|x| x.seq).collect();
    assert_eq!(seqs.len(), 4, "four distinct seq tags");

    // Each Push has src=host(0) and dst in {1,2,3,4}; each Wait has
    // matching src=host(0) and the same dst.
    let pushes: Vec<_> = xfers.iter().filter(|x| x.role == XferRole::Push).collect();
    let waits: Vec<_> = xfers.iter().filter(|x| x.role == XferRole::Wait).collect();
    let push_dsts: BTreeSet<WorkerId> = pushes.iter().map(|x| x.dst).collect();
    let wait_dsts: BTreeSet<WorkerId> = waits.iter().map(|x| x.dst).collect();
    assert_eq!(
        push_dsts,
        BTreeSet::from([WorkerId(1), WorkerId(2), WorkerId(3), WorkerId(4)]),
        "Push dsts cover every consumer worker"
    );
    assert_eq!(push_dsts, wait_dsts);
    for x in &xfers {
        assert_eq!(x.src, WorkerId(0));
        assert_eq!(x.data, DataId(0));
    }
}

/// N:1 gather (N compute producers, one host consumer) emits N
/// Push/Wait pairs with distinct `seq` tags.
///
/// Producers are multiple workers writing the same single-assignment
/// data symbol; build_acfg models this as one Operation with
/// `workers: {w0..w3}`. (Outside this synthetic shape — in real
/// schedules — distributed placement is the source.)
#[test]
fn fanout_n_to_one_emits_n_pairs() {
    let root = ACFGNode::Sequence(vec![
        op(&[1, 2, 3, 4], 100, vec![], Some(0)), // producer on {w1..w4}
        op(&[0], 101, vec![0], Some(1)),         // consumer on host
    ]);
    let acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["w1", "w2", "w3", "w4"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    assert_eq!(result.push_count(), 4, "one Push per producer worker");
    assert_eq!(result.wait_count(), 4, "one Wait per producer worker");

    let xfers = result.root.collect_xfers();
    let seqs: BTreeSet<SeqTag> = xfers.iter().map(|x| x.seq).collect();
    assert_eq!(seqs.len(), 4);

    let pushes: Vec<_> = xfers.iter().filter(|x| x.role == XferRole::Push).collect();
    let push_srcs: BTreeSet<WorkerId> = pushes.iter().map(|x| x.src).collect();
    assert_eq!(
        push_srcs,
        BTreeSet::from([WorkerId(1), WorkerId(2), WorkerId(3), WorkerId(4)]),
        "Push srcs cover every producer worker"
    );
    for x in &xfers {
        assert_eq!(x.dst, WorkerId(0));
        assert_eq!(x.data, DataId(0));
    }
}

/// The 1:1 host↔single-worker case (examples 01..07 shape) emits
/// exactly ONE pair after fan-out, with no spurious replication; this
/// is the no-regression guard for the pre-TASK-0117 contract on those
/// cells.
#[test]
fn fanout_one_to_one_unchanged() {
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),  // host
        op(&[1], 101, vec![0], Some(1)), // w0
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("host", 0), ("w0", 1)]);
    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    assert_eq!(result.push_count(), 1);
    assert_eq!(result.wait_count(), 1);
}

/// Determinism: the fan-out ordering is a deterministic function of
/// (producer_workers, consumer_workers). Re-running the pass on the
/// same input produces structurally identical XferPlaceholders with
/// the same seq tags (the State::next_seq counter is monotonic and
/// the BTreeSet iteration is sorted).
#[test]
fn fanout_is_deterministic_across_runs() {
    let mk_acfg = || {
        let root = ACFGNode::Sequence(vec![
            op(&[0], 100, vec![], Some(0)),
            op(&[1, 2, 3, 4], 101, vec![0], Some(1)),
        ]);
        synthetic_acfg(
            root,
            &[("d", 0), ("c", 1)],
            &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
        )
    };
    let linked = synthetic_linked_ir(
        &mk_acfg().name_data,
        &mk_acfg().name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let r1 = inject_transfers(&linked, mk_acfg())
        .expect("inject_transfers")
        .root
        .collect_xfers();
    let r2 = inject_transfers(&linked, mk_acfg())
        .expect("inject_transfers")
        .root
        .collect_xfers();
    assert_eq!(r1.len(), r2.len());
    for (a, b) in r1.iter().zip(r2.iter()) {
        assert_eq!(a.role, b.role);
        assert_eq!(a.src, b.src);
        assert_eq!(a.dst, b.dst);
        assert_eq!(a.data, b.data);
        assert_eq!(a.seq, b.seq, "seq tags reproduce across runs");
    }
}

/// With a `partition_worker_ranges` sidecar populated, fan-out
/// rewrites each pair's tile to the compute worker's slice. Tests
/// the 1:N input direction at the {host}→{w1..w4} shape over an
/// iter-var with B=8 split 4 ways.
#[test]
fn fanout_per_worker_tile_for_input_direction() {
    // Real partition_workers-shaped ACFG: the partitioned Repeat node
    // for IterVar(7) actually wraps the consumer op (TASK-0224 — the
    // partition-rewrite walks the ACFG topology, so the Repeat must
    // be present, mirroring what the partition_workers pass would
    // have produced for a `for n : partition=workers;` schedule).
    let body = ACFGNode::Sequence(vec![op(&[1, 2, 3, 4], 101, vec![0], Some(1))]);
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..8,
            body: Box::new(body),
            block_tag: None,
        },
    ]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    // Register iter-var "n" against IterVar(7) and populate the
    // partition sidecar: source range 0..8 split across {w1..w4} so
    // each worker owns a 2-element slice.
    acfg.name_iter_vars.insert("n".to_string(), IterVar(7));
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..2);
    per_worker.insert(WorkerId(2), 2..4);
    per_worker.insert(WorkerId(3), 4..6);
    per_worker.insert(WorkerId(4), 6..8);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    let xfers = result.root.collect_xfers();
    // Each pair's tile must carry the dst-worker's partition slice.
    for x in &xfers {
        // The compute worker for a (host, w_i) pair is w_i.
        let expected_range: std::ops::Range<i64> = match x.dst.0 {
            1 => 0..2,
            2 => 2..4,
            3 => 4..6,
            4 => 6..8,
            _ => panic!("unexpected dst worker {:?}", x.dst),
        };
        assert_eq!(
            x.tile.bounds,
            vec![(IterVar(7), expected_range)],
            "pair {:?}->{:?} tile must be the dst worker's partition slice",
            x.src,
            x.dst
        );
    }
}

/// Same per-worker tile assignment in the N:1 gather direction
/// (workers→host). The compute worker for each pair is the src.
#[test]
fn fanout_per_worker_tile_for_output_direction() {
    // Real partition_workers-shaped ACFG (see input-direction sibling
    // for the TASK-0224 rationale): producer op runs inside the
    // partitioned Repeat, gather happens at top-level on host.
    let producer_body = ACFGNode::Sequence(vec![op(&[1, 2, 3, 4], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..8,
            body: Box::new(producer_body),
            block_tag: None,
        },
        op(&[0], 101, vec![0], Some(1)),
    ]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    acfg.name_iter_vars.insert("n".to_string(), IterVar(7));
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..2);
    per_worker.insert(WorkerId(2), 2..4);
    per_worker.insert(WorkerId(3), 4..6);
    per_worker.insert(WorkerId(4), 6..8);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["w1", "w2", "w3", "w4"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    let xfers = result.root.collect_xfers();
    for x in &xfers {
        let expected_range: std::ops::Range<i64> = match x.src.0 {
            1 => 0..2,
            2 => 2..4,
            3 => 4..6,
            4 => 6..8,
            _ => panic!("unexpected src worker {:?}", x.src),
        };
        assert_eq!(
            x.tile.bounds,
            vec![(IterVar(7), expected_range)],
            "pair {:?}->{:?} tile must be the src worker's partition slice",
            x.src,
            x.dst
        );
    }
}

/// Empty `partition_worker_ranges` (the pre-TASK-0212 / non-partitioned
/// case) leaves the pair tiles untouched at the construction-site
/// enclosing tile. No regression for examples 01..07 which never set
// --------------------------------------------------------------------
// TASK-0134: pipeline-depth sidecar
// --------------------------------------------------------------------

#[test]
fn pipeline_depth_populated_for_inter_stage_transfers() {
    // TASK-0134 AC#1: the sidecar carries SeqTag -> D for every
    // Push/Wait pair created inside a pipelined loop body.
    //
    // Real fixture: example 13 with pipeline_parallel schedule.
    // `loop n : pipeline=3`; feat1 / feat2 transfers have producer
    // AND consumer inside the loop, so their seqs MUST be in the
    // sidecar with value 3. input / output cross the loop boundary
    // and are hoisted out -> no entry expected.
    use nucleus_compiler::algo::{lower_algo, parse_algo};
    use nucleus_compiler::sched::{lower_sched, parse_sched};

    let algo_ast = parse_algo(&read_example("13-cnn-inference/prog.algo.nuc")).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    ))
    .expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = nucleus_compiler::passes::sync_inject::inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    assert!(
        !acfg.pipeline_depth_for_seq.is_empty(),
        "expected at least one pipeline-depth annotation; got empty"
    );

    // Find the data_id for feat1, feat2, input, output.
    let id_of = |name: &str| acfg.name_data.get(name).copied();
    let feat1_id = id_of("feat1").expect("feat1");
    let feat2_id = id_of("feat2").expect("feat2");
    let input_id = id_of("input").expect("input");
    let output_id = id_of("output").expect("output");

    // Walk the ACFG and collect (seq, data) for every Xfer node.
    fn walk(node: &nucleus_compiler::acfg::ACFGNode, out: &mut Vec<(SeqTag, DataId)>) {
        match node {
            nucleus_compiler::acfg::ACFGNode::Xfer(x) => out.push((x.seq, x.data)),
            nucleus_compiler::acfg::ACFGNode::Sequence(cs) => cs.iter().for_each(|c| walk(c, out)),
            nucleus_compiler::acfg::ACFGNode::Repeat { body, .. } => walk(body, out),
            _ => {}
        }
    }
    let mut xfers: Vec<(SeqTag, DataId)> = Vec::new();
    walk(&acfg.root, &mut xfers);

    // Group by data_id.
    let mut seqs_for: BTreeMap<DataId, BTreeSet<SeqTag>> = BTreeMap::new();
    for (s, d) in xfers {
        seqs_for.entry(d).or_default().insert(s);
    }

    // Sanity: feat1 has at least one Push/Wait pair.
    assert!(!seqs_for
        .get(&feat1_id)
        .unwrap_or(&BTreeSet::new())
        .is_empty());
    assert!(!seqs_for
        .get(&feat2_id)
        .unwrap_or(&BTreeSet::new())
        .is_empty());

    let expect_depth = std::num::NonZeroU64::new(3).unwrap();
    for s in seqs_for.get(&feat1_id).unwrap() {
        assert_eq!(
            acfg.pipeline_depth_for_seq.get(s),
            Some(&expect_depth),
            "feat1 seq {:?} must carry pipeline depth 3",
            s
        );
    }
    for s in seqs_for.get(&feat2_id).unwrap() {
        assert_eq!(
            acfg.pipeline_depth_for_seq.get(s),
            Some(&expect_depth),
            "feat2 seq {:?} must carry pipeline depth 3",
            s
        );
    }

    // input / output get hoisted out of the loop (producer or
    // consumer outside); the post-hoist tile drops the `n` axis, so
    // they have NO pipeline-depth entry.
    for s in seqs_for.get(&input_id).unwrap_or(&BTreeSet::new()) {
        assert!(
            !acfg.pipeline_depth_for_seq.contains_key(s),
            "input seq {:?} must NOT carry pipeline depth (hoisted out of loop)",
            s
        );
    }
    for s in seqs_for.get(&output_id).unwrap_or(&BTreeSet::new()) {
        assert!(
            !acfg.pipeline_depth_for_seq.contains_key(s),
            "output seq {:?} must NOT carry pipeline depth (consumed outside loop)",
            s
        );
    }
}

#[test]
fn pipeline_depth_empty_for_non_pipelined_schedules() {
    // Regression guard: a schedule WITHOUT `pipeline=` must leave
    // `pipeline_depth_for_seq` empty. Pre-TASK-0134 behaviour
    // preserved for every existing example.
    use nucleus_compiler::algo::{lower_algo, parse_algo};
    use nucleus_compiler::sched::{lower_sched, parse_sched};

    let algo_ast = parse_algo(&read_example("02-split-add/prog.algo.nuc")).expect("algo");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast =
        parse_sched(&read_example("02-split-add/schedules/split.sched.nuc")).expect("sched");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = nucleus_compiler::passes::sync_inject::inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    assert!(
        acfg.pipeline_depth_for_seq.is_empty(),
        "no pipeline= in schedule must leave the sidecar empty; got {:?}",
        acfg.pipeline_depth_for_seq
    );
}

#[test]
fn fanout_empty_partition_sidecar_preserves_construction_tile() {
    // No partition_worker_ranges entries; the 1:1 case keeps the
    // pre-TASK-0117 tile (empty top-level for a flat sequence).
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        op(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("host", 0), ("w0", 1)]);
    // partition_worker_ranges left empty by `synthetic_acfg`.

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    let xfers = result.root.collect_xfers();
    for x in &xfers {
        assert!(
            x.tile.bounds.is_empty(),
            "no partition sidecar => no partition rewrite => empty tile"
        );
    }
}

// --------------------------------------------------------------------
// TASK-0216: partition=workers + pipeline=D combination
// --------------------------------------------------------------------

/// Synthetic fixture: partition_worker_ranges + pipeline=D on the
/// SAME loop variable. Asserts (AC#1) that pipeline_depth_for_seq is
/// populated correctly for each per-worker fan-out pair.
///
/// Setup mirrors `fanout_per_worker_tile_for_input_direction` but
/// adds a `loop n : pipeline=2;` schedule directive. The post-pass
/// `annotate_pipeline_depth_for_seq` walks each Xfer's `tile.bounds`
/// in `.rev()` order looking for an iter-var with a pipeline depth;
/// for the single-iter-var tile produced by partition-rewrite, the
/// .rev() walk finds IterVar(7) on the first step and reads D=2.
///
/// Latency: the architect's id-order-vs-nest-order concern was that
/// `rewrite_partition_tiles_inner` iterates `partition_ranges`
/// (BTreeMap<IterVar, ...>) in IterVar id order, NOT nest order; for
/// MULTIPLE partitioned iter-vars in nested fashion these can diverge
/// theoretically. For real schedules with a single partitioned
/// iter-var (the only shape any in-tree example uses) `bounds` has
/// length 1 and ordering is trivial. This test pins that case;
/// nested-multi-partitioned coverage is deferred to TASK-0216.01 if
/// it ever becomes a real shape.
#[test]
fn partition_with_pipeline_populates_pipeline_depth_per_fanout_pair() {
    // Real partition_workers-shaped ACFG: the partitioned Repeat node
    // for IterVar(7) actually exists in the tree and wraps the
    // consumer op (TASK-0224: the rewrite walks the ACFG topology to
    // derive partition-axis nest order, so the Repeat must really be
    // present — the partition_workers pass guarantees this invariant
    // in non-synthetic flows).
    let body = ACFGNode::Sequence(vec![op(&[1, 2, 3, 4], 101, vec![0], Some(1))]);
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..8,
            body: Box::new(body),
            block_tag: None,
        },
    ]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    acfg.name_iter_vars.insert("n".to_string(), IterVar(7));
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..2);
    per_worker.insert(WorkerId(2), 2..4);
    per_worker.insert(WorkerId(3), 4..6);
    per_worker.insert(WorkerId(4), 6..8);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        // Pipeline directive on the partitioned loop. The synthetic
        // sched-lower path doesn't require the loop var to exist in
        // an algorithm — it's a sched-side declaration.
        "loop n : pipeline=2;\n    transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    let xfers = result.root.collect_xfers();
    assert!(!xfers.is_empty(), "expected fan-out xfers; got none");

    let expect_depth = std::num::NonZeroU64::new(2).expect("D=2 nonzero");

    // Every fan-out pair's seq must carry pipeline_depth_for_seq[seq] = D.
    for x in &xfers {
        assert_eq!(
            x.tile.bounds.len(),
            1,
            "single-iter-var partition produces a 1-element bounds vec; \
             got {:?}",
            x.tile.bounds
        );
        assert_eq!(x.tile.bounds[0].0, IterVar(7));
        assert_eq!(
            result.pipeline_depth_for_seq.get(&x.seq),
            Some(&expect_depth),
            "pair seq {:?} must carry pipeline depth 2 (the post-pass \
             reads IterVar(7) from the tile and resolves pipeline_depth_for_iter_var[IterVar(7)]=2)",
            x.seq
        );
    }

    // Cross-check: the sidecar has one entry per UNIQUE seq (Push
    // and Wait of a pair share one seq, so 4 fan-out pairs = 4 seqs
    // = 4 annotations, NOT 8 xfers).
    let unique_seqs: std::collections::BTreeSet<_> = xfers.iter().map(|x| x.seq).collect();
    let annotated_count = result.pipeline_depth_for_seq.len();
    assert_eq!(
        annotated_count,
        unique_seqs.len(),
        "pipeline_depth_for_seq must have one entry per UNIQUE seq \
         (Push+Wait share a seq); got {} annotated vs {} unique seqs",
        annotated_count,
        unique_seqs.len()
    );
}

// --------------------------------------------------------------------
// TASK-0224: rewrite_partition_tiles bounds must be in NEST order
// (outer-to-inner), not BTreeMap-key order (IterVar-id ascending).
// --------------------------------------------------------------------

/// Synthetic fixture exercising 2 nested partitioned iter-vars whose
/// IterVar ids run COUNTER to nest order: outer Repeat uses IterVar(7),
/// inner Repeat uses IterVar(3). Both iter-vars have a
/// `partition_worker_ranges` entry for the same worker set.
///
/// `IterTile::bounds`'s outer-to-inner convention is load-bearing for
/// the post-pass `annotate_pipeline_depth_for_seq` `.rev()` walk
/// (innermost-wins). Before TASK-0224 `rewrite_partition_tiles_inner`
/// iterated `partition_ranges: BTreeMap<IterVar, ...>` in key-ascending
/// order — which is IterVar-id ascending order. For schedules where
/// id-order COINCIDES with nest order (every in-tree example today)
/// that produces correct bounds; for schedules where it DIVERGES (this
/// test) the bounds vec ends up reversed and the convention silently
/// breaks. TASK-0224 fixes this by walking the enclosing Repeat stack
/// instead.
///
/// The assertion: bounds must be `[(IterVar(7), 0..2), (IterVar(3),
/// 0..3)]` — OUTER-FIRST — for a fan-out pair landing on WorkerId(1),
/// not the reversed `[(IterVar(3), ...), (IterVar(7), ...)]`.
#[test]
fn rewrite_partition_tiles_bounds_in_nest_order_not_itervar_id_order() {
    // Outer Repeat: IterVar(7) range 0..4, partitioned across w1/w2.
    // Inner Repeat: IterVar(3) range 0..6, partitioned across w1/w2.
    // Inside inner body: op on {w1, w2} reads `d` (produced by host).
    // Fan-out triggers cross-worker Push/Wait per (host -> w_i) pair.
    let inner_body = ACFGNode::Sequence(vec![op(&[1, 2], 101, vec![0], Some(1))]);
    let outer_body = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: IterVar(3),
        range: 0..6,
        body: Box::new(inner_body),
        block_tag: None,
    }]);
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..4,
            body: Box::new(outer_body),
            block_tag: None,
        },
    ]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2)],
    );
    // Name both iter-vars so the schedule loop-name resolution would
    // work if a future schedule wanted to refer to them; not strictly
    // required for this test since we set partition_worker_ranges by
    // hand below.
    acfg.name_iter_vars.insert("n".to_string(), IterVar(7));
    acfg.name_iter_vars.insert("k".to_string(), IterVar(3));

    // OUTER partition: IterVar(7) -> {w1: 0..2, w2: 2..4}
    let mut outer_per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    outer_per_worker.insert(WorkerId(1), 0..2);
    outer_per_worker.insert(WorkerId(2), 2..4);
    acfg.partition_worker_ranges
        .insert(IterVar(7), outer_per_worker);
    // INNER partition: IterVar(3) -> {w1: 0..3, w2: 3..6}
    let mut inner_per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    inner_per_worker.insert(WorkerId(1), 0..3);
    inner_per_worker.insert(WorkerId(2), 3..6);
    acfg.partition_worker_ranges
        .insert(IterVar(3), inner_per_worker);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    // Collect Wait xfers grouped by dst worker; for each dst, the tile
    // bounds must be in OUTER-to-INNER nest order — IterVar(7) first,
    // IterVar(3) second — with each axis carrying the per-worker
    // partition slice for that dst.
    let xfers = result.root.collect_xfers();
    let waits: Vec<&XferPlaceholder> = xfers.iter().filter(|x| x.role == XferRole::Wait).collect();
    assert!(
        !waits.is_empty(),
        "expected fan-out Wait xfers from the nested-partitioned op; got none"
    );
    for w in &waits {
        assert_eq!(
            w.tile.bounds.len(),
            2,
            "two enclosing partitioned iter-vars => bounds.len()==2; \
             got {:?}",
            w.tile.bounds
        );
        // Position 0: outer IterVar(7); position 1: inner IterVar(3).
        // BEFORE TASK-0224 the order was reversed (IterVar-id ascending
        // = (3, 7)); AFTER the fix it follows nest order.
        assert_eq!(
            w.tile.bounds[0].0,
            IterVar(7),
            "bounds[0] must be the OUTER iter-var (IterVar(7)) per the \
             outer-to-inner convention; got {:?} (TASK-0224: rewrite \
             must walk enclosing-Repeat stack, not partition_ranges \
             BTreeMap keys)",
            w.tile.bounds[0].0
        );
        assert_eq!(
            w.tile.bounds[1].0,
            IterVar(3),
            "bounds[1] must be the INNER iter-var (IterVar(3)); got \
             {:?}",
            w.tile.bounds[1].0
        );
        // Cross-check the per-worker slice survived the refactor (the
        // load-bearing semantic the cycle-45 attempt broke).
        let expected_outer = match w.dst {
            WorkerId(1) => 0..2,
            WorkerId(2) => 2..4,
            other => panic!("unexpected fan-out dst {other:?}"),
        };
        let expected_inner = match w.dst {
            WorkerId(1) => 0..3,
            WorkerId(2) => 3..6,
            other => panic!("unexpected fan-out dst {other:?}"),
        };
        assert_eq!(
            w.tile.bounds[0].1, expected_outer,
            "outer slice for dst {:?} must be the per-worker partition \
             range from IterVar(7)'s sidecar entry",
            w.dst
        );
        assert_eq!(
            w.tile.bounds[1].1, expected_inner,
            "inner slice for dst {:?} must be the per-worker partition \
             range from IterVar(3)'s sidecar entry",
            w.dst
        );
    }
}

/// Three-level non-monotonic nest. Outer IterVar(9), middle IterVar(2),
/// inner IterVar(5). All three are partitioned across the same worker
/// set. Asserts the full 3-element bounds vec is in nest order
/// `[9, 2, 5]`, not IterVar-id ascending `[2, 5, 9]` and not
/// IterVar-id descending `[9, 5, 2]`. Catches a fix that merely sorts
/// instead of walking the actual enclosing stack.
#[test]
fn rewrite_partition_tiles_three_level_nest_order() {
    let innermost_body = ACFGNode::Sequence(vec![op(&[1, 2], 101, vec![0], Some(1))]);
    let middle_body = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: IterVar(5),
        range: 0..4,
        body: Box::new(innermost_body),
        block_tag: None,
    }]);
    let outer_body = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: IterVar(2),
        range: 0..4,
        body: Box::new(middle_body),
        block_tag: None,
    }]);
    let root = ACFGNode::Sequence(vec![
        op(&[0], 100, vec![], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(9),
            range: 0..4,
            body: Box::new(outer_body),
            block_tag: None,
        },
    ]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2)],
    );
    for (name, iv) in [("n", 9), ("k", 2), ("m", 5)] {
        acfg.name_iter_vars.insert(name.to_string(), IterVar(iv));
    }
    for iv_id in [9, 2, 5] {
        let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
        per_worker.insert(WorkerId(1), 0..2);
        per_worker.insert(WorkerId(2), 2..4);
        acfg.partition_worker_ranges
            .insert(IterVar(iv_id), per_worker);
    }

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    let xfers = result.root.collect_xfers();
    let waits: Vec<&XferPlaceholder> = xfers.iter().filter(|x| x.role == XferRole::Wait).collect();
    assert!(!waits.is_empty(), "expected fan-out Waits");
    for w in &waits {
        let order: Vec<IterVar> = w.tile.bounds.iter().map(|(iv, _)| *iv).collect();
        assert_eq!(
            order,
            vec![IterVar(9), IterVar(2), IterVar(5)],
            "three-level nest bounds must be OUTER-to-INNER (9, 2, 5); \
             got {:?} — IterVar-id ascending would be (2, 5, 9), \
             id-descending (9, 5, 2); only a true Repeat-stack walk \
             yields (9, 2, 5)",
            order
        );
    }
}

// --------------------------------------------------------------------
// TASK-0301: per-data axis-mapping filter
// --------------------------------------------------------------------

/// Pin the 07-matmul/distributed shape at the unit-test level: a triple
/// loop `for i { for j { for k { c[i][j] <-- madd(c[i][j], a[i][k],
/// b[k][j]) }}}` with `partition=workers` on `i` must produce per-Xfer
/// tiles that reflect ONLY the partitioned ivs that actually index the
/// transferred data symbol. Specifically:
///
/// - `a` (indexed `[i][k]`) — `i` is observed; bounds must carry
///   `(i, i_band)`.
/// - `c` (indexed `[i][j]`) — `i` is observed; bounds must carry
///   `(i, i_band)`.
/// - `b` (indexed `[k][j]`) — `i` is NOT observed; bounds must be
///   EMPTY → wait_slice will hit the whole-array arm (broadcast full
///   `b` to every worker).
///
/// Pre-TASK-0301 every fan-out Xfer received `bounds = [(i, i_band)]`
/// unconditionally, including `b`'s — which `wait_slice` then silently
/// sliced as a leading-axis range of `b`'s `k` dimension. This test
/// would have caught that silent mis-slice at unit speed.
///
/// The 07-matmul/distributed e2e cell is the only other proving ground;
/// this pin localises a future regression.
#[test]
fn rewrite_partition_tiles_filters_non_indexing_iv_for_07_matmul_shape() {
    use nucleus_compiler::acfg::{DataAccess, DataflowDag, DataflowEdge, Operation};
    use nucleus_compiler::algo::ir::IrExpr;
    use nucleus_compiler::event::ArgBinding;

    // IterVar ids — pick non-monotonic so a regression that reverts to
    // BTreeMap-key-order is independently visible (cf. TASK-0224).
    const IV_I: IterVar = IterVar(7);
    const IV_J: IterVar = IterVar(3);
    const IV_K: IterVar = IterVar(5);
    // Data ids
    const D_A: DataId = DataId(0);
    const D_B: DataId = DataId(1);
    const D_C: DataId = DataId(2);

    fn ident(name: &str) -> IrExpr {
        IrExpr::Ident(name.to_string())
    }

    fn access(data: DataId, ivs: &[&str]) -> DataAccess {
        DataAccess {
            data,
            indices: ivs.iter().map(|n| ident(n)).collect(),
        }
    }

    // Madd op on {w0..w3}: reads c[i][j], a[i][k], b[k][j]; writes c[i][j].
    // Modelled as ONE DataflowEdge with three reads (a, b, c) and one
    // write (c). `data_in` retains duplicates per `DataflowEdge::new`'s
    // invariant — but here we are constructing the edge directly with
    // proper access info, so we include c twice in data_in (once as a
    // read, matching the c[i][j] argument in `madd(c[i][j], a[i][k],
    // b[k][j])`).
    let edge = DataflowEdge {
        data_in: vec![D_C, D_A, D_B],
        kernel: KernelId(100),
        data_out: Some(D_C),
        data_in_access: vec![
            access(D_C, &["i", "j"]),
            access(D_A, &["i", "k"]),
            access(D_B, &["k", "j"]),
        ],
        data_out_access: Some(access(D_C, &["i", "j"])),
        args: vec![
            ArgBinding::Data(access(D_C, &["i", "j"])),
            ArgBinding::Data(access(D_A, &["i", "k"])),
            ArgBinding::Data(access(D_B, &["k", "j"])),
        ],
    };
    let madd_op = ACFGNode::Operation(Operation {
        kernel: KernelId(100),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![edge] },
    });

    // Triple loop nest: outer `i` (partitioned), middle `j`, inner `k`.
    let k_body = ACFGNode::Sequence(vec![madd_op]);
    let k_loop = ACFGNode::Repeat {
        iter_var: IV_K,
        range: 0..16,
        body: Box::new(k_body),
        block_tag: None,
    };
    let j_body = ACFGNode::Sequence(vec![k_loop]);
    let j_loop = ACFGNode::Repeat {
        iter_var: IV_J,
        range: 0..16,
        body: Box::new(j_body),
        block_tag: None,
    };
    let i_body = ACFGNode::Sequence(vec![j_loop]);
    let i_loop = ACFGNode::Repeat {
        iter_var: IV_I,
        range: 0..16,
        body: Box::new(i_body),
        block_tag: None,
    };

    // Host-side load_a / load_b ops at top level (producers of a, b).
    // c is produced by madd_op (workers), consumed by save_c (host).
    // load_* read no inputs; their data_out_access carries `data` with
    // empty indices (bare aggregate load).
    fn host_loader(data_out: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![],
            kernel: KernelId(99),
            data_out: Some(data_out),
            data_in_access: vec![],
            data_out_access: Some(DataAccess {
                data: data_out,
                indices: vec![],
            }),
            args: vec![],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(99),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }
    fn host_saver(data_in: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![data_in],
            kernel: KernelId(98),
            data_out: None,
            data_in_access: vec![DataAccess {
                data: data_in,
                indices: vec![],
            }],
            data_out_access: None,
            args: vec![ArgBinding::Data(DataAccess {
                data: data_in,
                indices: vec![],
            })],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(98),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }

    let root = ACFGNode::Sequence(vec![
        host_loader(D_A),
        host_loader(D_B),
        i_loop,
        host_saver(D_C),
    ]);

    let mut acfg = synthetic_acfg(
        root,
        &[("a", 0), ("b", 1), ("c", 2)],
        &[
            ("host", 0),
            ("w0", 1),
            ("w1", 2),
            ("w2", 3),
            ("w3", 4),
        ],
    );
    acfg.name_iter_vars.insert("i".to_string(), IV_I);
    acfg.name_iter_vars.insert("j".to_string(), IV_J);
    acfg.name_iter_vars.insert("k".to_string(), IV_K);

    // Partition i across 4 workers: 0..4, 4..8, 8..12, 12..16.
    let mut i_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    i_bands.insert(WorkerId(1), 0..4);
    i_bands.insert(WorkerId(2), 4..8);
    i_bands.insert(WorkerId(3), 8..12);
    i_bands.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges.insert(IV_I, i_bands);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("a", &["host"]), ("b", &["host"]), ("c", &["w0", "w1", "w2", "w3"])],
        "transfer a : sync; transfer b : sync; transfer c : sync;",
    );

    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
    let xfers = result.root.collect_xfers();

    // Group Waits by data — we care about the SHAPE of bounds, not the
    // dst-specific range value (which is already pinned by other tests).
    let waits_for = |data: DataId| -> Vec<XferPlaceholder> {
        xfers
            .iter()
            .filter(|x| x.role == XferRole::Wait && x.data == data)
            .cloned()
            .collect()
    };

    let a_waits = waits_for(D_A);
    let b_waits = waits_for(D_B);
    let c_waits = waits_for(D_C);

    assert!(!a_waits.is_empty(), "expected fan-out Waits for data a");
    assert!(!b_waits.is_empty(), "expected fan-out Waits for data b");
    assert!(!c_waits.is_empty(), "expected gather Waits for data c");

    // a is indexed [i][k] — observed iv set = {i, k}. Filter keeps i
    // (the only partitioned axis); bounds must carry exactly [(i, ...)].
    for w in &a_waits {
        let order: Vec<IterVar> = w.tile.bounds.iter().map(|(iv, _)| *iv).collect();
        assert_eq!(
            order,
            vec![IV_I],
            "data a bounds must be [(i, i_band)] only (i indexes a); \
             got {:?}",
            order
        );
    }

    // c is indexed [i][j] — observed iv set = {i, j}. Filter keeps i;
    // bounds must carry exactly [(i, ...)].
    for w in &c_waits {
        let order: Vec<IterVar> = w.tile.bounds.iter().map(|(iv, _)| *iv).collect();
        assert_eq!(
            order,
            vec![IV_I],
            "data c bounds must be [(i, i_band)] only (i indexes c); \
             got {:?}",
            order
        );
    }

    // b is indexed [k][j] — observed iv set = {k, j}. i is NOT in it,
    // so the TASK-0301 filter excludes i → bounds must be EMPTY (the
    // wait_slice whole-array arm). Pre-TASK-0301 this carried
    // [(i, i_band)] and silently mis-sliced b's k axis.
    for w in &b_waits {
        assert!(
            w.tile.bounds.is_empty(),
            "data b bounds must be EMPTY (i does not index b — b is \
             [k][j]); got {:?}. Pre-TASK-0301 this carried [(i, \
             i_band)] and `wait_slice` silently sliced b's k dim by \
             i_band — worker 0 would receive only b[k=0..i_band.end] \
             instead of full b.",
            w.tile.bounds
        );
    }
}

// --------------------------------------------------------------------
// TASK-0302: per-dim contiguous-prefix axis-mapping filter
// --------------------------------------------------------------------

/// Pin the 07-matmul/distributed-2d shape at the unit-test level:
/// triple loop `for i { for j { for k { c[i][j] <-- madd(c[i][j],
/// a[i][k], b[k][j]) }}}` with `partition=blocks2d` on the (i, j) nest
/// must produce per-Xfer tiles consistent with the data-dim-prefix
/// invariant `wait_slice` rests on (`tile.bounds[i].iter_var ↔
/// data.dim[i]`). Specifically:
///
/// - `a` (indexed `[i][k]`) — dim 0 covered by partitioned `i`, dim 1
///   not covered. Coverage `{0}` is a contiguous prefix; bounds must
///   carry exactly `[(i, i_band)]`.
/// - `c` (indexed `[i][j]`) — dim 0 covered by `i`, dim 1 covered by
///   `j`. Coverage `{0, 1}` is a contiguous prefix; bounds must carry
///   exactly `[(i, i_band), (j, j_band)]` in dim order.
/// - `b` (indexed `[k][j]`) — dim 0 NOT covered (k is not partitioned),
///   dim 1 covered by `j`. Coverage `{1}` is NOT a contiguous prefix
///   from dim 0; bounds must be EMPTY → `wait_slice` whole-array arm.
///
/// Pre-TASK-0302 (with only TASK-0301's per-symbol-union filter), `b`
/// would have received `bounds = [(j, j_band)]` because j IS in b's
/// observed iv union — and `wait_slice` would have silently sliced
/// b's k dimension by `j_band`, mis-addressing the data. This test
/// would catch that regression at unit speed.
///
/// The 07-matmul/distributed-2d e2e cell is the only other proving
/// ground; this pin localises a future regression and exercises the
/// 2D non-prefix arm that the 1D-only `distributed` cell does not.
#[test]
fn rewrite_partition_tiles_dim_prefix_check_for_07_matmul_blocks2d_shape() {
    use nucleus_compiler::acfg::{DataAccess, DataflowDag, DataflowEdge, Operation};
    use nucleus_compiler::algo::ir::IrExpr;
    use nucleus_compiler::event::ArgBinding;

    // IterVar ids — pick non-monotonic so a regression that reverts to
    // BTreeMap-key-order is independently visible (cf. TASK-0224).
    const IV_I: IterVar = IterVar(7);
    const IV_J: IterVar = IterVar(3);
    const IV_K: IterVar = IterVar(5);
    // Data ids
    const D_A: DataId = DataId(0);
    const D_B: DataId = DataId(1);
    const D_C: DataId = DataId(2);

    fn ident(name: &str) -> IrExpr {
        IrExpr::Ident(name.to_string())
    }
    fn access(data: DataId, ivs: &[&str]) -> DataAccess {
        DataAccess {
            data,
            indices: ivs.iter().map(|n| ident(n)).collect(),
        }
    }

    // madd op on {w0..w3}: reads c[i][j], a[i][k], b[k][j]; writes c[i][j].
    let edge = DataflowEdge {
        data_in: vec![D_C, D_A, D_B],
        kernel: KernelId(100),
        data_out: Some(D_C),
        data_in_access: vec![
            access(D_C, &["i", "j"]),
            access(D_A, &["i", "k"]),
            access(D_B, &["k", "j"]),
        ],
        data_out_access: Some(access(D_C, &["i", "j"])),
        args: vec![
            ArgBinding::Data(access(D_C, &["i", "j"])),
            ArgBinding::Data(access(D_A, &["i", "k"])),
            ArgBinding::Data(access(D_B, &["k", "j"])),
        ],
    };
    let madd_op = ACFGNode::Operation(Operation {
        kernel: KernelId(100),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![edge] },
    });

    // Triple loop nest: outer `i` (partitioned), middle `j`
    // (partitioned via blocks2d's pair), inner `k`.
    let k_body = ACFGNode::Sequence(vec![madd_op]);
    let k_loop = ACFGNode::Repeat {
        iter_var: IV_K,
        range: 0..16,
        body: Box::new(k_body),
        block_tag: None,
    };
    let j_body = ACFGNode::Sequence(vec![k_loop]);
    let j_loop = ACFGNode::Repeat {
        iter_var: IV_J,
        range: 0..16,
        body: Box::new(j_body),
        block_tag: None,
    };
    let i_body = ACFGNode::Sequence(vec![j_loop]);
    let i_loop = ACFGNode::Repeat {
        iter_var: IV_I,
        range: 0..16,
        body: Box::new(i_body),
        block_tag: None,
    };

    fn host_loader(data_out: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![],
            kernel: KernelId(99),
            data_out: Some(data_out),
            data_in_access: vec![],
            data_out_access: Some(DataAccess {
                data: data_out,
                indices: vec![],
            }),
            args: vec![],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(99),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }
    fn host_saver(data_in: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![data_in],
            kernel: KernelId(98),
            data_out: None,
            data_in_access: vec![DataAccess {
                data: data_in,
                indices: vec![],
            }],
            data_out_access: None,
            args: vec![ArgBinding::Data(DataAccess {
                data: data_in,
                indices: vec![],
            })],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(98),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }

    let root = ACFGNode::Sequence(vec![
        host_loader(D_A),
        host_loader(D_B),
        i_loop,
        host_saver(D_C),
    ]);

    let mut acfg = synthetic_acfg(
        root,
        &[("a", 0), ("b", 1), ("c", 2)],
        &[
            ("host", 0),
            ("w0", 1),
            ("w1", 2),
            ("w2", 3),
            ("w3", 4),
        ],
    );
    acfg.name_iter_vars.insert("i".to_string(), IV_I);
    acfg.name_iter_vars.insert("j".to_string(), IV_J);
    acfg.name_iter_vars.insert("k".to_string(), IV_K);

    // Simulate blocks2d's (2 x 2) decomposition: i_band = 8 rows, j_band
    // = 8 cols. Same row-major BTreeSet WorkerId assignment that
    // partition_blocks2d uses (verified via 05-stencil/distributed-2d's
    // schedule comment which documents the exact mapping).
    //   w0 -> i=0..8,  j=0..8
    //   w1 -> i=0..8,  j=8..16
    //   w2 -> i=8..16, j=0..8
    //   w3 -> i=8..16, j=8..16
    let mut i_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    i_bands.insert(WorkerId(1), 0..8);
    i_bands.insert(WorkerId(2), 0..8);
    i_bands.insert(WorkerId(3), 8..16);
    i_bands.insert(WorkerId(4), 8..16);
    acfg.partition_worker_ranges.insert(IV_I, i_bands);
    let mut j_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    j_bands.insert(WorkerId(1), 0..8);
    j_bands.insert(WorkerId(2), 8..16);
    j_bands.insert(WorkerId(3), 0..8);
    j_bands.insert(WorkerId(4), 8..16);
    acfg.partition_worker_ranges.insert(IV_J, j_bands);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("a", &["host"]), ("b", &["host"]), ("c", &["w0", "w1", "w2", "w3"])],
        "transfer a : sync; transfer b : sync; transfer c : sync;",
    );

    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
    let xfers = result.root.collect_xfers();

    let waits_for = |data: DataId| -> Vec<XferPlaceholder> {
        xfers
            .iter()
            .filter(|x| x.role == XferRole::Wait && x.data == data)
            .cloned()
            .collect()
    };

    let a_waits = waits_for(D_A);
    let b_waits = waits_for(D_B);
    let c_waits = waits_for(D_C);

    assert!(!a_waits.is_empty(), "expected fan-out Waits for data a");
    assert!(!b_waits.is_empty(), "expected fan-out Waits for data b");
    assert!(!c_waits.is_empty(), "expected gather Waits for data c");

    // a is indexed [i][k] — dim coverage {0}. Contiguous prefix. Bounds
    // must carry exactly [(i, ...)] in dim order.
    for w in &a_waits {
        let order: Vec<IterVar> = w.tile.bounds.iter().map(|(iv, _)| *iv).collect();
        assert_eq!(
            order,
            vec![IV_I],
            "data a bounds must be [(i, i_band)] (i indexes a dim 0; \
             k indexes dim 1 but k is not partitioned); got {:?}",
            order
        );
    }

    // c is indexed [i][j] — dim coverage {0, 1}. Contiguous prefix.
    // Bounds must carry exactly [(i, ...), (j, ...)] in dim order.
    for w in &c_waits {
        let order: Vec<IterVar> = w.tile.bounds.iter().map(|(iv, _)| *iv).collect();
        assert_eq!(
            order,
            vec![IV_I, IV_J],
            "data c bounds must be [(i, i_band), (j, j_band)] in DIM \
             order — i indexes dim 0, j indexes dim 1; got {:?}",
            order
        );
    }

    // b is indexed [k][j] — dim coverage {1} only (k not partitioned).
    // NOT a contiguous prefix from dim 0; bounds must be EMPTY (wait_slice
    // whole-array arm). Pre-TASK-0302 the TASK-0301 per-symbol filter
    // would have admitted j (j IS in b's observed union) and emitted
    // bounds = [(j, j_band)] — which wait_slice would silently slice as
    // b's leading-axis range, mis-mapping j_band to b's k dim.
    for w in &b_waits {
        assert!(
            w.tile.bounds.is_empty(),
            "data b bounds must be EMPTY (b is [k][j]; k is not \
             partitioned, so dim coverage is {{1}} — NOT a contiguous \
             prefix from dim 0; TASK-0302 drops to whole-array \
             broadcast); got {:?}. Pre-TASK-0302 this would have been \
             [(j, j_band)] and `wait_slice` would silently mis-map j to \
             b's k dim.",
            w.tile.bounds
        );
    }
}

/// Pin the AMBIGUOUS multi-iv-per-dim arm of
/// `compute_partition_bounds_with_dim_prefix` (TASK-0302, architect
/// cycle 121 P2.2). When a single data dim is observed indexed by
/// MULTIPLE partitioned ivs (e.g. `a[i+j]` where both `i` and `j`
/// are partitioned), the slicing shape is ambiguous (which partition
/// is "the" partition of this dim?). The defensive choice is to
/// drop to whole-array broadcast — empty bounds.
///
/// No shipped grammar / schedule constructs this shape today (every
/// shipped schedule keeps iv-per-dim cardinality at 1); this pin is
/// for a future grammar widening that admits multi-iv index
/// expressions to a partitioned axis.
#[test]
fn rewrite_partition_tiles_drops_ambiguous_multi_partitioned_iv_per_dim() {
    use nucleus_compiler::acfg::{DataAccess, DataflowDag, DataflowEdge, Operation};
    use nucleus_compiler::algo::ir::{IrBinOp, IrExpr};
    use nucleus_compiler::event::ArgBinding;

    const IV_I: IterVar = IterVar(7);
    const IV_J: IterVar = IterVar(3);
    const D_A: DataId = DataId(0);

    fn ident(name: &str) -> IrExpr {
        IrExpr::Ident(name.to_string())
    }

    // a is indexed at dim 0 by `i + j` — two partitioned ivs at the
    // same dim.
    let access_a = DataAccess {
        data: D_A,
        indices: vec![IrExpr::BinOp(
            IrBinOp::Add,
            Box::new(ident("i")),
            Box::new(ident("j")),
        )],
    };
    let edge = DataflowEdge {
        data_in: vec![D_A],
        kernel: KernelId(100),
        data_out: None,
        data_in_access: vec![access_a.clone()],
        data_out_access: None,
        args: vec![ArgBinding::Data(access_a)],
    };
    let body_op = ACFGNode::Operation(Operation {
        kernel: KernelId(100),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![edge] },
    });

    // Outer i (partitioned), inner j (partitioned).
    let j_loop = ACFGNode::Repeat {
        iter_var: IV_J,
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![body_op])),
        block_tag: None,
    };
    let i_loop = ACFGNode::Repeat {
        iter_var: IV_I,
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![j_loop])),
        block_tag: None,
    };

    // Host producer of a (forces a fan-out Xfer to compute workers).
    let producer = {
        let edge = DataflowEdge {
            data_in: vec![],
            kernel: KernelId(99),
            data_out: Some(D_A),
            data_in_access: vec![],
            data_out_access: Some(DataAccess {
                data: D_A,
                indices: vec![],
            }),
            args: vec![],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(99),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    };

    let root = ACFGNode::Sequence(vec![producer, i_loop]);
    let mut acfg = synthetic_acfg(
        root,
        &[("a", 0)],
        &[
            ("host", 0),
            ("w0", 1),
            ("w1", 2),
            ("w2", 3),
            ("w3", 4),
        ],
    );
    acfg.name_iter_vars.insert("i".to_string(), IV_I);
    acfg.name_iter_vars.insert("j".to_string(), IV_J);

    let mut i_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    i_bands.insert(WorkerId(1), 0..4);
    i_bands.insert(WorkerId(2), 4..8);
    i_bands.insert(WorkerId(3), 8..12);
    i_bands.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges.insert(IV_I, i_bands);
    let mut j_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    j_bands.insert(WorkerId(1), 0..4);
    j_bands.insert(WorkerId(2), 4..8);
    j_bands.insert(WorkerId(3), 8..12);
    j_bands.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges.insert(IV_J, j_bands);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("a", &["host"])],
        "transfer a : sync;",
    );

    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
    let xfers = result.root.collect_xfers();
    let a_waits: Vec<XferPlaceholder> = xfers
        .iter()
        .filter(|x| x.role == XferRole::Wait && x.data == D_A)
        .cloned()
        .collect();
    assert!(!a_waits.is_empty(), "expected fan-out Waits for data a");

    // a's dim 0 is indexed by BOTH i and j (both partitioned). The
    // ambiguity arm of compute_partition_bounds_with_dim_prefix
    // returns Some(Vec::new()) → wait_slice whole-array. If a future
    // change picks "first partitioned iv" or "outer iv" instead, this
    // test fires LOUD.
    for w in &a_waits {
        assert!(
            w.tile.bounds.is_empty(),
            "data a bounds must be EMPTY (dim 0 indexed by BOTH \
             partitioned i and j — ambiguous; TASK-0302 drops to \
             whole-array broadcast); got {:?}",
            w.tile.bounds
        );
    }
}

// --------------------------------------------------------------------
// TASK-0263 Stage 2: halo extension on per-tile transfer ranges
// --------------------------------------------------------------------

/// Build a synthetic ACFG matching the 05-stencil/distributed shape:
/// outer y-loop over `1..15` partitioned across 4 workers; inner x-loop
/// (1..15); body's Operation reads data `d` (DataId 0) with kernel
/// `blur3` (KernelId 100). Returns the ACFG with the halo_widths sidecar
/// pre-populated as `halo_widths[blur3][y] = halo_y, [x] = halo_x`.
fn build_stencil_like_acfg(halo_y: u64, halo_x: u64) -> (ACFG, LinkedIR) {
    // Body: blur3 on {w1, w2, w3, w4} reads `d`.
    let body_op = op(&[1, 2, 3, 4], 100, vec![0], Some(1));
    let inner = ACFGNode::Repeat {
        iter_var: IterVar(8), // x
        range: 1..15,
        body: Box::new(ACFGNode::Sequence(vec![body_op])),
        block_tag: None,
    };
    let outer = ACFGNode::Repeat {
        iter_var: IterVar(7), // y
        range: 1..15,
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
    };
    // Producer of `d` is host (worker 0) — outside the y-loop.
    let root = ACFGNode::Sequence(vec![op(&[0], 99, vec![], Some(0)), outer]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    acfg.name_iter_vars.insert("y".to_string(), IterVar(7));
    acfg.name_iter_vars.insert("x".to_string(), IterVar(8));
    acfg.name_kernels.insert("blur3".to_string(), KernelId(100));

    // Floor-with-spillover partition for y on 4 workers, range 1..15:
    // first 2 get 4 rows, last 2 get 3.
    let mut y_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    y_bands.insert(WorkerId(1), 1..5);
    y_bands.insert(WorkerId(2), 5..9);
    y_bands.insert(WorkerId(3), 9..12);
    y_bands.insert(WorkerId(4), 12..15);
    acfg.partition_worker_ranges.insert(IterVar(7), y_bands);

    // Halo: blur3 reads grid[y+/-1] → halo_widths[blur3][y] = halo_y.
    let mut blur3_halo: BTreeMap<IterVar, u64> = BTreeMap::new();
    if halo_y > 0 {
        blur3_halo.insert(IterVar(7), halo_y);
    }
    if halo_x > 0 {
        blur3_halo.insert(IterVar(8), halo_x);
    }
    if !blur3_halo.is_empty() {
        acfg.halo_widths.insert(KernelId(100), blur3_halo);
    }

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : async, buffer=2, notify=event;",
    );
    (acfg, linked)
}

/// Halo=1 along y, no halo on x — the 05-stencil case. After
/// transfer_inject, each Wait's tile axis along y must be extended by
/// 1 on each side. The (partition × halo) composition:
///   w1: y in 1..5  → 0..6  (extended ±1, clamped to source 1..15 + halo 0..16)
///   w2: y in 5..9  → 4..10
///   w3: y in 9..12 → 8..13
///   w4: y in 12..15 → 11..16
#[test]
fn halo_extends_partition_tile_05_stencil_shape() {
    let (acfg, linked) = build_stencil_like_acfg(/*halo_y=*/ 1, /*halo_x=*/ 0);
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");

    // Expect one Wait per (host, w_i) pair across the 4 workers.
    let waits: Vec<XferPlaceholder> = result
        .root
        .collect_xfers()
        .into_iter()
        .filter(|x| x.role == XferRole::Wait && x.data == DataId(0))
        .collect();
    assert_eq!(waits.len(), 4, "one Wait per fan-out destination");

    // Each Wait's tile carries y-axis as ONLY partitioned axis with the
    // extended range.
    let expect: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::from([
        (WorkerId(1), 0..6),
        (WorkerId(2), 4..10),
        (WorkerId(3), 8..13),
        (WorkerId(4), 11..16),
    ]);
    for w in &waits {
        let bounds = &w.tile.bounds;
        assert_eq!(bounds.len(), 1, "only y is partitioned → 1 axis");
        let (iv, range) = &bounds[0];
        assert_eq!(*iv, IterVar(7));
        let want = expect
            .get(&w.dst)
            .unwrap_or_else(|| panic!("unexpected dst worker {:?}", w.dst));
        assert_eq!(
            range, want,
            "worker {:?} Wait tile y-range should extend partition band by halo=1; \
             got {range:?}, expected {want:?}",
            w.dst
        );
    }
}

/// Empty halo_widths sidecar → no-op. The Wait tiles match the
/// partition bands verbatim (no halo extension), i.e. the pre-Stage-2
/// behaviour is preserved when the algo's kernels carry no halo.
#[test]
fn halo_empty_sidecar_is_identity() {
    let (acfg, linked) = build_stencil_like_acfg(/*halo_y=*/ 0, /*halo_x=*/ 0);
    assert!(
        acfg.halo_widths.is_empty(),
        "fixture must produce empty sidecar"
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
    let waits: Vec<XferPlaceholder> = result
        .root
        .collect_xfers()
        .into_iter()
        .filter(|x| x.role == XferRole::Wait && x.data == DataId(0))
        .collect();
    assert_eq!(waits.len(), 4);
    let expect: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::from([
        (WorkerId(1), 1..5),
        (WorkerId(2), 5..9),
        (WorkerId(3), 9..12),
        (WorkerId(4), 12..15),
    ]);
    for w in &waits {
        let bounds = &w.tile.bounds;
        assert_eq!(bounds.len(), 1);
        let (iv, range) = &bounds[0];
        assert_eq!(*iv, IterVar(7));
        assert_eq!(range, expect.get(&w.dst).expect("worker entry"));
    }
}

/// Symmetric halo on both axes for a hypothetical 2D-blocks2d
/// partition. Verifies that ALL axes in the tile get extended, not just
/// the first. This is forward-compatible with TASK-0259 partition=blocks2d
/// when block-pair recovery (TASK-0264) lands.
#[test]
fn halo_extends_multiple_axes_when_both_partitioned() {
    // Build a synthetic 2D-partitioned ACFG: both y and x partitioned
    // across 2x2 = 4 workers. Halo=1 on both axes.
    let body_op = op(&[1, 2, 3, 4], 100, vec![0], Some(1));
    let inner = ACFGNode::Repeat {
        iter_var: IterVar(8), // x
        range: 0..8,
        body: Box::new(ACFGNode::Sequence(vec![body_op])),
        block_tag: None,
    };
    let outer = ACFGNode::Repeat {
        iter_var: IterVar(7), // y
        range: 0..8,
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
    };
    let root = ACFGNode::Sequence(vec![op(&[0], 99, vec![], Some(0)), outer]);
    let mut acfg = synthetic_acfg(
        root,
        &[("d", 0), ("c", 1)],
        &[("host", 0), ("w1", 1), ("w2", 2), ("w3", 3), ("w4", 4)],
    );
    acfg.name_iter_vars.insert("y".to_string(), IterVar(7));
    acfg.name_iter_vars.insert("x".to_string(), IterVar(8));
    acfg.name_kernels.insert("blur3".to_string(), KernelId(100));
    // 2x2 partition: w1=(0..4, 0..4), w2=(0..4, 4..8), w3=(4..8, 0..4),
    // w4=(4..8, 4..8). Use partition_worker_ranges per axis (the
    // pre-rewrite-partition-tiles map keys per IterVar).
    let mut y_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    y_bands.insert(WorkerId(1), 0..4);
    y_bands.insert(WorkerId(2), 0..4);
    y_bands.insert(WorkerId(3), 4..8);
    y_bands.insert(WorkerId(4), 4..8);
    let mut x_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    x_bands.insert(WorkerId(1), 0..4);
    x_bands.insert(WorkerId(2), 4..8);
    x_bands.insert(WorkerId(3), 0..4);
    x_bands.insert(WorkerId(4), 4..8);
    acfg.partition_worker_ranges.insert(IterVar(7), y_bands);
    acfg.partition_worker_ranges.insert(IterVar(8), x_bands);

    let mut blur3_halo: BTreeMap<IterVar, u64> = BTreeMap::new();
    blur3_halo.insert(IterVar(7), 1);
    blur3_halo.insert(IterVar(8), 1);
    acfg.halo_widths.insert(KernelId(100), blur3_halo);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[("d", &["host"])],
        "transfer d : sync;",
    );
    let result = inject_transfers(&linked, acfg).expect("inject_transfers");
    let waits: Vec<XferPlaceholder> = result
        .root
        .collect_xfers()
        .into_iter()
        .filter(|x| x.role == XferRole::Wait && x.data == DataId(0))
        .collect();
    assert_eq!(waits.len(), 4);
    // w1's tile: y in 0..4 → -1..5 clamped to source 0..8+halo (-1..9) → -1..5;
    // x in 0..4 → -1..5 clamped to -1..9 → -1..5. (The negative bound
    // is a halo-into-margin; the kernel handles the boundary.)
    let w1 = waits.iter().find(|w| w.dst == WorkerId(1)).unwrap();
    let bounds: BTreeMap<IterVar, std::ops::Range<i64>> = w1
        .tile
        .bounds
        .iter()
        .map(|(iv, r)| (*iv, r.clone()))
        .collect();
    assert_eq!(bounds.get(&IterVar(7)), Some(&(-1..5)));
    assert_eq!(bounds.get(&IterVar(8)), Some(&(-1..5)));
    // w4 (the diagonally-opposite corner): y in 4..8 → 3..9; x in
    // 4..8 → 3..9.
    let w4 = waits.iter().find(|w| w.dst == WorkerId(4)).unwrap();
    let bounds: BTreeMap<IterVar, std::ops::Range<i64>> = w4
        .tile
        .bounds
        .iter()
        .map(|(iv, r)| (*iv, r.clone()))
        .collect();
    assert_eq!(bounds.get(&IterVar(7)), Some(&(3..9)));
    assert_eq!(bounds.get(&IterVar(8)), Some(&(3..9)));
}

// --------------------------------------------------------------------
// TASK-0324 cycle-144 AC#5 — silent-elision-risk validator fixtures
// --------------------------------------------------------------------
//
// Two end-to-end fixtures pin the AC#2 fail-loud guard:
//
// - `task0324_ac5_positive_fires_on_06_distributed2_shape`: the
//   06-separable-filter/distributed2 shape — pass-1 producer and
//   pass-2 consumer both on {w0..w3}, both with their own
//   partition=rows iv on axis 0 of the data, but reader-iv != writer-
//   iv. Validator MUST return Err(SameSetSilentElisionRisk).
//
// - `task0324_ac5_negative_does_not_fire_on_13_cnn_batch_parallel_shape`:
//   the 13-cnn-inference/batch_parallel shape — producer writes
//   `feat1[n]` on {w0..w3} with `loop n : partition=workers`,
//   consumer reads `feat1[n]` on the same set with the same iv.
//   Validator MUST succeed (no error).

#[test]
fn task0324_ac5_positive_fires_on_06_distributed2_shape() {
    use nucleus_compiler::acfg::{DataAccess, DataflowDag, DataflowEdge, Operation};
    use nucleus_compiler::algo::ir::IrExpr;
    use nucleus_compiler::event::ArgBinding;
    use nucleus_compiler::passes::transfer_inject::TransferInjectError;

    // IterVars for the two passes. Non-monotonic ids so a regression
    // that depends on iter-var ordering is independently visible.
    const IV_HY: IterVar = IterVar(11);
    const IV_HX: IterVar = IterVar(12);
    const IV_VY: IterVar = IterVar(13);
    const IV_VX: IterVar = IterVar(14);
    const IV_VM: IterVar = IterVar(15);
    // DataIds.
    const D_IN_ARR: DataId = DataId(0);
    const D_TMP: DataId = DataId(1);
    const D_OUT: DataId = DataId(2);

    fn ident(name: &str) -> IrExpr {
        IrExpr::Ident(name.to_string())
    }
    fn access(data: DataId, ivs: &[&str]) -> DataAccess {
        DataAccess {
            data,
            indices: ivs.iter().map(|n| ident(n)).collect(),
        }
    }

    // Pass-1 op: hblur_acc reads in_arr[hy][hx] (whole, not the
    // hk-summing inner that the real algorithm uses; the validator's
    // discriminator is index-pattern-driven, so a minimal model is
    // sufficient). Writes tmp[hy][hx]. Placed on {w0..w3}.
    let hblur_edge = DataflowEdge {
        data_in: vec![D_IN_ARR],
        kernel: KernelId(200),
        data_out: Some(D_TMP),
        data_in_access: vec![access(D_IN_ARR, &["hy", "hx"])],
        data_out_access: Some(access(D_TMP, &["hy", "hx"])),
        args: vec![ArgBinding::Data(access(D_IN_ARR, &["hy", "hx"]))],
    };
    let hblur_op = ACFGNode::Operation(Operation {
        kernel: KernelId(200),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![hblur_edge] },
    });

    // Pass-2 op: vblur_acc reads tmp[vm][vx] (the offending non-
    // partition-iv read on axis 0). Writes out[vy][vx]. Placed on
    // {w0..w3}.
    let vblur_edge = DataflowEdge {
        data_in: vec![D_TMP],
        kernel: KernelId(201),
        data_out: Some(D_OUT),
        data_in_access: vec![access(D_TMP, &["vm", "vx"])],
        data_out_access: Some(access(D_OUT, &["vy", "vx"])),
        args: vec![ArgBinding::Data(access(D_TMP, &["vm", "vx"]))],
    };
    let vblur_op = ACFGNode::Operation(Operation {
        kernel: KernelId(201),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![vblur_edge] },
    });

    // Pass-1 nest: for hy : 0..16 { for hx : 0..16 { hblur_op } }
    let pass1 = ACFGNode::Repeat {
        iter_var: IV_HY,
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![ACFGNode::Repeat {
            iter_var: IV_HX,
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![hblur_op])),
            block_tag: None,
        }])),
        block_tag: None,
    };
    // Pass-2 nest: for vy : 0..16 { for vx : 0..16 { for vm : 0..16 { vblur_op } } }
    let pass2 = ACFGNode::Repeat {
        iter_var: IV_VY,
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![ACFGNode::Repeat {
            iter_var: IV_VX,
            range: 0..16,
            body: Box::new(ACFGNode::Sequence(vec![ACFGNode::Repeat {
                iter_var: IV_VM,
                range: 0..16,
                body: Box::new(ACFGNode::Sequence(vec![vblur_op])),
                block_tag: None,
            }])),
            block_tag: None,
        }])),
        block_tag: None,
    };

    // Host loader for in_arr, host saver for out.
    fn host_loader(data_out: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![],
            kernel: KernelId(99),
            data_out: Some(data_out),
            data_in_access: vec![],
            data_out_access: Some(DataAccess {
                data: data_out,
                indices: vec![],
            }),
            args: vec![],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(99),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }
    fn host_saver(data_in: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![data_in],
            kernel: KernelId(98),
            data_out: None,
            data_in_access: vec![DataAccess {
                data: data_in,
                indices: vec![],
            }],
            data_out_access: None,
            args: vec![ArgBinding::Data(DataAccess {
                data: data_in,
                indices: vec![],
            })],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(98),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }

    let root = ACFGNode::Sequence(vec![
        host_loader(D_IN_ARR),
        pass1,
        pass2,
        host_saver(D_OUT),
    ]);

    let mut acfg = synthetic_acfg(
        root,
        &[("in_arr", 0), ("tmp", 1), ("out", 2)],
        &[
            ("host", 0),
            ("w0", 1),
            ("w1", 2),
            ("w2", 3),
            ("w3", 4),
        ],
    );
    acfg.name_iter_vars.insert("hy".to_string(), IV_HY);
    acfg.name_iter_vars.insert("hx".to_string(), IV_HX);
    acfg.name_iter_vars.insert("vy".to_string(), IV_VY);
    acfg.name_iter_vars.insert("vx".to_string(), IV_VX);
    acfg.name_iter_vars.insert("vm".to_string(), IV_VM);

    // Partition hy AND vy across {w0..w3} (matches the schedule's
    // `loop hy : partition=rows; loop vy : partition=rows`).
    let bands_4 = || {
        let mut b: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
        b.insert(WorkerId(1), 0..4);
        b.insert(WorkerId(2), 4..8);
        b.insert(WorkerId(3), 8..12);
        b.insert(WorkerId(4), 12..16);
        b
    };
    acfg.partition_worker_ranges.insert(IV_HY, bands_4());
    acfg.partition_worker_ranges.insert(IV_VY, bands_4());

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        // tmp produced by pass-1 on workers; in_arr loaded on host;
        // out written by pass-2 on workers, saved on host.
        &[
            ("in_arr", &["host"]),
            ("tmp", &["w0", "w1", "w2", "w3"]),
            ("out", &["w0", "w1", "w2", "w3"]),
        ],
        "transfer in_arr : sync; transfer tmp : sync; transfer out : sync;",
    );

    let result = inject_transfers(&linked, acfg);
    match result {
        Err(TransferInjectError::SameSetSilentElisionRisk { data, message }) => {
            assert_eq!(
                data, D_TMP,
                "expected the rejection to name `tmp` (the cross-pass data)"
            );
            assert!(
                message.contains("TASK-0324"),
                "ContractGap message must forward-link TASK-0324; got: {message}"
            );
            assert!(
                message.contains("partition-sliced"),
                "ContractGap message must name the per-axis discrimination reason; \
                 got: {message}"
            );
        }
        Ok(_) => panic!(
            "expected TransferInjectError::SameSetSilentElisionRisk on the \
             06-separable-filter/distributed2 shape (producer-set == consumer-set \
             + reader iv `vm` ≠ partition iv on axis 0)"
        ),
        // TransferInjectError is `#[non_exhaustive]` (reviewer P2.2
        // cycle-144 fold-back): future variants land without breaking
        // this match. Any new variant firing on the AC#5 positive
        // fixture is itself a regression worth surfacing — panic with
        // the discovered shape.
        #[allow(unreachable_patterns)]
        Err(other) => panic!(
            "expected SameSetSilentElisionRisk on the 06/distributed2 shape; \
             got a different TransferInjectError variant: {other:?}"
        ),
    }
}

#[test]
fn task0324_ac5_negative_does_not_fire_on_13_cnn_batch_parallel_shape() {
    use nucleus_compiler::acfg::{DataAccess, DataflowDag, DataflowEdge, Operation};
    use nucleus_compiler::algo::ir::IrExpr;
    use nucleus_compiler::event::ArgBinding;

    // IterVar for the batch loop.
    const IV_N: IterVar = IterVar(21);
    // DataIds.
    const D_INPUT: DataId = DataId(0);
    const D_FEAT1: DataId = DataId(1);
    const D_FEAT2: DataId = DataId(2);
    const D_OUTPUT: DataId = DataId(3);

    fn ident(name: &str) -> IrExpr {
        IrExpr::Ident(name.to_string())
    }
    fn access_n(data: DataId) -> DataAccess {
        DataAccess {
            data,
            indices: vec![ident("n")],
        }
    }

    // conv_block_1: reads input[n], writes feat1[n] on {w0..w3}.
    let cb1_edge = DataflowEdge {
        data_in: vec![D_INPUT],
        kernel: KernelId(300),
        data_out: Some(D_FEAT1),
        data_in_access: vec![access_n(D_INPUT)],
        data_out_access: Some(access_n(D_FEAT1)),
        args: vec![ArgBinding::Data(access_n(D_INPUT))],
    };
    let cb1_op = ACFGNode::Operation(Operation {
        kernel: KernelId(300),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![cb1_edge] },
    });
    // conv_block_2: reads feat1[n], writes feat2[n] on {w0..w3}.
    let cb2_edge = DataflowEdge {
        data_in: vec![D_FEAT1],
        kernel: KernelId(301),
        data_out: Some(D_FEAT2),
        data_in_access: vec![access_n(D_FEAT1)],
        data_out_access: Some(access_n(D_FEAT2)),
        args: vec![ArgBinding::Data(access_n(D_FEAT1))],
    };
    let cb2_op = ACFGNode::Operation(Operation {
        kernel: KernelId(301),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![cb2_edge] },
    });
    // classifier: reads feat2[n], writes output[n] on {w0..w3}.
    let cls_edge = DataflowEdge {
        data_in: vec![D_FEAT2],
        kernel: KernelId(302),
        data_out: Some(D_OUTPUT),
        data_in_access: vec![access_n(D_FEAT2)],
        data_out_access: Some(access_n(D_OUTPUT)),
        args: vec![ArgBinding::Data(access_n(D_FEAT2))],
    };
    let cls_op = ACFGNode::Operation(Operation {
        kernel: KernelId(302),
        workers: ws(&[1, 2, 3, 4]),
        dataflow: DataflowDag { edges: vec![cls_edge] },
    });

    let body = ACFGNode::Sequence(vec![cb1_op, cb2_op, cls_op]);
    let n_loop = ACFGNode::Repeat {
        iter_var: IV_N,
        range: 0..16,
        body: Box::new(body),
        block_tag: None,
    };

    fn host_loader(data_out: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![],
            kernel: KernelId(99),
            data_out: Some(data_out),
            data_in_access: vec![],
            data_out_access: Some(DataAccess {
                data: data_out,
                indices: vec![],
            }),
            args: vec![],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(99),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }
    fn host_saver(data_in: DataId) -> ACFGNode {
        let edge = DataflowEdge {
            data_in: vec![data_in],
            kernel: KernelId(98),
            data_out: None,
            data_in_access: vec![DataAccess {
                data: data_in,
                indices: vec![],
            }],
            data_out_access: None,
            args: vec![ArgBinding::Data(DataAccess {
                data: data_in,
                indices: vec![],
            })],
        };
        ACFGNode::Operation(Operation {
            kernel: KernelId(98),
            workers: ws(&[0]),
            dataflow: DataflowDag { edges: vec![edge] },
        })
    }

    let root = ACFGNode::Sequence(vec![
        host_loader(D_INPUT),
        n_loop,
        host_saver(D_OUTPUT),
    ]);

    let mut acfg = synthetic_acfg(
        root,
        &[("input", 0), ("feat1", 1), ("feat2", 2), ("output", 3)],
        &[
            ("host", 0),
            ("w0", 1),
            ("w1", 2),
            ("w2", 3),
            ("w3", 4),
        ],
    );
    acfg.name_iter_vars.insert("n".to_string(), IV_N);

    // Partition n across {w0..w3} (batch_parallel: `loop n : partition=workers`).
    let mut n_bands: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    n_bands.insert(WorkerId(1), 0..4);
    n_bands.insert(WorkerId(2), 4..8);
    n_bands.insert(WorkerId(3), 8..12);
    n_bands.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges.insert(IV_N, n_bands);

    let linked = synthetic_linked_ir(
        &acfg.name_data,
        &acfg.name_workers,
        &[
            ("input", &["host"]),
            ("feat1", &["w0", "w1", "w2", "w3"]),
            ("feat2", &["w0", "w1", "w2", "w3"]),
            ("output", &["w0", "w1", "w2", "w3"]),
        ],
        "transfer input : sync; transfer feat1 : sync; transfer feat2 : sync; \
         transfer output : sync;",
    );

    // Validator MUST succeed: every shared-set producer/consumer pair
    // (feat1 between cb1 and cb2, feat2 between cb2 and cls) has the
    // same partition iv `n` on the only data axis — the per-axis check
    // sees `P_0 == C_0 == Ident("n")`.
    let _result = inject_transfers(&linked, acfg)
        .expect("13-cnn-inference/batch_parallel shape MUST NOT trigger the AC#2 guard");
}
