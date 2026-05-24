//! Integration tests for the `partition=workers` loop-bound rewrite
//! (TASK-0212).
//!
//! Pins the exact per-worker range shape the partition pass + the
//! per-worker projection produce, so any regression that lets every
//! worker iterate the full source range again (the pre-TASK-0212
//! behaviour described in the task notes) becomes a hard failure.
//!
//! The strategy is hand-built synthetic ACFGs: a `Repeat` over a
//! source range whose body is an `Operation` placed on N workers, plus
//! the per-worker sidecar `partition_worker_ranges` populated
//! manually. Per-worker `Event::Loop.range` projections are then
//! asserted exactly. We exercise the partition pass directly through
//! a small `LinkedIR` constructed only with the schedule fields it
//! reads — the same surface the driver uses.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{build_acfg, ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{DataId, Event, IterVar, KernelId, WorkerId};
use nucleus_compiler::link;
use nucleus_compiler::passes::partition_workers::{apply_partition_workers, PartitionError};
use nucleus_compiler::passes::petri_to_events::acfg_to_events;
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

fn op_on(workers: &[u64]) -> ACFGNode {
    let ws: BTreeSet<WorkerId> = workers.iter().copied().map(WorkerId).collect();
    ACFGNode::Operation(Operation {
        kernel: KernelId(0),
        workers: ws,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(
                vec![DataId(0)],
                KernelId(0),
                Some(DataId(1)),
            )],
        },
    })
}

/// Build a minimal ACFG with one outer `Sequence` containing a single
/// `Repeat` over `range` whose body is one `Operation` placed on
/// `body_workers`. `iter_var_name` is registered in `name_iter_vars`
/// against `iter_var_id`.
fn build_synthetic_acfg(
    iter_var_name: &str,
    iter_var_id: u64,
    range: std::ops::Range<i64>,
    body_workers: &[u64],
) -> ACFG {
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    // host appears in `name_workers` but never in the body; mirrors
    // example 13-cnn-inference/batch_parallel where host is declared
    // but partition=workers targets only the compute workers.
    name_workers.insert("host".to_string(), WorkerId(0));
    for w in body_workers {
        // Test-only label: name workers by their id so 0 stays "host"
        // and any worker can appear in `body_workers` without name
        // collision arithmetic.
        if *w == 0 {
            // Already inserted as host above.
            continue;
        }
        name_workers.insert(format!("w{w}"), WorkerId(*w));
    }

    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert(iter_var_name.to_string(), IterVar(iter_var_id));

    let repeat = ACFGNode::Repeat {
        iter_var: IterVar(iter_var_id),
        range,
        body: Box::new(ACFGNode::Sequence(vec![op_on(body_workers)])),
        block_tag: None,
    };

    ACFG {
        root: ACFGNode::Sequence(vec![repeat]),
        name_kernels: Default::default(),
        name_data: Default::default(),
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
    }
}

/// Extract every worker's outermost `Event::Loop` for the given
/// `iter_var`. Returns a `BTreeMap<WorkerId, Range<i64>>` of the
/// projected ranges. Asserts exactly one match per worker that has a
/// loop in its EventList.
fn collect_loop_ranges_per_worker(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    iter_var: IterVar,
) -> BTreeMap<WorkerId, std::ops::Range<i64>> {
    let mut out: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    for (wid, events) in per_worker {
        for ev in events {
            if let Event::Loop {
                iter_var: iv,
                range,
                ..
            } = ev
            {
                if *iv == iter_var {
                    let prev = out.insert(*wid, range.clone());
                    assert!(
                        prev.is_none(),
                        "worker {wid:?} has more than one Event::Loop for {iter_var:?}"
                    );
                }
            }
        }
    }
    out
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

/// AC#3 from TASK-0212: synthetic 4-element range across 2 workers.
/// Each worker's projected Event::Loop must carry its own exclusive
/// half — w0: 0..2, w1: 2..4.
#[test]
fn projection_honours_per_worker_range_two_workers() {
    let mut acfg = build_synthetic_acfg("n", 7, 0..4, &[1, 2]);
    // Populate the sidecar by hand — equivalent to running
    // `apply_partition_workers` on a schedule with `partition=workers`.
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..2);
    per_worker.insert(WorkerId(2), 2..4);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let events = acfg_to_events(&acfg);
    let by_worker = collect_loop_ranges_per_worker(&events, IterVar(7));

    assert_eq!(by_worker.get(&WorkerId(1)), Some(&(0..2)));
    assert_eq!(by_worker.get(&WorkerId(2)), Some(&(2..4)));
    // Union of the two slices covers the source range exactly once.
    let mut covered: Vec<i64> = Vec::new();
    for r in by_worker.values() {
        for v in r.clone() {
            covered.push(v);
        }
    }
    covered.sort();
    assert_eq!(covered, vec![0, 1, 2, 3]);
}

