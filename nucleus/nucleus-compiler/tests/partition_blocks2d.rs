//! Integration tests for the `partition=blocks2d` 2D-grid consumer
//! pass (TASK-0259).
//!
//! Pins:
//!
//! - **Positive (4 workers, 2x2 grid)**: a synthetic 2D Repeat-of-
//!   Repeat with `partition=blocks2d` on the outer axis populates
//!   BOTH `partition_worker_ranges[outer]` (per-worker y-band) and
//!   `partition_worker_ranges[inner]` (per-worker x-band). Each
//!   worker's (y_band, x_band) pair forms its grid cell.
//! - **Positive (6 workers, 2x3 grid)**: pins the deterministic
//!   close-to-square decomposition (`floor(sqrt(6)) == 2`, `6 % 2 == 0`
//!   → `R=2, C=3`) and the worker → (row, col) assignment.
//! - **Negative 1 (1D iter)**: applying `partition=blocks2d` to a
//!   Repeat with no inner Repeat → `NotOuterOf2DNest`.
//! - **Negative 2 (single-worker body)**: `NoMultiWorkerBody`.
//! - **Negative 3 (prime worker count)**: 7 workers → `DegenerateGridShape`.
//!   No non-degenerate 2D factorisation.
//! - **Negative 4 (non-divisible y)**: y range not evenly divisible by
//!   grid_rows → `NonDivisible { axis: 'y', ... }`.
//! - **Negative 5 (non-divisible x)**: x range not evenly divisible by
//!   grid_cols → `NonDivisible { axis: 'x', ... }`.
//! - **Determinism**: two runs of the pass produce byte-identical
//!   sidecar contents.
//! - **No-directive identity**: an ACFG with no `partition=blocks2d`
//!   directive passes through unchanged.
//!
//! Strategy mirrors `tests/partition_rows.rs`: hand-built synthetic
//! ACFG + minimal LinkedIR.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
use nucleus_compiler::event::{DataId, IterVar, KernelId, WorkerId};
use nucleus_compiler::link::LinkedIR;
use nucleus_compiler::passes::partition_blocks2d::{
    apply_partition_blocks2d, PartitionBlocks2dError,
};
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
        partition_pairs: std::collections::BTreeMap::new(),
        grid_shape_for_outer_iv: std::collections::BTreeMap::new(),
    }
}

/// Build a 1D ACFG (Repeat → Op, no inner Repeat). Used for the
/// "partition=blocks2d on a 1D iteration" negative.
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
        partition_pairs: std::collections::BTreeMap::new(),
        grid_shape_for_outer_iv: std::collections::BTreeMap::new(),
    }
}

