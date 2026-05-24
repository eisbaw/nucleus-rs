//! Integration tests for the `partition=rows` row-band consumer pass
//! (TASK-0258).
//!
//! Pins:
//!
//! - **Positive (2D nest, multi-worker body)**: a synthetic
//!   Repeat-of-Repeat with `partition=rows` on the outer axis populates
//!   `partition_worker_ranges` with per-worker row-bands (same data
//!   shape `partition=workers` produces).
//! - **Negative 1 (1D iter)**: applying `partition=rows` to a Repeat
//!   whose body has no inner Repeat → `NotOuterOf2DNest`. PRD §6.3.3
//!   "partition=rows on a 1D iteration" is a category error.
//! - **Negative 2 (single-worker body)**: the directive needs a
//!   multi-worker body to be meaningful → `NoMultiWorkerBody`. Mirrors
//!   `PartitionError::NoMultiWorkerBody`.
//! - **Negative 3 (non-divisible)**: the row-count must split exactly
//!   across the worker count → `NonDivisible`. First-cut policy
//!   matching `partition_workers`.
//! - **Determinism**: two runs of the pass produce byte-identical
//!   sidecar contents (BTreeMap discipline).
//!
//! Strategy mirrors `tests/partition_workers.rs`: hand-built synthetic
//! ACFG + minimal LinkedIR (only the `sched.loops` field the pass
//! reads). Both passes share the same downstream contract on
//! `partition_worker_ranges`, so the projection-side coverage from
//! `tests/partition_workers.rs` already covers the post-pass shape;
//! this file pins only the partition_rows-specific structural checks
//! and error variants.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
use nucleus_compiler::event::{DataId, IterVar, KernelId, WorkerId};
use nucleus_compiler::link::LinkedIR;
use nucleus_compiler::passes::partition_rows::{apply_partition_rows, PartitionRowsError};
use nucleus_compiler::sched::{PartitionKind, ResolvedLoopDirective, ResolvedLoopOption, SchedIR};

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

