//! Integration tests for the per-worker EventList projection pass
//! (TASK-0027, PRD §8.1 and §8.3).
//!
//! Strategy mirrors `tests/acfg_to_petri.rs`:
//!
//! - **Synthetic positive cases**: hand-built tiny ACFGs targeting one
//!   variant per test (single Operation, Push/Wait pair, Sync, Repeat
//!   unrolling).
//! - **End-to-end**: build the ACFG for example 02-split-add under
//!   `split.sched.nuc`, run sync+transfer injection, lower to Petri
//!   net, then project to per-worker `EventList`s. Assert: one
//!   EventList per declared worker; bit-identical between two runs.
//!
//! What this file does NOT cover:
//! - The petri-net firing semantics (lives in `tests/petri.rs`).
//! - Capability validation. Out-of-scope.

use std::collections::{BTreeMap, BTreeSet};

use compiler::acfg::{
    ACFGNode, DataflowDag, DataflowEdge, NotifyMode, Operation, SyncPlaceholder, TransferPolicy,
    XferPlaceholder, XferRole, ACFG,
};
use compiler::algo::{lower_algo, parse_algo};
use compiler::event::{DataId, Event, IterTile, KernelId, SeqTag, SyncKind, WorkerId};
use compiler::link;
use compiler::passes::acfg_to_petri::acfg_to_net;
use compiler::passes::petri_to_events::{acfg_to_events, petri_to_events};
use compiler::passes::sync_inject::inject_syncs;
use compiler::passes::transfer_inject::inject_transfers;
use compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Synthetic-ACFG helpers (copied from tests/acfg_to_petri.rs so the
// two test files stay independent of each other).
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
    }
}

// --------------------------------------------------------------------
// Synthetic case 1: single worker, single Operation
// --------------------------------------------------------------------

#[test]
fn single_worker_single_op_emits_one_fire() {
    let root = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);

    let events = acfg_to_events(&acfg);

    assert_eq!(events.len(), 1, "one worker -> one EventList");
    let list = events.get(&WorkerId(0)).expect("w0 has a list");
    assert_eq!(list.len(), 1);
    match &list[0] {
        Event::Fire { kernel, tile } => {
            assert_eq!(*kernel, KernelId(100));
            assert!(tile.is_empty(), "M2: tile empty for unrolled ops");
        }
        other => panic!("expected Fire, got {:?}", other),
    }
}

#[test]
fn declared_workers_appear_even_if_silent() {
    // w1 is declared in name_workers but never appears in any
    // Operation; the projection should still surface it with an
    // empty EventList so the backend can deterministically iterate.
    let root = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);

    assert_eq!(events.len(), 2);
    assert_eq!(events.get(&WorkerId(0)).unwrap().len(), 1);
    assert_eq!(events.get(&WorkerId(1)).unwrap().len(), 0, "w1 silent");
}

// --------------------------------------------------------------------
// Synthetic case 2: two workers, matched Push/Wait pair
// --------------------------------------------------------------------

#[test]
fn two_worker_push_wait_pair_routes_correctly_with_matching_seq() {
    let tile = IterTile::empty();
    let policy = TransferPolicy::default();
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(42),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(42),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);

    let w0 = events.get(&WorkerId(0)).expect("w0 present");
    let w1 = events.get(&WorkerId(1)).expect("w1 present");

    // w0: Fire(100), Push(seq=42).  No Wait.
    assert_eq!(w0.len(), 2);
    match &w0[0] {
        Event::Fire { kernel, .. } => assert_eq!(*kernel, KernelId(100)),
        e => panic!("w0[0] expected Fire, got {:?}", e),
    }
    let push_seq = match &w0[1] {
        Event::Push { dst, data, seq, .. } => {
            assert_eq!(*dst, WorkerId(1));
            assert_eq!(*data, DataId(0));
            *seq
        }
        e => panic!("w0[1] expected Push, got {:?}", e),
    };

    // w1: Wait(seq=42), Fire(101).  No Push.
    assert_eq!(w1.len(), 2);
    let wait_seq = match &w1[0] {
        Event::Wait { src, data, seq, .. } => {
            assert_eq!(*src, WorkerId(0));
            assert_eq!(*data, DataId(0));
            *seq
        }
        e => panic!("w1[0] expected Wait, got {:?}", e),
    };
    match &w1[1] {
        Event::Fire { kernel, .. } => assert_eq!(*kernel, KernelId(101)),
        e => panic!("w1[1] expected Fire, got {:?}", e),
    }

    assert_eq!(push_seq, wait_seq, "Push/Wait seq tags must match");
    assert_eq!(push_seq, SeqTag(42));

    // Each worker only carries its own endpoint of the pair.
    assert!(
        !w0.iter().any(|e| matches!(e, Event::Wait { .. })),
        "w0 must not see any Wait"
    );
    assert!(
        !w1.iter().any(|e| matches!(e, Event::Push { .. })),
        "w1 must not see any Push"
    );
}