/// The exact shape from the task brief: B=16 split across 4 workers
/// must yield w0=0..4, w1=4..8, w2=8..12, w3=12..16.
#[test]
fn cnn_batch_parallel_shape_b16_n4() {
    let mut acfg = build_synthetic_acfg("n", 7, 0..16, &[1, 2, 3, 4]);
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..4);
    per_worker.insert(WorkerId(2), 4..8);
    per_worker.insert(WorkerId(3), 8..12);
    per_worker.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges
        .insert(IterVar(7), per_worker.clone());

    let events = acfg_to_events(&acfg);
    let by_worker = collect_loop_ranges_per_worker(&events, IterVar(7));

    assert_eq!(by_worker.get(&WorkerId(1)), Some(&(0..4)));
    assert_eq!(by_worker.get(&WorkerId(2)), Some(&(4..8)));
    assert_eq!(by_worker.get(&WorkerId(3)), Some(&(8..12)));
    assert_eq!(by_worker.get(&WorkerId(4)), Some(&(12..16)));
}

/// A worker not listed in the per-iter-var override map gets the
/// source range — the pre-TASK-0212 contract for non-participating
/// workers (e.g. host) that nonetheless project body events.
#[test]
fn worker_not_in_override_falls_back_to_source_range() {
    // Body on w0+w1+host (host=0). Override only for w0,w1. Host
    // should fall back to 0..4.
    let mut acfg = build_synthetic_acfg("n", 7, 0..4, &[0, 1, 2]);
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..2);
    per_worker.insert(WorkerId(2), 2..4);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let events = acfg_to_events(&acfg);
    let by_worker = collect_loop_ranges_per_worker(&events, IterVar(7));

    assert_eq!(by_worker.get(&WorkerId(0)), Some(&(0..4))); // host: source range
    assert_eq!(by_worker.get(&WorkerId(1)), Some(&(0..2)));
    assert_eq!(by_worker.get(&WorkerId(2)), Some(&(2..4)));
}

/// A Repeat with no per-iter-var override (the empty-sidecar case) is
/// byte-identical to the pre-TASK-0212 projection: every worker that
/// projects body events sees the source range.
#[test]
fn no_override_preserves_source_range() {
    let acfg = build_synthetic_acfg("n", 7, 0..16, &[1, 2, 3, 4]);
    // No insertion into partition_worker_ranges.

    let events = acfg_to_events(&acfg);
    let by_worker = collect_loop_ranges_per_worker(&events, IterVar(7));

    for wid in [WorkerId(1), WorkerId(2), WorkerId(3), WorkerId(4)] {
        assert_eq!(
            by_worker.get(&wid),
            Some(&(0..16)),
            "worker {wid:?} should iterate the full source range when the override is absent"
        );
    }
}

// --------------------------------------------------------------------
// End-to-end: real CNN batch_parallel schedule
// --------------------------------------------------------------------

fn read_example(relpath: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let full = repo_root.join("nuc-nucleus").join("examples").join(relpath);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read example {full:?}: {e}"))
}