/// Minimal LinkedIR with a single `partition=blocks2d` directive on
/// `iter_name`. The pass reads only `linked.sched.loops`; other fields
/// stay at `Default`.
fn linked_with_blocks2d_directive(iter_name: &str) -> LinkedIR {
    let mut loops = BTreeMap::new();
    loops.insert(
        iter_name.to_string(),
        ResolvedLoopDirective {
            var: iter_name.to_string(),
            options: vec![ResolvedLoopOption::Partition(PartitionKind::Blocks2d)],
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
// Positive tests
// --------------------------------------------------------------------

/// AC: synthetic 2D Repeat-of-Repeat with `partition=blocks2d` on the
/// outer axis over a 4-worker body produces a 2x2 grid. Pinned worker
/// assignment (BTreeSet iteration order on WorkerId):
///   w1 → (row=0, col=0) → y=0..8,  x=0..16
///   w2 → (row=0, col=1) → y=0..8,  x=16..32
///   w3 → (row=1, col=0) → y=8..16, x=0..16
///   w4 → (row=1, col=1) → y=8..16, x=16..32
#[test]
fn positive_4_workers_records_2x2_per_worker_ranges() {
    let acfg = build_2d_acfg(
        "y",
        7,
        0..16, // outer
        "x",
        8,
        0..32, // inner
        &[1, 2, 3, 4],
    );
    let linked = linked_with_blocks2d_directive("y");

    let acfg =
        apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d must accept 4 workers");

    // Both iter_vars must have per-worker overrides.
    let per_y = acfg
        .partition_worker_ranges
        .get(&IterVar(7))
        .expect("outer iter_var must have a y-band override");
    let per_x = acfg
        .partition_worker_ranges
        .get(&IterVar(8))
        .expect("inner iter_var must have an x-band override");
    assert_eq!(per_y.len(), 4);
    assert_eq!(per_x.len(), 4);

    // Pin per-worker (y_band, x_band) pairs.
    assert_eq!(per_y.get(&WorkerId(1)), Some(&(0..8)));
    assert_eq!(per_x.get(&WorkerId(1)), Some(&(0..16)));

    assert_eq!(per_y.get(&WorkerId(2)), Some(&(0..8)));
    assert_eq!(per_x.get(&WorkerId(2)), Some(&(16..32)));

    assert_eq!(per_y.get(&WorkerId(3)), Some(&(8..16)));
    assert_eq!(per_x.get(&WorkerId(3)), Some(&(0..16)));

    assert_eq!(per_y.get(&WorkerId(4)), Some(&(8..16)));
    assert_eq!(per_x.get(&WorkerId(4)), Some(&(16..32)));

    // TASK-0264 cycle 113 AC#1: partition_pairs records the (outer, inner)
    // pairing of the blocks2d directive. Keyed by outer_iter_var; value
    // is the paired inner iv.
    assert_eq!(
        acfg.partition_pairs.get(&IterVar(7)),
        Some(&IterVar(8)),
        "partition_pairs[outer_iv]=inner_iv must record the blocks2d coupling"
    );
    assert_eq!(
        acfg.partition_pairs.len(),
        1,
        "exactly one pair recorded for one blocks2d directive"
    );

    // TASK-0264 cycle 113 AC#2: grid_shape_for_outer_iv records the
    // decompose_grid result. 4 workers ⇒ (2, 2) perfect square.
    assert_eq!(
        acfg.grid_shape_for_outer_iv.get(&IterVar(7)),
        Some(&(2u32, 2u32)),
        "grid_shape_for_outer_iv[outer_iv]=(rows, cols) must record the (2, 2) grid"
    );
    assert_eq!(
        acfg.grid_shape_for_outer_iv.len(),
        1,
        "exactly one grid shape recorded for one blocks2d directive"
    );
}

/// AC: 6 workers → 2x3 grid (R=2, C=3 by deterministic decomposition).
/// Pin the exact layout:
///   w1 → (0, 0) → y=0..8,  x=0..6
///   w2 → (0, 1) → y=0..8,  x=6..12
///   w3 → (0, 2) → y=0..8,  x=12..18
///   w4 → (1, 0) → y=8..16, x=0..6
///   w5 → (1, 1) → y=8..16, x=6..12
///   w6 → (1, 2) → y=8..16, x=12..18
#[test]
fn positive_6_workers_records_2x3_per_worker_ranges() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..18, &[1, 2, 3, 4, 5, 6]);
    let linked = linked_with_blocks2d_directive("y");

    let acfg =
        apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d must accept 6 workers");

    let per_y = acfg.partition_worker_ranges.get(&IterVar(7)).unwrap();
    let per_x = acfg.partition_worker_ranges.get(&IterVar(8)).unwrap();
    assert_eq!(per_y.len(), 6);
    assert_eq!(per_x.len(), 6);

    // Row 0
    assert_eq!(per_y.get(&WorkerId(1)), Some(&(0..8)));
    assert_eq!(per_x.get(&WorkerId(1)), Some(&(0..6)));
    assert_eq!(per_y.get(&WorkerId(2)), Some(&(0..8)));
    assert_eq!(per_x.get(&WorkerId(2)), Some(&(6..12)));
    assert_eq!(per_y.get(&WorkerId(3)), Some(&(0..8)));
    assert_eq!(per_x.get(&WorkerId(3)), Some(&(12..18)));

    // Row 1
    assert_eq!(per_y.get(&WorkerId(4)), Some(&(8..16)));
    assert_eq!(per_x.get(&WorkerId(4)), Some(&(0..6)));
    assert_eq!(per_y.get(&WorkerId(5)), Some(&(8..16)));
    assert_eq!(per_x.get(&WorkerId(5)), Some(&(6..12)));
    assert_eq!(per_y.get(&WorkerId(6)), Some(&(8..16)));
    assert_eq!(per_x.get(&WorkerId(6)), Some(&(12..18)));

    // TASK-0264 cycle 113 AC#1 + AC#2: 6-worker grid pins are (2, 3).
    assert_eq!(acfg.partition_pairs.get(&IterVar(7)), Some(&IterVar(8)));
    assert_eq!(
        acfg.grid_shape_for_outer_iv.get(&IterVar(7)),
        Some(&(2u32, 3u32)),
    );
}

/// TASK-0264 cycle 113 architect P2.1: the existing positive tests
/// above pin the ACFG writer contract; this test extends to the
/// END-TO-END wire shape (`build_sidecar` mirror). A regression that
/// dropped the mirror from `build_sidecar` would silently zero out
/// the TASK-0289 consumer's input — the partition_pairs entry would
/// be present in ACFG but absent in NameSidecar. This test bites such
/// a regression.
#[test]
fn positive_4_workers_sidecar_mirrors_pair_and_grid_shape() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = linked_with_blocks2d_directive("y");
    let acfg =
        nucleus_compiler::passes::partition_blocks2d::apply_partition_blocks2d(&linked, acfg)
            .expect("partition_blocks2d must accept 4 workers");

    let sidecar = nucleus_compiler::build_sidecar(&linked, &acfg).expect("build_sidecar");

    // Mirror invariants: both new fields land in NameSidecar with the
    // same (outer -> inner) and (outer -> (rows, cols)) shape the ACFG
    // recorded. The mirror is a `.clone()` in build_sidecar; a future
    // regression that drops the clone would zero one or both maps
    // here.
    assert_eq!(
        sidecar.partition_pairs, acfg.partition_pairs,
        "NameSidecar.partition_pairs must mirror ACFG.partition_pairs exactly"
    );
    assert_eq!(
        sidecar.grid_shape_for_outer_iv, acfg.grid_shape_for_outer_iv,
        "NameSidecar.grid_shape_for_outer_iv must mirror ACFG.grid_shape_for_outer_iv exactly"
    );
    // Positive shape pins (non-empty + correct values).
    assert_eq!(sidecar.partition_pairs.get(&IterVar(7)), Some(&IterVar(8)));
    assert_eq!(
        sidecar.grid_shape_for_outer_iv.get(&IterVar(7)),
        Some(&(2u32, 2u32)),
    );
}