/// Build a 2D Repeat-of-Repeat ACFG: outer iter var = `outer_name`/
/// `outer_id` over `outer_range`; inner iter var = `inner_name`/
/// `inner_id` over `inner_range`; inner body = one Operation placed
/// on `body_workers`.
fn build_2d_acfg(
    outer_name: &str,
    outer_id: u64,
    outer_range: std::ops::Range<i64>,
    inner_name: &str,
    inner_id: u64,
    inner_range: std::ops::Range<i64>,
    body_workers: &[u64],
) -> ACFG {
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".to_string(), WorkerId(0));
    for w in body_workers {
        if *w == 0 {
            continue;
        }
        name_workers.insert(format!("w{w}"), WorkerId(*w));
    }

    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert(outer_name.to_string(), IterVar(outer_id));
    name_iter_vars.insert(inner_name.to_string(), IterVar(inner_id));

    let inner = ACFGNode::Repeat {
        iter_var: IterVar(inner_id),
        range: inner_range,
        body: Box::new(ACFGNode::Sequence(vec![op_on(body_workers)])),
        block_tag: None,
    };
    let outer = ACFGNode::Repeat {
        iter_var: IterVar(outer_id),
        range: outer_range,
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
    };

    ACFG {
        root: ACFGNode::Sequence(vec![outer]),
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

/// Build a 1D ACFG (Repeat → Op, no inner Repeat). Used for the
/// "partition=rows on a 1D iteration" negative.
fn build_1d_acfg(
    iter_name: &str,
    iter_id: u64,
    range: std::ops::Range<i64>,
    body_workers: &[u64],
) -> ACFG {
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".to_string(), WorkerId(0));
    for w in body_workers {
        if *w == 0 {
            continue;
        }
        name_workers.insert(format!("w{w}"), WorkerId(*w));
    }
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert(iter_name.to_string(), IterVar(iter_id));

    let repeat = ACFGNode::Repeat {
        iter_var: IterVar(iter_id),
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

/// Minimal LinkedIR with a single `partition=rows` directive on
/// `iter_name`. The pass reads only `linked.sched.loops`; other fields
/// stay at `Default`.
fn linked_with_rows_directive(iter_name: &str) -> LinkedIR {
    let mut loops = BTreeMap::new();
    loops.insert(
        iter_name.to_string(),
        ResolvedLoopDirective {
            var: iter_name.to_string(),
            options: vec![ResolvedLoopOption::Partition(PartitionKind::Rows)],
            var_span: None,
        },
    );
    let sched = SchedIR {
        loops,
        ..Default::default()
    };
    LinkedIR {
        algo: Default::default(),
        sched,
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers: Default::default(),
        data_consumers: Default::default(),
    }
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

/// AC: a synthetic 2D Repeat-of-Repeat with `partition=rows` on the
/// outer axis over a 4-worker body produces per-worker row-band
/// ranges (0..4, 4..8, 8..12, 12..16) keyed by the outer iter var.
#[test]
fn outer_of_2d_records_per_worker_row_bands() {
    let acfg = build_2d_acfg(
        "y",
        7,
        0..16, // outer (row axis)
        "x",
        8,
        0..32, // inner (col axis — left intact per worker)
        &[1, 2, 3, 4],
    );
    let linked = linked_with_rows_directive("y");

    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows must accept");

    // Outer iter var `y` (id 7) carries the row-band override.
    let per = acfg
        .partition_worker_ranges
        .get(&IterVar(7))
        .expect("outer iter_var must have an override");
    assert_eq!(per.get(&WorkerId(1)), Some(&(0..4)));
    assert_eq!(per.get(&WorkerId(2)), Some(&(4..8)));
    assert_eq!(per.get(&WorkerId(3)), Some(&(8..12)));
    assert_eq!(per.get(&WorkerId(4)), Some(&(12..16)));
    assert_eq!(per.len(), 4);

    // Inner iter var `x` (id 8) MUST NOT be partitioned — partition=rows
    // bands the OUTER axis, the inner runs intact per worker.
    assert!(
        !acfg.partition_worker_ranges.contains_key(&IterVar(8)),
        "inner iter var must NOT have a partition override; partition=rows only bands the outer"
    );
}

/// Negative 1 (PRD §6.3.3): `partition=rows` on a 1D Repeat → typed
/// `NotOuterOf2DNest` error. "rows" presupposes two iteration axes;
/// a 1D loop is a category error.
#[test]
fn negative_partition_rows_on_1d_iter_is_rejected() {
    let acfg = build_1d_acfg("n", 7, 0..16, &[1, 2, 3, 4]);
    let linked = linked_with_rows_directive("n");

    let err = apply_partition_rows(&linked, acfg).expect_err("1D iter must be rejected");
    match err {
        PartitionRowsError::NotOuterOf2DNest { var } => assert_eq!(var, "n"),
        other => panic!("expected NotOuterOf2DNest, got {other:?}"),
    }
}

/// Negative 2: `partition=rows` on a 2D nest whose inner body is
/// single-worker → `NoMultiWorkerBody`. Row-banding across one worker
/// is a no-op.
#[test]
fn negative_single_worker_body_is_rejected() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1]);
    let linked = linked_with_rows_directive("y");

    let err = apply_partition_rows(&linked, acfg).expect_err("single-worker body must be rejected");
    match err {
        PartitionRowsError::NoMultiWorkerBody { var, workers } => {
            assert_eq!(var, "y");
            assert_eq!(workers, 1);
        }
        other => panic!("expected NoMultiWorkerBody, got {other:?}"),
    }
}

/// Negative 3: `partition=rows` on a 2D nest whose outer range does
/// not evenly divide across the worker count → `NonDivisible`. First-
/// cut policy, mirrors `PartitionError::NonDivisible`.
#[test]
fn negative_non_divisible_range_is_rejected() {
    // 17 outer rows across 4 workers — not divisible.
    let acfg = build_2d_acfg("y", 7, 0..17, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = linked_with_rows_directive("y");

    let err = apply_partition_rows(&linked, acfg).expect_err("non-divisible must be rejected");
    match err {
        PartitionRowsError::NonDivisible {
            var,
            lo,
            hi,
            workers,
        } => {
            assert_eq!(var, "y");
            assert_eq!(lo, 0);
            assert_eq!(hi, 17);
            assert_eq!(workers, 4);
        }
        other => panic!("expected NonDivisible, got {other:?}"),
    }
}

/// Determinism: two runs of `apply_partition_rows` on the same ACFG
/// produce byte-identical sidecar contents. Pinned because the
/// per-worker BTreeMap discipline is load-bearing for the
/// cross-(schedule × backend) differential.
#[test]
fn partition_rows_is_deterministic_across_runs() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = linked_with_rows_directive("y");

    let a = apply_partition_rows(&linked, acfg.clone()).expect("run a");
    let b = apply_partition_rows(&linked, acfg).expect("run b");

    assert_eq!(
        a.partition_worker_ranges, b.partition_worker_ranges,
        "two runs of partition_rows must produce identical sidecars"
    );
}

/// No-directive case: an ACFG with no `partition=rows` directive in
/// `linked.sched.loops` is passed through unchanged (empty sidecar).
/// Mirrors `partition_workers`'s no-op behaviour on the `naive`
/// schedule.
#[test]
fn no_directive_is_identity() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2]);
    let linked = LinkedIR {
        algo: Default::default(),
        sched: SchedIR::default(),
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers: Default::default(),
        data_consumers: Default::default(),
    };

    let after = apply_partition_rows(&linked, acfg).expect("no directive must be identity");
    assert!(
        after.partition_worker_ranges.is_empty(),
        "no partition=rows directive → empty sidecar"
    );
}