/// AC#1/AC#2 end-to-end: lower the actual CNN batch_parallel schedule
/// through the partition pass, then project. Assert every compute
/// worker's outermost `Event::Loop` over the batch iter var carries
/// the expected B/N slice. B=16, N=4 ⇒ w0=0..4, w1=4..8, w2=8..12,
/// w3=12..16. This is the headline result from the task description.
#[test]
fn cnn_batch_parallel_projects_b_over_n_per_worker() {
    let algo_src = read_example("13-cnn-inference/prog.algo.nuc");
    let sched_src = read_example("13-cnn-inference/schedules/batch_parallel.sched.nuc");
    let algo = lower_algo(&parse_algo(&algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&sched_src).expect("sched parse")).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");

    // The batch loop variable in 13-cnn-inference/prog.algo.nuc is `n`.
    let n_id = *acfg
        .name_iter_vars
        .get("n")
        .expect("acfg should expose iter var `n`");
    let w0 = *acfg.name_workers.get("w0").expect("w0 in name_workers");
    let w1 = *acfg.name_workers.get("w1").expect("w1 in name_workers");
    let w2 = *acfg.name_workers.get("w2").expect("w2 in name_workers");
    let w3 = *acfg.name_workers.get("w3").expect("w3 in name_workers");

    // Sidecar contents: the partition pass must have recorded entries
    // for `n` covering the four compute workers, each owning exactly
    // 4 batch elements (16/4).
    let per = acfg
        .partition_worker_ranges
        .get(&n_id)
        .expect("partition pass should record per-worker ranges for `n`");
    assert_eq!(per.get(&w0), Some(&(0..4)));
    assert_eq!(per.get(&w1), Some(&(4..8)));
    assert_eq!(per.get(&w2), Some(&(8..12)));
    assert_eq!(per.get(&w3), Some(&(12..16)));

    // Project — every compute worker's Event::Loop over n carries its
    // slice; the host (not in the body's Operation.workers) gets no
    // entry for `n` (or, if it did, would fall back to source range —
    // but in this algorithm the host has no body event under `for n`).
    let events = acfg_to_events(&acfg);
    let ranges = collect_loop_ranges_per_worker(&events, n_id);
    assert_eq!(
        ranges.get(&w0),
        Some(&(0..4)),
        "w0 must iterate batch slice 0..4, not 0..16"
    );
    assert_eq!(ranges.get(&w1), Some(&(4..8)));
    assert_eq!(ranges.get(&w2), Some(&(8..12)));
    assert_eq!(ranges.get(&w3), Some(&(12..16)));
}

/// AC#4 no-regression: an algorithm + schedule with NO
/// `partition=workers` directive (e.g. the CNN's `naive` schedule)
/// produces an empty `partition_worker_ranges` sidecar and the
/// projection matches the pre-TASK-0212 source-range emit.
#[test]
fn naive_schedule_records_no_partition_ranges() {
    let algo_src = read_example("13-cnn-inference/prog.algo.nuc");
    let sched_src = read_example("13-cnn-inference/schedules/naive.sched.nuc");
    let algo = lower_algo(&parse_algo(&algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&sched_src).expect("sched parse")).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    assert!(
        acfg.partition_worker_ranges.is_empty(),
        "naive schedule has no partition=workers; sidecar must stay empty"
    );
}

/// TASK-0262: a non-divisible (length, N) range no longer rejects.
/// Floor-with-spillover policy: 17 rows across 3 workers → floor=5,
/// extras=17%3=2 → first 2 workers get 6 rows, last gets 5. Pin the
/// exact per-worker bands so the policy choice is regression-locked.
#[test]
fn non_divisible_range_spillover_policy() {
    use nucleus_compiler::sched::{
        PartitionKind, ResolvedLoopDirective, ResolvedLoopOption, SchedIR,
    };

    let mut loops = BTreeMap::new();
    loops.insert(
        "n".to_string(),
        ResolvedLoopDirective {
            var: "n".to_string(),
            options: vec![ResolvedLoopOption::Partition(PartitionKind::Workers)],
            // TASK-0099: hand-built test fixture has no source text.
            var_span: None,
        },
    );
    let sched = SchedIR {
        loops,
        ..Default::default()
    };

    let linked = nucleus_compiler::link::LinkedIR {
        algo: Default::default(),
        sched,
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers: Default::default(),
        data_consumers: Default::default(),
    };

    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("w0".to_string(), WorkerId(1));
    name_workers.insert("w1".to_string(), WorkerId(2));
    name_workers.insert("w2".to_string(), WorkerId(3));
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("n".to_string(), IterVar(7));

    let acfg = ACFG {
        root: ACFGNode::Sequence(vec![ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..17,
            body: Box::new(ACFGNode::Sequence(vec![op_on(&[1, 2, 3])])),
            block_tag: None,
        }]),
        name_kernels: Default::default(),
        name_data: Default::default(),
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
    };

    let after = apply_partition_workers(&linked, acfg).expect("non-divisible now lowers");
    let bands = after
        .partition_worker_ranges
        .get(&IterVar(7))
        .expect("partition entry recorded");
    assert_eq!(bands.len(), 3);
    // Spillover: w1, w2 each get +1 row; w3 gets the floor.
    assert_eq!(bands[&WorkerId(1)], 0..6);
    assert_eq!(bands[&WorkerId(2)], 6..12);
    assert_eq!(bands[&WorkerId(3)], 12..17);
}