// --------------------------------------------------------------------
// Synthetic case 3: Sync barrier
// --------------------------------------------------------------------

#[test]
fn sync_barrier_emitted_on_every_participant() {
    let mut participants = BTreeSet::new();
    participants.insert(WorkerId(0));
    participants.insert(WorkerId(1));
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        ACFGNode::Sync(SyncPlaceholder {
            participants: participants.clone(),
        }),
        op_node(&[1], 101, vec![], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);

    let w0 = events.get(&WorkerId(0)).unwrap();
    let w1 = events.get(&WorkerId(1)).unwrap();

    // w0: Fire(100), Sync({w0,w1}).
    assert_eq!(w0.len(), 2);
    match &w0[1] {
        Event::Sync {
            participants: ps,
            kind,
        } => {
            assert_eq!(*ps, participants);
            assert_eq!(*kind, SyncKind::Barrier);
        }
        e => panic!("w0[1] expected Sync, got {:?}", e),
    }
    // w1: Sync({w0,w1}), Fire(101).
    assert_eq!(w1.len(), 2);
    match &w1[0] {
        Event::Sync {
            participants: ps, ..
        } => assert_eq!(*ps, participants),
        e => panic!("w1[0] expected Sync, got {:?}", e),
    }
}

// --------------------------------------------------------------------
// Synthetic case 4: Repeat unrolls
// --------------------------------------------------------------------

#[test]
fn repeat_unrolls_in_event_list() {
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: compiler::event::IterVar(0),
        range: 0..3,
        body: Box::new(body),
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);

    let events = acfg_to_events(&acfg);
    let w0 = events.get(&WorkerId(0)).unwrap();
    assert_eq!(w0.len(), 3, "range 0..3 -> 3 fires");
    for ev in w0 {
        assert!(matches!(ev, Event::Fire { kernel, .. } if *kernel == KernelId(100)));
    }
}

#[test]
fn repeat_empty_range_emits_no_events() {
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: compiler::event::IterVar(0),
        range: 5..5,
        body: Box::new(body),
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);

    let events = acfg_to_events(&acfg);
    assert_eq!(events.get(&WorkerId(0)).unwrap().len(), 0);
}

// --------------------------------------------------------------------
// `petri_to_events` wrapper agrees with `acfg_to_events`
// --------------------------------------------------------------------

#[test]
fn petri_wrapper_agrees_with_acfg_entry_point() {
    let tile = IterTile::empty();
    let policy = TransferPolicy {
        synchronous: false,
        buffer: 4,
        notify: NotifyMode::Event,
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
    let via_acfg = acfg_to_events(&acfg);
    let via_petri = petri_to_events(&acfg, &net);
    assert_eq!(
        via_acfg, via_petri,
        "petri_to_events wrapper must agree with acfg_to_events"
    );
}

// --------------------------------------------------------------------
// Determinism on a mixed synthetic case
// --------------------------------------------------------------------

#[test]
fn determinism_two_projections_of_same_acfg_match() {
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
            iter_var: compiler::event::IterVar(0),
            range: 0..2,
            body: Box::new(body),
        },
        ACFGNode::Sync(SyncPlaceholder { participants }),
        op_node(&[1], 101, vec![0], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let a = acfg_to_events(&acfg);
    let b = acfg_to_events(&acfg);
    assert_eq!(a, b, "projection must be deterministic");
}

// --------------------------------------------------------------------
// End-to-end: example 02 (split) projected to EventLists
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

fn pipeline_to_events(
    algo_rel: &str,
    sched_rel: &str,
) -> (BTreeMap<WorkerId, Vec<Event>>, BTreeMap<String, WorkerId>) {
    let algo_ast = parse_algo(&read_example(algo_rel)).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(sched_rel)).expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = compiler::acfg::build_acfg(&linked);
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    let name_workers = acfg.name_workers.clone();
    // Route through `petri_to_events` so the wrapper is exercised on
    // a real input too.
    let net = acfg_to_net(&acfg);
    (petri_to_events(&acfg, &net), name_workers)
}