// --------------------------------------------------------------------
// Negative tests
// --------------------------------------------------------------------

/// Negative 1 (PRD §6.3.3): `partition=blocks2d` on a 1D Repeat → typed
/// `NotOuterOf2DNest`. The "2D blocks" name presupposes two iteration
/// axes; a 1D loop is a category error.
#[test]
fn negative_partition_blocks2d_on_1d_iter_is_rejected() {
    let acfg = build_1d_acfg("n", 7, 0..16, &[1, 2, 3, 4]);
    let linked = linked_with_blocks2d_directive("n");

    let err = apply_partition_blocks2d(&linked, acfg).expect_err("1D iter must be rejected");
    match err {
        PartitionBlocks2dError::NotOuterOf2DNest { var } => assert_eq!(var, "n"),
        other => panic!("expected NotOuterOf2DNest, got {other:?}"),
    }
}

/// Negative 2: single-worker body → `NoMultiWorkerBody`. 2D grid is
/// meaningless with one worker.
#[test]
fn negative_single_worker_body_is_rejected() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1]);
    let linked = linked_with_blocks2d_directive("y");

    let err =
        apply_partition_blocks2d(&linked, acfg).expect_err("single-worker body must be rejected");
    match err {
        PartitionBlocks2dError::NoMultiWorkerBody { var, workers } => {
            assert_eq!(var, "y");
            assert_eq!(workers, 1);
        }
        other => panic!("expected NoMultiWorkerBody, got {other:?}"),
    }
}

/// Bite test (TASK-0400): a `partition=blocks2d` directive naming a loop
/// var ABSENT from `name_iter_vars` trips `UnknownLoopVar` at the name-
/// resolution guard. WHITE-BOX invariant pin (the linker resolves
/// directive vars against declared loops on the surface path, so this
/// inconsistent `LinkedIR`/`ACFG` pair is hand-built); completes the
/// 3-pass sibling sweep with `partition_workers` + `partition_rows`.
#[test]
fn negative_unknown_loop_var_when_directive_var_absent() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = linked_with_blocks2d_directive("ghost");

    let err = apply_partition_blocks2d(&linked, acfg)
        .expect_err("directive var absent from name_iter_vars must reject");
    match err {
        PartitionBlocks2dError::UnknownLoopVar { var } => assert_eq!(var, "ghost"),
        other => panic!("expected UnknownLoopVar, got {other:?}"),
    }
}

/// Negative 3: prime worker count → `DegenerateGridShape`. 7 workers
/// has no non-degenerate 2D factorisation (only (1, 7)).
#[test]
fn negative_prime_workers_degenerate_grid() {
    let acfg = build_2d_acfg("y", 7, 0..14, "x", 8, 0..28, &[1, 2, 3, 4, 5, 6, 11]);
    let linked = linked_with_blocks2d_directive("y");

    let err = apply_partition_blocks2d(&linked, acfg)
        .expect_err("7 workers must be rejected as degenerate");
    match err {
        PartitionBlocks2dError::DegenerateGridShape { var, workers } => {
            assert_eq!(var, "y");
            assert_eq!(workers, 7);
        }
        other => panic!("expected DegenerateGridShape, got {other:?}"),
    }
}