/// TASK-0262: when L < N (fewer rows than workers), the pass still
/// rejects with the renamed `InsufficientWork` variant — spillover
/// cannot give every worker at least one row. 3 rows across 4 workers.
#[test]
fn insufficient_work_range_is_rejected() {
    use nucleus_compiler::sched::{
        PartitionKind, ResolvedLoopDirective, ResolvedLoopOption, SchedIR,
    };

    let mut loops = BTreeMap::new();
    loops.insert(
        "n".to_string(),
        ResolvedLoopDirective {
            var: "n".to_string(),
            options: vec![ResolvedLoopOption::Partition(PartitionKind::Workers)],
            var_span: None,
        },
    );
    let sched = SchedIR {
        loops,
        ..Default::default()
    };

    let linked = nucleus_compiler::link::LinkedIR {
        algo: Default::default(),
        sched,
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers: Default::default(),
        data_consumers: Default::default(),
    };

    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("w0".to_string(), WorkerId(1));
    name_workers.insert("w1".to_string(), WorkerId(2));
    name_workers.insert("w2".to_string(), WorkerId(3));
    name_workers.insert("w3".to_string(), WorkerId(4));
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("n".to_string(), IterVar(7));

    let acfg = ACFG {
        root: ACFGNode::Sequence(vec![ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..3,
            body: Box::new(ACFGNode::Sequence(vec![op_on(&[1, 2, 3, 4])])),
            block_tag: None,
        }]),
        name_kernels: Default::default(),
        name_data: Default::default(),
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
    };

    let err = apply_partition_workers(&linked, acfg).expect_err("L<N must reject");
    match err {
        PartitionError::InsufficientWork {
            var,
            lo,
            hi,
            workers,
        } => {
            assert_eq!(var, "n");
            assert_eq!(lo, 0);
            assert_eq!(hi, 3);
            assert_eq!(workers, 4);
        }
        other => panic!("expected InsufficientWork, got {other:?}"),
    }
}

/// Deterministic: two runs of `acfg_to_events` produce byte-identical
/// per-worker projections under the override. Pinned because the
/// sidecar is a BTreeMap and any HashMap creep would break the
/// cross-(schedule × backend) differential.
#[test]
fn projection_is_deterministic_under_override() {
    let mut acfg = build_synthetic_acfg("n", 7, 0..16, &[1, 2, 3, 4]);
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..4);
    per_worker.insert(WorkerId(2), 4..8);
    per_worker.insert(WorkerId(3), 8..12);
    per_worker.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let a = acfg_to_events(&acfg);
    let b = acfg_to_events(&acfg);
    assert_eq!(a, b, "two projections of the same ACFG must be identical");
}

// --------------------------------------------------------------------
// TASK-0117 ↔ TASK-0212 ↔ sync-injection composition tests
// --------------------------------------------------------------------

/// On a partitioned `Repeat`, `inject_syncs` must NOT inject a
/// body-internal entry / exit `Sync`: the per-iteration barrier would
/// deadlock because the host iterates the source range while each
/// compute worker iterates its slice. The loop-boundary Syncs (between
/// the Repeat and its prior/next siblings) are unaffected.
///
/// Regression test for the sync-injection co-fix in TASK-0117 cycle-1.
#[test]
fn partitioned_repeat_skips_body_entry_exit_syncs() {
    use nucleus_compiler::acfg::ACFGNode;

    // Mirror the example-13 batch_parallel shape with synthetic ops:
    //   host: write `a` -> Repeat (body on {w1..w4}: read `a`) ->
    //   host read `b`. The body has cross-worker writes -> pre-fix
    //   this would attach an entry-sync at body[0] AND an exit-sync
    //   at body[end].
    use nucleus_compiler::acfg::{DataflowDag, DataflowEdge, Operation};
    fn op_on(workers: &[u64], data_in: &[u64], data_out: Option<u64>) -> ACFGNode {
        let ws: BTreeSet<WorkerId> = workers.iter().copied().map(WorkerId).collect();
        ACFGNode::Operation(Operation {
            kernel: nucleus_compiler::event::KernelId(0),
            workers: ws,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    data_in.iter().copied().map(DataId).collect(),
                    nucleus_compiler::event::KernelId(0),
                    data_out.map(DataId),
                )],
            },
        })
    }

    let body = ACFGNode::Sequence(vec![op_on(&[1, 2, 3, 4], &[0], Some(1))]);
    let root = ACFGNode::Sequence(vec![
        op_on(&[0], &[], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        },
        op_on(&[0], &[1], None),
    ]);

    // Sidecar marks IterVar(7) as partitioned across {w1..w4}.
    let mut acfg = nucleus_compiler::acfg::ACFG {
        root,
        name_kernels: Default::default(),
        name_data: BTreeMap::from([("a".to_string(), DataId(0)), ("b".to_string(), DataId(1))]),
        name_workers: BTreeMap::from([
            ("host".to_string(), WorkerId(0)),
            ("w1".to_string(), WorkerId(1)),
            ("w2".to_string(), WorkerId(2)),
            ("w3".to_string(), WorkerId(3)),
            ("w4".to_string(), WorkerId(4)),
        ]),
        name_iter_vars: BTreeMap::from([("n".to_string(), IterVar(7))]),
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
    };
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..4);
    per_worker.insert(WorkerId(2), 4..8);
    per_worker.insert(WorkerId(3), 8..12);
    per_worker.insert(WorkerId(4), 12..16);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    let after = inject_syncs(acfg);

    // The Repeat's body must contain ZERO Sync nodes.
    if let ACFGNode::Sequence(children) = &after.root {
        let repeat = children
            .iter()
            .find_map(|c| match c {
                ACFGNode::Repeat { body, .. } => Some(body.as_ref()),
                _ => None,
            })
            .expect("Repeat node");
        if let ACFGNode::Sequence(body_children) = repeat {
            let sync_count = body_children
                .iter()
                .filter(|c| matches!(c, ACFGNode::Sync(_)))
                .count();
            assert_eq!(
                sync_count, 0,
                "partitioned Repeat body must have zero Sync nodes (got {sync_count})"
            );
        } else {
            panic!("Repeat body must be a Sequence");
        }
    } else {
        panic!("expected top-level Sequence");
    }
}

