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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let r1 = inject_transfers(&linked, mk_acfg()).root.collect_xfers();
    let r2 = inject_transfers(&linked, mk_acfg()).root.collect_xfers();
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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let acfg = inject_transfers(&linked, acfg);

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
    let acfg = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);

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
    let result = inject_transfers(&linked, acfg);
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
    let result = inject_transfers(&linked, acfg);
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