#[test]
fn e2e_example_02_split_one_eventlist_per_declared_worker() {
    let (events, names) = pipeline_to_events(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );

    // Schedule names exactly two workers: host, w0.
    assert_eq!(
        names.len(),
        2,
        "split.sched.nuc declares two workers; got {:?}",
        names.keys().collect::<Vec<_>>()
    );
    assert_eq!(events.len(), names.len(), "one EventList per worker");
    for wid in names.values() {
        assert!(events.contains_key(wid), "missing EventList for {:?}", wid);
    }

    // Both workers do *something*: host loads + saves, w0 fires the add.
    for (wid, list) in &events {
        assert!(
            !list.is_empty(),
            "worker {:?} has empty EventList in split schedule",
            wid
        );
    }

    // Every Push on some worker has a matching Wait on the
    // corresponding destination worker carrying the same seq + data.
    //
    // TASK-0136 / TASK-0139: `transfer_inject` now splices Pushes
    // across Sequence/Repeat scope boundaries (Pass A whole-symbol
    // hoist + Pass B global Push finaliser). For example 02 the
    // producer `load_input` lives at the top-level sequence while the
    // consumer `add` lives inside a `for` loop; the cross-scope
    // finaliser pairs them. We therefore now assert the *strong*
    // property: at least one Push is present, and every Push has a
    // matching Wait on its declared dst.
    let mut pushes: Vec<(WorkerId, &Event)> = Vec::new();
    let mut waits: Vec<(WorkerId, &Event)> = Vec::new();
    for (wid, list) in &events {
        for ev in list {
            match ev {
                Event::Push { .. } => pushes.push((*wid, ev)),
                Event::Wait { .. } => waits.push((*wid, ev)),
                _ => {}
            }
        }
    }
    // Waits should at least be present (consumer side of the split).
    assert!(
        !waits.is_empty(),
        "split schedule consumer should produce at least one Wait"
    );
    // TASK-0136 AC#2: Pushes must now be present (producer side of
    // the split — the cross-scope finaliser pairs every Wait).
    assert!(
        !pushes.is_empty(),
        "split schedule must produce at least one Push after \
         TASK-0136/0139 cross-scope finalisation"
    );
    for (push_owner, push) in &pushes {
        let (push_dst, push_data, push_seq) = match push {
            Event::Push { dst, data, seq, .. } => (*dst, *data, *seq),
            _ => unreachable!(),
        };
        let mate = waits.iter().find(|(wait_owner, wait)| {
            *wait_owner == push_dst
                && match wait {
                    Event::Wait { src, data, seq, .. } => {
                        *src == *push_owner && *data == push_data && *seq == push_seq
                    }
                    _ => false,
                }
        });
        assert!(
            mate.is_some(),
            "Push on {:?} (data={:?} seq={:?}) has no matching Wait on {:?}",
            push_owner,
            push_data,
            push_seq,
            push_dst
        );
    }
}

#[test]
fn e2e_example_02_split_determinism() {
    let (a, _) = pipeline_to_events(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    let (b, _) = pipeline_to_events(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    assert_eq!(
        a, b,
        "two full-pipeline projections of the same input must be bit-identical"
    );
}

#[test]
fn e2e_example_01_naive_single_worker_no_transfers() {
    // Sanity check on the easier example: naive schedule, one worker
    // declared, no Push/Wait/Sync should appear in the EventLists.
    let (events, names) = pipeline_to_events(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    assert_eq!(names.len(), events.len());
    for list in events.values() {
        for ev in list {
            assert!(
                !matches!(ev, Event::Push { .. } | Event::Wait { .. }),
                "single-worker schedule must produce no Push/Wait events; got {:?}",
                ev
            );
        }
    }
}
