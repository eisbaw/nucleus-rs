//! Integration tests for the ACFG -> Petri-net lowering pass
//! (TASK-0026, PRD §8.2).
//!
//! Strategy:
//!
//! - **Synthetic positive cases**: hand-built tiny ACFGs that exercise
//!   one mapping rule per test (single Operation, Push/Wait pair,
//!   Sync barrier, Repeat unrolling). Structural assertions on place
//!   and transition counts; no full-tree snapshots.
//!
//! - **Buffer capacity**: a Push/Wait pair with `buffer=4` policy
//!   yields a buffer place with capacity 4.
//!
//! - **End-to-end**: build the ACFG for examples 01, 02, 03 (every
//!   schedule under each), run sync+transfer injection, then lower.
//!   Assert structural properties (every place capacity is non-zero,
//!   transition count matches the ACFG operation/sync/xfer total).
//!
//! - **Determinism**: lowering the same ACFG twice produces equal
//!   nets (structural equality on places/transitions/arcs).
//!
//! What this file does NOT cover:
//! - Net firing semantics. Tests for that live in `tests/petri.rs`.
//! - Boundedness or deadlock analyses. TASK-0028/0029.
//! - Per-worker EventList projection. TASK-0027.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{
    ACFGNode, DataflowDag, DataflowEdge, NotifyMode, Operation, SyncPlaceholder, TransferPolicy,
    XferPlaceholder, XferRole, ACFG,
};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{DataId, IterTile, KernelId, SeqTag, WorkerId};
use nucleus_compiler::link;
use nucleus_compiler::passes::acfg_to_petri::acfg_to_net;
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::petri::{ArcKind, Net};
use nucleus_compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Synthetic-ACFG helpers
// --------------------------------------------------------------------

fn ws(ids: &[u64]) -> BTreeSet<WorkerId> {
    ids.iter().copied().map(WorkerId).collect()
}

fn op_node(workers: &[u64], kernel: u64, data_in: Vec<u64>, data_out: Option<u64>) -> ACFGNode {
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

/// Count how many distinct transitions in `net` are owned by `worker`.
fn transitions_owned_by(net: &Net, worker: WorkerId) -> usize {
    net.transitions
        .iter()
        .filter(|t| t.worker == Some(worker))
        .count()
}

/// True iff every place in `net` has Some(capacity) — i.e. no
/// unbounded analysis-only places leaked into a production net.
fn all_places_bounded(net: &Net) -> bool {
    net.places.iter().all(|p| p.capacity.is_some())
}

/// True iff every transition has at least one incoming arc and at
/// least one outgoing arc — a transition with no inputs or no outputs
/// would be an unconnected node, almost always a builder bug.
fn all_transitions_connected(net: &Net) -> bool {
    net.transitions.iter().all(|t| {
        let has_in = net
            .arcs
            .iter()
            .any(|a| a.transition == t.id && a.kind == ArcKind::PtoT);
        let has_out = net
            .arcs
            .iter()
            .any(|a| a.transition == t.id && a.kind == ArcKind::TtoP);
        has_in && has_out
    })
}

// --------------------------------------------------------------------
// Synthetic case 1: single worker, single Operation
// --------------------------------------------------------------------

#[test]
fn single_worker_single_op_yields_one_transition() {
    // Sequence { op on w0 }. Expect:
    //   - exactly 1 transition (the op)
    //   - 2 control places (start + after)
    //   - 2 arcs (one PtoT from start, one TtoP to after)
    //   - no buffer places (no transfers)
    let root = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);
    let net = acfg_to_net(&acfg);

    assert_eq!(net.transitions.len(), 1, "exactly one Operation transition");
    assert_eq!(net.places.len(), 2, "two control places (before, after)");
    assert_eq!(net.arcs.len(), 2, "one PtoT + one TtoP");
    assert!(all_places_bounded(&net));
    assert!(all_transitions_connected(&net));
    assert_eq!(transitions_owned_by(&net, WorkerId(0)), 1);

    // Start place has initial_marking = 1; second place starts at 0.
    let initial = &net.initial_marking;
    let mut marked: Vec<u32> = net.places.iter().map(|p| initial.get(p.id)).collect();
    marked.sort();
    assert_eq!(marked, vec![0, 1], "exactly one start token across places");
}