/// The same shape on an UN-partitioned Repeat must still get its
/// per-iteration entry/exit Syncs, so the regression for examples
/// like 02-split-add (which legitimately need the per-iteration
/// barrier) is locked in.
#[test]
fn non_partitioned_repeat_keeps_body_entry_exit_syncs() {
    use nucleus_compiler::acfg::{ACFGNode, DataflowDag, DataflowEdge, Operation};
    fn op_on(workers: &[u64], data_in: &[u64], data_out: Option<u64>) -> ACFGNode {
        let ws: BTreeSet<WorkerId> = workers.iter().copied().map(WorkerId).collect();
        ACFGNode::Operation(Operation {
            kernel: nucleus_compiler::event::KernelId(0),
            workers: ws,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    data_in.iter().copied().map(DataId).collect(),
                    nucleus_compiler::event::KernelId(0),
                    data_out.map(DataId),
                )],
            },
        })
    }

    let body = ACFGNode::Sequence(vec![op_on(&[1], &[0], Some(1))]);
    let root = ACFGNode::Sequence(vec![
        op_on(&[0], &[], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        },
        op_on(&[0], &[1], None),
    ]);
    let acfg = nucleus_compiler::acfg::ACFG {
        root,
        name_kernels: Default::default(),
        name_data: BTreeMap::from([("a".to_string(), DataId(0)), ("b".to_string(), DataId(1))]),
        name_workers: BTreeMap::from([
            ("host".to_string(), WorkerId(0)),
            ("w0".to_string(), WorkerId(1)),
        ]),
        name_iter_vars: BTreeMap::from([("n".to_string(), IterVar(7))]),
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: BTreeMap::new(), // empty — no partition
        pipeline_depth_for_seq: BTreeMap::new(),
        halo_widths: BTreeMap::new(),
        reuse_widths: BTreeMap::new(),
    };

    let after = inject_syncs(acfg);

    if let ACFGNode::Sequence(children) = &after.root {
        let repeat = children
            .iter()
            .find_map(|c| match c {
                ACFGNode::Repeat { body, .. } => Some(body.as_ref()),
                _ => None,
            })
            .expect("Repeat node");
        if let ACFGNode::Sequence(body_children) = repeat {
            let sync_count = body_children
                .iter()
                .filter(|c| matches!(c, ACFGNode::Sync(_)))
                .count();
            assert!(
                sync_count >= 1,
                "non-partitioned Repeat body MUST keep its entry/exit Syncs \
                 (got {sync_count})"
            );
        } else {
            panic!("Repeat body must be a Sequence");
        }
    }
}