/// Negative 4: 4 workers (2x2 grid), y range of length 5 → 5 % 2 != 0
/// → `NonDivisible { axis: 'y' }`.
#[test]
fn negative_non_divisible_y_axis() {
    let acfg = build_2d_acfg("y", 7, 0..5, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = linked_with_blocks2d_directive("y");

    let err =
        apply_partition_blocks2d(&linked, acfg).expect_err("non-divisible y axis must be rejected");
    match err {
        PartitionBlocks2dError::NonDivisible {
            var,
            axis,
            lo,
            hi,
            cells,
        } => {
            assert_eq!(var, "y");
            assert_eq!(axis, 'y');
            assert_eq!(lo, 0);
            assert_eq!(hi, 5);
            assert_eq!(cells, 2); // grid_rows
        }
        other => panic!("expected NonDivisible(y), got {other:?}"),
    }
}

/// Negative 5: 4 workers (2x2 grid), x range of length 5 → 5 % 2 != 0
/// → `NonDivisible { axis: 'x' }`. (y must be divisible to reach the
/// x check.)
#[test]
fn negative_non_divisible_x_axis() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..5, &[1, 2, 3, 4]);
    let linked = linked_with_blocks2d_directive("y");

    let err =
        apply_partition_blocks2d(&linked, acfg).expect_err("non-divisible x axis must be rejected");
    match err {
        PartitionBlocks2dError::NonDivisible {
            var,
            axis,
            lo,
            hi,
            cells,
        } => {
            assert_eq!(var, "y");
            assert_eq!(axis, 'x');
            assert_eq!(lo, 0);
            assert_eq!(hi, 5);
            assert_eq!(cells, 2); // grid_cols
        }
        other => panic!("expected NonDivisible(x), got {other:?}"),
    }
}

// --------------------------------------------------------------------
// Determinism + identity
// --------------------------------------------------------------------

/// Determinism: two runs of `apply_partition_blocks2d` on the same
/// ACFG produce byte-identical sidecar contents (BTreeMap discipline
/// + integer-only grid decomposition).
#[test]
fn partition_blocks2d_is_deterministic_across_runs() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = linked_with_blocks2d_directive("y");

    let a = apply_partition_blocks2d(&linked, acfg.clone()).expect("run a");
    let b = apply_partition_blocks2d(&linked, acfg).expect("run b");

    assert_eq!(
        a.partition_worker_ranges, b.partition_worker_ranges,
        "two runs of partition_blocks2d must produce identical sidecars"
    );
}

/// No-directive case: an ACFG with no `partition=blocks2d` directive
/// is passed through unchanged (empty sidecar).
#[test]
fn no_directive_is_identity() {
    let acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2, 3, 4]);
    let linked = LinkedIR {
        algo: Default::default(),
        sched: SchedIR::default(),
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers: Default::default(),
        data_consumers: Default::default(),
    };

    let after = apply_partition_blocks2d(&linked, acfg).expect("no directive must be identity");
    assert!(
        after.partition_worker_ranges.is_empty(),
        "no partition=blocks2d directive → empty sidecar"
    );
}

/// Composition: when partition_rows has already written into the
/// sidecar for some iter_var, partition_blocks2d on a DIFFERENT outer
/// iter_var should ADD to the sidecar without trampling the existing
/// entries. By grammar construction (at most one `partition=` per
/// loop) the IterVar keys are disjoint; this test pins that property
/// at the pass-composition layer.
#[test]
fn composition_does_not_trample_prior_partition_entries() {
    // Pre-populate with a synthetic non-overlapping prior entry on
    // IterVar(99) (simulating an already-recorded row-band from
    // partition_rows or partition_workers on a separate loop).
    let mut acfg = build_2d_acfg("y", 7, 0..16, "x", 8, 0..32, &[1, 2, 3, 4]);
    let mut prior: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    prior.insert(WorkerId(1), 0..10);
    acfg.partition_worker_ranges.insert(IterVar(99), prior);

    let linked = linked_with_blocks2d_directive("y");
    let after = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d must accept");

    // Prior entry survives.
    let surviving = after.partition_worker_ranges.get(&IterVar(99)).unwrap();
    assert_eq!(surviving.get(&WorkerId(1)), Some(&(0..10)));
    // New entries land for outer + inner.
    assert!(after.partition_worker_ranges.contains_key(&IterVar(7)));
    assert!(after.partition_worker_ranges.contains_key(&IterVar(8)));
}