// --------------------------------------------------------------------
// Synthetic case 2: two workers, one Push/Wait pair
// --------------------------------------------------------------------

#[test]
fn two_worker_xfer_pair_yields_intermediate_place_and_two_transitions() {
    // op_p on w0 -> Push (seq=0) -> Wait (seq=0) -> op_c on w1.
    // We build the Xfer nodes by hand to keep this test independent
    // from the transfer-injection pass.
    let tile = IterTile::empty();
    let policy = TransferPolicy {
        synchronous: true,
        buffer: 1,
        notify: NotifyMode::Default,
        // TASK-0438.01: transport is incidental here; default Pio.
        ..TransferPolicy::default()
    };
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(0),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(0),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);
    let net = acfg_to_net(&acfg);

    // Transitions: op_p, push, wait, op_c = 4 transitions total.
    assert_eq!(net.transitions.len(), 4);

    // Two of them owned by each worker.
    assert_eq!(
        transitions_owned_by(&net, WorkerId(0)),
        2,
        "w0 owns op + push"
    );
    assert_eq!(
        transitions_owned_by(&net, WorkerId(1)),
        2,
        "w1 owns wait + op"
    );

    // Places: 1 start per worker = 2; one "after" per transition for
    // its threading worker = 4 (op_p produces ctl_w0_1; push produces
    // ctl_w0_2; wait produces ctl_w1_1; op_c produces ctl_w1_2);
    // plus one buffer place = 7 total.
    assert_eq!(net.places.len(), 7);

    // Exactly one buffer place. We detect it by name prefix.
    let buf_places: Vec<_> = net
        .places
        .iter()
        .filter(|p| p.name.starts_with("buf_"))
        .collect();
    assert_eq!(buf_places.len(), 1, "exactly one buffer place");

    // Buffer capacity matches policy.buffer = 1.
    assert_eq!(buf_places[0].capacity.unwrap().get(), 1);

    // Buffer place has one incoming arc (from Push) and one outgoing
    // arc (to Wait).
    let bid = buf_places[0].id;
    let arcs_to_buf: Vec<_> = net
        .arcs
        .iter()
        .filter(|a| a.place == bid && a.kind == ArcKind::TtoP)
        .collect();
    let arcs_from_buf: Vec<_> = net
        .arcs
        .iter()
        .filter(|a| a.place == bid && a.kind == ArcKind::PtoT)
        .collect();
    assert_eq!(arcs_to_buf.len(), 1, "Push deposits into buffer");
    assert_eq!(arcs_from_buf.len(), 1, "Wait consumes from buffer");

    assert!(all_places_bounded(&net));
    assert!(all_transitions_connected(&net));
}

// --------------------------------------------------------------------
// Synthetic case 3: Sync barrier
// --------------------------------------------------------------------

#[test]
fn sync_barrier_spans_all_participants() {
    // op_a on w0 ; sync(w0,w1) ; op_b on w1.
    // The sync transition must consume one token from each participant's
    // current control place and produce one for each.
    let mut participants = BTreeSet::new();
    participants.insert(WorkerId(0));
    participants.insert(WorkerId(1));
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        ACFGNode::Sync(SyncPlaceholder {
            participants,
            ..Default::default()
        }),
        op_node(&[1], 101, vec![], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);
    let net = acfg_to_net(&acfg);

    // 3 transitions: op_a, sync, op_b.
    assert_eq!(net.transitions.len(), 3);

    // Sync transition has arcs to/from both workers' control chains.
    let sync_t = net
        .transitions
        .iter()
        .find(|t| t.name.starts_with("sync_"))
        .expect("sync transition present");
    let arcs_in: Vec<_> = net
        .arcs
        .iter()
        .filter(|a| a.transition == sync_t.id && a.kind == ArcKind::PtoT)
        .collect();
    let arcs_out: Vec<_> = net
        .arcs
        .iter()
        .filter(|a| a.transition == sync_t.id && a.kind == ArcKind::TtoP)
        .collect();
    assert_eq!(
        arcs_in.len(),
        2,
        "sync consumes one token from each participant"
    );
    assert_eq!(
        arcs_out.len(),
        2,
        "sync produces one token for each participant"
    );

    assert!(all_places_bounded(&net));
    assert!(all_transitions_connected(&net));
}