/// End-to-end composition: apply partition_workers + transfer_inject
/// on a synthetic 1:N broadcast inside a partitioned Repeat, and
/// assert that the FINAL `Xfer` tile carries the per-worker partition
/// slice (the `rewrite_partition_tiles` sink). This is the load-bearing
/// composability check between TASK-0212 and TASK-0117.
#[test]
fn transfer_fanout_composes_with_partition_sidecar() {
    use nucleus_compiler::acfg::{ACFGNode, DataflowDag, DataflowEdge, Operation, XferRole, ACFG};
    use nucleus_compiler::link::{LinkedIR, WorkerEntity};

    fn op_on(workers: &[u64], data_in: &[u64], data_out: Option<u64>) -> ACFGNode {
        let ws: BTreeSet<WorkerId> = workers.iter().copied().map(WorkerId).collect();
        ACFGNode::Operation(Operation {
            kernel: nucleus_compiler::event::KernelId(0),
            workers: ws,
            dataflow: DataflowDag {
                edges: vec![DataflowEdge::new(
                    data_in.iter().copied().map(DataId).collect(),
                    nucleus_compiler::event::KernelId(0),
                    data_out.map(DataId),
                )],
            },
        })
    }

    // host writes `a` at top level; Repeat body on {w1..w4} reads
    // `a` (cross-worker, broadcast 1:N).
    let body = ACFGNode::Sequence(vec![op_on(&[1, 2, 3, 4], &[0], Some(1))]);
    let root = ACFGNode::Sequence(vec![
        op_on(&[0], &[], Some(0)),
        ACFGNode::Repeat {
            iter_var: IterVar(7),
            range: 0..8,
            body: Box::new(body),
            block_tag: None,
        },
    ]);

    let mut acfg = ACFG {
        root,
        name_kernels: Default::default(),
        name_data: BTreeMap::from([("a".to_string(), DataId(0)), ("b".to_string(), DataId(1))]),
        name_workers: BTreeMap::from([
            ("host".to_string(), WorkerId(0)),
            ("w1".to_string(), WorkerId(1)),
            ("w2".to_string(), WorkerId(2)),
            ("w3".to_string(), WorkerId(3)),
            ("w4".to_string(), WorkerId(4)),
        ]),
        name_iter_vars: BTreeMap::from([("n".to_string(), IterVar(7))]),
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
    };

    // Populate the partition sidecar by hand — equivalent to running
    // apply_partition_workers on a schedule that declares
    // `loop n : partition=workers;`.
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(1), 0..2);
    per_worker.insert(WorkerId(2), 2..4);
    per_worker.insert(WorkerId(3), 4..6);
    per_worker.insert(WorkerId(4), 6..8);
    acfg.partition_worker_ranges.insert(IterVar(7), per_worker);

    // Build a minimal LinkedIR carrying just enough for
    // inject_transfers: the producer entity for `a` (host) and a
    // schedule with the transfer directive.
    let sched_src = r#"schedule for "../prog.algo.nuc" {
    workers = { host, w1, w2, w3, w4 };
    transfer a : sync;
}"#;
    let sched_ast = parse_sched(sched_src).expect("parse");
    let sched = lower_sched(&sched_ast).expect("lower");
    let mut data_producers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    data_producers.insert(
        "a".to_string(),
        WorkerEntity(BTreeSet::from(["host".to_string()])),
    );
    let linked = LinkedIR {
        algo: Default::default(),
        sched,
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers,
        data_consumers: Default::default(),
    };

    let after = inject_transfers(&linked, acfg);

    // After the fan-out, count Wait nodes whose tile == the expected
    // per-worker slice for their dst.
    fn collect_waits(node: &ACFGNode, out: &mut Vec<(WorkerId, std::ops::Range<i64>)>) {
        match node {
            ACFGNode::Xfer(x) if x.role == XferRole::Wait => {
                let r = x
                    .tile
                    .bounds
                    .first()
                    .map(|(_, r)| r.clone())
                    .unwrap_or(0..0);
                out.push((x.dst, r));
            }
            ACFGNode::Sequence(cs) => {
                for c in cs {
                    collect_waits(c, out);
                }
            }
            ACFGNode::Repeat { body, .. } => collect_waits(body, out),
            _ => {}
        }
    }
    let mut waits = Vec::new();
    collect_waits(&after.root, &mut waits);
    waits.sort_by_key(|(w, _)| w.0);
    assert_eq!(
        waits,
        vec![
            (WorkerId(1), 0..2),
            (WorkerId(2), 2..4),
            (WorkerId(3), 4..6),
            (WorkerId(4), 6..8),
        ],
        "every fan-out Wait's tile must carry its dst worker's partition slice"
    );
}