// --------------------------------------------------------------------
// Synthetic case 4: Repeat unrolls
// --------------------------------------------------------------------

#[test]
fn repeat_unrolls_body() {
    // for i : 0..3 { op on w0 }  ==> 3 op transitions.
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: nucleus_compiler::event::IterVar(0),
        range: 0..3,
        body: Box::new(body),
        block_tag: None,
        break_cond: None,
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);
    let net = acfg_to_net(&acfg);

    assert_eq!(
        net.transitions.len(),
        3,
        "Repeat range length 3 -> 3 firings"
    );
    assert_eq!(transitions_owned_by(&net, WorkerId(0)), 3);
}

#[test]
fn repeat_empty_range_emits_no_transitions() {
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: nucleus_compiler::event::IterVar(0),
        range: 5..5,
        body: Box::new(body),
        block_tag: None,
        break_cond: None,
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);
    let net = acfg_to_net(&acfg);

    // No transitions; only the seeded start place.
    assert_eq!(net.transitions.len(), 0);
    assert_eq!(net.places.len(), 1);
}

// --------------------------------------------------------------------
// Buffer capacity follows TransferPolicy.buffer
// --------------------------------------------------------------------

#[test]
fn buffer_capacity_follows_policy_buffer_field() {
    let tile = IterTile::empty();
    let policy = TransferPolicy {
        synchronous: false,
        buffer: 4,
        notify: NotifyMode::Event,
        // TASK-0438.01: transport is incidental here; default Pio.
        ..TransferPolicy::default()
    };
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(7),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(7),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);
    let net = acfg_to_net(&acfg);

    let buf = net
        .places
        .iter()
        .find(|p| p.name.starts_with("buf_"))
        .expect("buffer place present");
    assert_eq!(buf.capacity.unwrap().get(), 4);
}

// --------------------------------------------------------------------
// Determinism
// --------------------------------------------------------------------

/// Two `Net`s compare structurally equal if their places, transitions,
/// arcs, and initial markings match element-wise. The crate provides
/// `Clone` on `Net` but no `PartialEq` (yet — the petri module
/// derives `PartialEq` on the leaf types but not on the container).
/// We compare field-by-field.
fn nets_structurally_equal(a: &Net, b: &Net) -> bool {
    a.places == b.places
        && a.transitions == b.transitions
        && a.arcs == b.arcs
        && a.initial_marking == b.initial_marking
}

#[test]
fn determinism_two_lowerings_of_same_acfg_match() {
    // Mixed sequence: op + push/wait + sync + repeat.
    let tile = IterTile::empty();
    let policy = TransferPolicy::default();
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(0),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(0),
        policy,
    });
    let mut participants = BTreeSet::new();
    participants.insert(WorkerId(0));
    participants.insert(WorkerId(1));
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0)), push, wait]);
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: nucleus_compiler::event::IterVar(0),
            range: 0..2,
            body: Box::new(body),
            block_tag: None,
            break_cond: None,
        },
        ACFGNode::Sync(SyncPlaceholder {
            participants,
            ..Default::default()
        }),
        op_node(&[1], 101, vec![0], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let net_a = acfg_to_net(&acfg);
    let net_b = acfg_to_net(&acfg);

    assert!(
        nets_structurally_equal(&net_a, &net_b),
        "two lowerings of the same ACFG must produce equal nets"
    );
}

// --------------------------------------------------------------------
// End-to-end on real examples
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

fn pipeline_to_net(algo_rel: &str, sched_rel: &str) -> Net {
    let algo_ast = parse_algo(&read_example(algo_rel)).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(sched_rel)).expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    acfg_to_net(&acfg)
}

#[test]
fn e2e_example_01_naive() {
    // Single-worker schedule. Expect no buffer places, no sync.
    let net = pipeline_to_net(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    assert!(!net.transitions.is_empty(), "non-empty program");
    assert!(all_places_bounded(&net), "every place bounded");
    assert!(all_transitions_connected(&net));
    let buf_count = net
        .places
        .iter()
        .filter(|p| p.name.starts_with("buf_"))
        .count();
    assert_eq!(buf_count, 0, "single-worker -> no transfer buffers");
}

#[test]
fn e2e_example_02_split() {
    // Two-worker schedule with cross-worker transfers. Expect at least
    // one buffer place, and at least as many transitions as there
    // are Push/Wait pairs + Operations.
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    assert!(!net.transitions.is_empty());
    assert!(all_places_bounded(&net));
    assert!(all_transitions_connected(&net));
    let buf_count = net
        .places
        .iter()
        .filter(|p| p.name.starts_with("buf_"))
        .count();
    assert!(
        buf_count >= 1,
        "split schedule produces at least one buffer place, got {}",
        buf_count
    );
}

#[test]
fn e2e_example_02_naive() {
    // Single-worker schedule version of example 2: no cross-worker
    // transfers expected.
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/naive.sched.nuc",
    );
    assert!(!net.transitions.is_empty());
    assert!(all_places_bounded(&net));
    let buf_count = net
        .places
        .iter()
        .filter(|p| p.name.starts_with("buf_"))
        .count();
    assert_eq!(buf_count, 0);
}

#[test]
fn e2e_example_03_naive() {
    // Reduction on a single worker (naive schedule).
    let net = pipeline_to_net(
        "03-reduction/prog.algo.nuc",
        "03-reduction/schedules/naive.sched.nuc",
    );
    assert!(!net.transitions.is_empty());
    assert!(all_places_bounded(&net));
    assert!(all_transitions_connected(&net));
}

#[test]
fn e2e_determinism_real_example_02() {
    // The end-to-end pipeline is deterministic — lowering twice yields
    // identical nets.
    let net_a = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    let net_b = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    assert!(nets_structurally_equal(&net_a, &net_b));
}

// --------------------------------------------------------------------
// TASK-0134: pipeline=D -> buffer place initial_marking = D
// --------------------------------------------------------------------

#[test]
fn synthetic_pipeline_depth_sets_initial_marking() {
    // Hand-built: a Push/Wait pair whose seq is annotated with
    // pipeline depth 3 on the ACFG sidecar. The buffer place should
    // start with 3 tokens, not 0.
    let tile = IterTile::empty();
    let policy = TransferPolicy {
        synchronous: false,
        buffer: 4, // capacity high enough to hold the head-start
        notify: NotifyMode::Default,
        // TASK-0438.01: transport is incidental here; default Pio.
        ..TransferPolicy::default()
    };
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(7),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(7),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], Some(1)),
    ]);
    let mut acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);
    acfg.pipeline_depth_for_seq
        .insert(SeqTag(7), std::num::NonZeroU64::new(3).expect("3 != 0"));

    let net = acfg_to_net(&acfg);

    let buf = net
        .places
        .iter()
        .find(|p| p.name.starts_with("buf_"))
        .expect("buffer place present");
    assert_eq!(
        buf.initial_marking, 3,
        "pipeline_depth_for_seq[seq=7]=3 must set initial_marking=3"
    );
    // The Net::initial_marking aggregate also reports this:
    assert_eq!(
        net.initial_marking.get(buf.id),
        3,
        "Net.initial_marking reflects the place's initial_marking"
    );
    // Capacity is still buffer=4 from the policy.
    assert_eq!(buf.capacity.unwrap().get(), 4);
}

#[test]
fn synthetic_no_pipeline_depth_keeps_initial_marking_zero() {
    // Regression guard: with NO sidecar entry, the buffer place must
    // start at 0 tokens (pre-TASK-0134 behaviour preserved).
    let tile = IterTile::empty();
    let policy = TransferPolicy {
        synchronous: true,
        buffer: 1,
        notify: NotifyMode::Default,
        // TASK-0438.01: transport is incidental here; default Pio.
        ..TransferPolicy::default()
    };
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(0),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(0),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);
    // No pipeline_depth_for_seq insert.

    let net = acfg_to_net(&acfg);
    let buf = net
        .places
        .iter()
        .find(|p| p.name.starts_with("buf_"))
        .expect("buffer place present");
    assert_eq!(buf.initial_marking, 0);
}

#[test]
fn e2e_example_13_pipeline_parallel_sets_buffer_initial_markings() {
    // Real fixture: example 13 with pipeline_parallel schedule.
    // `loop n : pipeline=3`; the inter-stage transfers (feat1, feat2)
    // have both producer AND consumer inside the loop, so their
    // buffer places should pick up initial_marking = 3.
    let net = pipeline_to_net(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );

    // Collect every buffer place by name -> initial_marking.
    let mut buf_markings: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    for p in &net.places {
        if p.name.starts_with("buf_") {
            buf_markings.insert(p.name.clone(), p.initial_marking);
        }
    }

    // We don't pin exact seq numbers (they are an internal allocation
    // detail) — we filter by data-symbol name in the human-readable
    // place name (`buf_<data>_seq<N>`).
    let feat1_markings: Vec<u32> = buf_markings
        .iter()
        .filter(|(n, _)| n.starts_with("buf_feat1_"))
        .map(|(_, m)| *m)
        .collect();
    let feat2_markings: Vec<u32> = buf_markings
        .iter()
        .filter(|(n, _)| n.starts_with("buf_feat2_"))
        .map(|(_, m)| *m)
        .collect();
    assert!(
        !feat1_markings.is_empty(),
        "expected at least one buf_feat1_* place: got {:?}",
        buf_markings
    );
    assert!(
        feat1_markings.iter().all(|m| *m == 3),
        "every buf_feat1_* place must have initial_marking = 3 (pipeline=3); got {:?}",
        feat1_markings
    );
    assert!(
        feat2_markings.iter().all(|m| *m == 3),
        "every buf_feat2_* place must have initial_marking = 3; got {:?}",
        feat2_markings
    );

    // `output` is produced inside the loop but consumed outside
    // (save_output runs after the loop). Its Push/Wait pair is
    // hoisted out by hoist_invariant_waits / splice_pushes_global,
    // so the tile no longer contains `n` and pipeline depth does
    // NOT apply.
    for (name, marking) in &buf_markings {
        if name.starts_with("buf_output_") {
            assert_eq!(
                *marking, 0,
                "output transfer is hoisted out of the pipelined loop; initial_marking must stay 0 (got {})",
                marking
            );
        }
    }
}

#[test]
fn e2e_example_13_pipeline_parallel_is_deterministic() {
    // TASK-0134 AC#5: build the pipelined net twice; structurally
    // identical including initial markings.
    let net_a = pipeline_to_net(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );
    let net_b = pipeline_to_net(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );
    assert!(nets_structurally_equal(&net_a, &net_b));
}

#[test]
fn e2e_example_13_pipeline_parallel_passes_boundedness_and_deadlock() {
    // TASK-0218 AC#2: with `sync_inject` no longer interposing a
    // barrier between a Push and its matching Wait (the bare-Operation
    // Sequence-rule case), the structural dependency cycle that
    // forced TASK-0213's path-2 elision is gone. Boundedness on this
    // fixture is now resolved by path-1 alone — the marking-aware
    // firing-order reorder in `derive_firing_order`. The path-2
    // TtoP-arc elision in `acfg_to_petri::emit_xfer` has been
    // reverted (every Push deposits a real token, in lockstep with
    // the runtime token trace).
    //
    // Both boundedness and deadlock-freedom hold under path-1 alone.
    let net = pipeline_to_net(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );
    let firing_order = nucleus_compiler::passes::boundedness::derive_firing_order(&net);
    nucleus_compiler::passes::boundedness::check_bounded(&net, &firing_order)
        .expect("boundedness must hold");
    nucleus_compiler::passes::deadlock::check_deadlock_free(&net, &firing_order)
        .expect("deadlock-free must hold");
}
