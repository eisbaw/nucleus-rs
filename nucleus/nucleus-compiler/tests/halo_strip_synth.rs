//! Integration tests for the TASK-0289 cycle 114a halo-strip Push/Wait
//! synthesis pass (an internal helper inside `transfer_inject`, but
//! exercised externally via `inject_transfers` which is the
//! pass's public entry point).
//!
//! Pins:
//!
//! - **Positive (3x3 grid, halo=1)**: 9 workers in a 3x3 grid; the
//!   center worker (w5, row=1 col=1) gets exactly 4 new Push/Wait
//!   pairs (N, S, W, E); each edge cell gets 3; each corner cell gets
//!   2. The (src, dst, data) triples and tile ranges are pinned
//!   exactly.
//! - **Positive (2x2 grid, halo=1)**: 4 workers; every worker is a
//!   corner; each gets exactly 2 new pairs (one to each adjacent
//!   neighbour).
//! - **AC#3 short-circuit**: an ACFG with empty `partition_pairs`
//!   produces no halo-strip pairs (additive-only contract — every
//!   shipped schedule today has empty pairs).
//! - **Determinism**: two runs produce byte-identical ACFG.
//!
//! Strategy: hand-built synthetic ACFG mirroring
//! `tests/partition_blocks2d.rs::build_2d_acfg`, with the partition
//! sidecar AND halo_widths populated up front (skipping the
//! partition_blocks2d / halo_inference passes to keep the test focused
//! on synthesis behaviour). The `inject_transfers` entry point is
//! then called directly — its halo-strip-synthesis step is the last
//! finalisation in the chain, so the input shape of the rest of the
//! tree does not affect what we measure.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{
    ACFGNode, DataflowDag, DataflowEdge, Operation, XferRole, ACFG,
};
use nucleus_compiler::event::{DataId, IterVar, KernelId, WorkerId};
use nucleus_compiler::link::LinkedIR;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::SchedIR;

// --------------------------------------------------------------------
// Fixture builders
// --------------------------------------------------------------------

/// Build a synthetic 2D Repeat-of-Repeat ACFG with the partition +
/// halo sidecars populated up front. Body is a single Operation on
/// `body_workers` reading `data_id` produced by a kernel with id
/// `kernel_id`.
///
/// The partition sidecar is populated EXACTLY as
/// `apply_partition_blocks2d` would have written it for a 2D grid
/// of `body_workers.len()` workers — so this fixture mirrors the
/// post-partition_blocks2d ACFG shape without invoking that pass.
///
/// `halo_widths[kernel_id][outer_iv] = halo_y` and
/// `halo_widths[kernel_id][inner_iv] = halo_x` are populated; the
/// in-tree halo_inference pass would have inferred these from the
/// kernel's access pattern, but here we skip that step and pin the
/// values directly.
fn build_2d_acfg_with_partition_and_halo(
    grid_rows: usize,
    grid_cols: usize,
    outer_range: std::ops::Range<i64>,
    inner_range: std::ops::Range<i64>,
    halo_y: u64,
    halo_x: u64,
) -> ACFG {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let kernel_id = KernelId(42);
    let data_id = DataId(99);

    // Workers numbered 1..=N (matches partition_blocks2d test fixture).
    let n_workers = grid_rows * grid_cols;
    let body_workers: BTreeSet<WorkerId> = (1..=(n_workers as u64)).map(WorkerId).collect();

    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".to_string(), WorkerId(0));
    for w in &body_workers {
        name_workers.insert(format!("w{}", w.0), *w);
    }

    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("y".to_string(), outer_iv);
    name_iter_vars.insert("x".to_string(), inner_iv);

    let mut name_kernels: BTreeMap<String, KernelId> = BTreeMap::new();
    name_kernels.insert("k".to_string(), kernel_id);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".to_string(), data_id);

    // Inner-body Operation that reads `data_id` on the partitioned
    // workers. We intentionally DO NOT thread the producer entity
    // through `LinkedIR.data_producers` — that would trigger the
    // regular (non-halo-strip) Push/Wait fan-out and make the
    // Xfer-counting assertions noisy. The halo-strip synthesis path
    // we are testing reads only the ACFG sidecars; it does not need
    // `data_producers` to fire, and it does not need a producer
    // Operation in the ACFG either — the synthesised Push/Wait pairs
    // are appended AFTER `hoist_invariant_waits` + `splice_pushes_global`,
    // so the hoist's "escape with no producing Operation" panic
    // (line ~358 / ~1451 in transfer_inject.rs) is unreachable for
    // them. Empirically verified TASK-0289 review-hardening cycle:
    // removing the previously-included `load_op` here changes nothing.
    let body_op = ACFGNode::Operation(Operation {
        kernel: kernel_id,
        workers: body_workers.clone(),
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(vec![data_id], kernel_id, None)],
        },
    });

    let inner = ACFGNode::Repeat {
        iter_var: inner_iv,
        range: inner_range.clone(),
        body: Box::new(ACFGNode::Sequence(vec![body_op])),
        block_tag: None,
    };
    let outer = ACFGNode::Repeat {
        iter_var: outer_iv,
        range: outer_range.clone(),
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
    };

    // Partition sidecar: per-worker y-band (from outer_iv) + x-band
    // (from inner_iv), mirroring apply_partition_blocks2d's writes.
    let y_slice = (outer_range.end - outer_range.start) / (grid_rows as i64);
    let x_slice = (inner_range.end - inner_range.start) / (grid_cols as i64);
    let mut per_worker_y: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    let mut per_worker_x: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    for (i, &wid) in body_workers.iter().enumerate() {
        let row = (i / grid_cols) as i64;
        let col = (i % grid_cols) as i64;
        let y_lo = outer_range.start + row * y_slice;
        let y_hi = y_lo + y_slice;
        let x_lo = inner_range.start + col * x_slice;
        let x_hi = x_lo + x_slice;
        per_worker_y.insert(wid, y_lo..y_hi);
        per_worker_x.insert(wid, x_lo..x_hi);
    }
    let mut partition_worker_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> =
        BTreeMap::new();
    partition_worker_ranges.insert(outer_iv, per_worker_y);
    partition_worker_ranges.insert(inner_iv, per_worker_x);

    // partition_pairs + grid_shape_for_outer_iv: as
    // apply_partition_blocks2d would have written.
    let mut partition_pairs: BTreeMap<IterVar, IterVar> = BTreeMap::new();
    partition_pairs.insert(outer_iv, inner_iv);
    let mut grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)> = BTreeMap::new();
    grid_shape_for_outer_iv.insert(outer_iv, (grid_rows as u32, grid_cols as u32));

    // halo_widths: per-(kernel, iv). `halo_inference` would record
    // these from the kernel's access pattern; we pin the values
    // directly.
    let mut halo_widths: BTreeMap<KernelId, BTreeMap<IterVar, u64>> = BTreeMap::new();
    let mut per_iv: BTreeMap<IterVar, u64> = BTreeMap::new();
    if halo_y > 0 {
        per_iv.insert(outer_iv, halo_y);
    }
    if halo_x > 0 {
        per_iv.insert(inner_iv, halo_x);
    }
    if !per_iv.is_empty() {
        halo_widths.insert(kernel_id, per_iv);
    }

    ACFG {
        // Root sequence: just the outer Repeat. No top-level loader
        // Operation is needed (see comment above next to body_op).
        root: ACFGNode::Sequence(vec![outer]),
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges,
        pipeline_depth_for_seq: BTreeMap::new(),
        halo_widths,
        reuse_widths: BTreeMap::new(),
        partition_pairs,
        grid_shape_for_outer_iv,
    }
}

/// Minimal LinkedIR with no schedule loops + no data_producers /
/// data_consumers. The halo-strip-synthesis path does NOT read
/// LinkedIR (it reads the ACFG sidecars exclusively); the other
/// inject_transfers paths are short-circuited by the empty
/// data_producers (no cross-worker dataflow ⇒ no Push/Wait emitted by
/// the regular path) and empty schedule.transfers (no policy
/// directives ⇒ all synthesised pairs inherit TransferPolicy::default).
fn empty_linked() -> LinkedIR {
    LinkedIR {
        algo: Default::default(),
        sched: SchedIR::default(),
        placements: Default::default(),
        kernel_workers: Default::default(),
        data_producers: Default::default(),
        data_consumers: Default::default(),
    }
}

// --------------------------------------------------------------------
// Counting helpers (over the post-inject_transfers ACFG)
// --------------------------------------------------------------------

/// Count Push/Wait pairs whose `dst` matches `worker`. Each
/// synthesised halo strip emits one Push and one Wait (with the same
/// SeqTag and src/dst); we report pair count = `wait_count(worker)`.
fn count_pairs_for_worker(acfg: &ACFG, worker: WorkerId) -> usize {
    let mut waits: usize = 0;
    let xfers = acfg.root.collect_xfers();
    for x in &xfers {
        if x.role == XferRole::Wait && x.dst == worker {
            waits += 1;
        }
    }
    waits
}

/// Find the unique Wait whose (src, dst, data) matches; panics if
/// zero or multiple match. Returns its tile.
fn unique_wait_tile(
    acfg: &ACFG,
    src: WorkerId,
    dst: WorkerId,
    data: DataId,
) -> nucleus_compiler::event::IterTile {
    let xfers = acfg.root.collect_xfers();
    let matched: Vec<_> = xfers
        .iter()
        .filter(|x| x.role == XferRole::Wait && x.src == src && x.dst == dst && x.data == data)
        .collect();
    assert_eq!(
        matched.len(),
        1,
        "expected exactly 1 Wait with src={src:?} dst={dst:?} data={data:?}; found {}",
        matched.len()
    );
    matched[0].tile.clone()
}

// --------------------------------------------------------------------
// Positive: 3x3 grid (halo=1) — corner / edge / center pair counts
// --------------------------------------------------------------------

/// AC#1: 9 workers in a 3x3 grid (halo_y = halo_x = 1).
///
/// Pinned per-worker neighbour count (N/S/E/W only, no corners):
///
/// ```text
///   w1 (0,0) corner  → 2 pairs (S, E)
///   w2 (0,1) edge    → 3 pairs (S, W, E)
///   w3 (0,2) corner  → 2 pairs (S, W)
///   w4 (1,0) edge    → 3 pairs (N, S, E)
///   w5 (1,1) center  → 4 pairs (N, S, W, E)
///   w6 (1,2) edge    → 3 pairs (N, S, W)
///   w7 (2,0) corner  → 2 pairs (N, E)
///   w8 (2,1) edge    → 3 pairs (N, W, E)
///   w9 (2,2) corner  → 2 pairs (N, W)
/// ```
///
/// Total pairs = 4*2 (corners) + 4*3 (edges) + 1*4 (center) = 24.
/// Each pair contributes 1 Push + 1 Wait ⇒ 48 new Xfer nodes total
/// (assuming the non-halo-strip injection paths add 0, which holds
/// for our synthetic LinkedIR with empty data_producers).
#[test]
fn positive_3x3_halo_1_per_worker_pair_counts() {
    let acfg = build_2d_acfg_with_partition_and_halo(
        3, 3,    // grid
        0..30,   // outer y range (divisible by 3)
        0..30,   // inner x range (divisible by 3)
        1, 1,    // halo widths
    );
    let linked = empty_linked();

    let after = inject_transfers(&linked, acfg);

    // Pin the per-worker pair counts.
    assert_eq!(count_pairs_for_worker(&after, WorkerId(1)), 2, "w1 (0,0) corner");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(2)), 3, "w2 (0,1) edge");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(3)), 2, "w3 (0,2) corner");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(4)), 3, "w4 (1,0) edge");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(5)), 4, "w5 (1,1) center");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(6)), 3, "w6 (1,2) edge");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(7)), 2, "w7 (2,0) corner");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(8)), 3, "w8 (2,1) edge");
    assert_eq!(count_pairs_for_worker(&after, WorkerId(9)), 2, "w9 (2,2) corner");

    // Aggregate: 24 Push + 24 Wait = 48 Xfers total.
    assert_eq!(after.push_count(), 24, "total Push count");
    assert_eq!(after.wait_count(), 24, "total Wait count");
    assert_eq!(after.xfer_count(), 48, "total Xfer count");
}

/// AC#1: 3x3 grid (halo=1) — the center worker w5 at (1,1) gets four
/// pairs, one per cardinal direction. Pin the exact (src, dst) pairing
/// AND the per-direction halo-strip tile.
///
/// Per-worker bands (3x3 over y=0..30, x=0..30, slice=10 each):
///   w5 (1,1): y_band=10..20, x_band=10..20
///   N from w2 (0,1): strip y=9..10, x=10..20
///   S from w8 (2,1): strip y=20..21, x=10..20
///   W from w4 (1,0): strip y=10..20, x=9..10
///   E from w6 (1,2): strip y=10..20, x=20..21
#[test]
fn positive_3x3_halo_1_center_worker_w5_pair_shapes() {
    let acfg = build_2d_acfg_with_partition_and_halo(3, 3, 0..30, 0..30, 1, 1);
    let linked = empty_linked();
    let after = inject_transfers(&linked, acfg);

    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);
    let w5 = WorkerId(5);

    // N: src=w2.
    let n_tile = unique_wait_tile(&after, WorkerId(2), w5, data);
    assert_eq!(n_tile.bounds, vec![(outer_iv, 9..10), (inner_iv, 10..20)]);

    // S: src=w8.
    let s_tile = unique_wait_tile(&after, WorkerId(8), w5, data);
    assert_eq!(s_tile.bounds, vec![(outer_iv, 20..21), (inner_iv, 10..20)]);

    // W: src=w4.
    let w_tile = unique_wait_tile(&after, WorkerId(4), w5, data);
    assert_eq!(w_tile.bounds, vec![(outer_iv, 10..20), (inner_iv, 9..10)]);

    // E: src=w6.
    let e_tile = unique_wait_tile(&after, WorkerId(6), w5, data);
    assert_eq!(e_tile.bounds, vec![(outer_iv, 10..20), (inner_iv, 20..21)]);
}

// --------------------------------------------------------------------
// Positive: 2x2 grid (halo=1) — every worker is a corner
// --------------------------------------------------------------------

/// AC#1: 4 workers in a 2x2 grid; every worker is a corner. Each
/// worker gets exactly 2 pairs (one to each adjacent in-grid neighbour).
/// Total = 4 * 2 = 8 pairs ⇒ 8 Push + 8 Wait = 16 Xfers.
///
/// Per-worker bands (2x2 over y=0..16, x=0..16, slice=8 each):
///   w1 (0,0): y_band=0..8,  x_band=0..8
///     S from w3: strip y=8..9,   x=0..8
///     E from w2: strip y=0..8,   x=8..9
///   w2 (0,1): y_band=0..8,  x_band=8..16
///     S from w4: strip y=8..9,   x=8..16
///     W from w1: strip y=0..8,   x=7..8
///   w3 (1,0): y_band=8..16, x_band=0..8
///     N from w1: strip y=7..8,   x=0..8
///     E from w4: strip y=8..16,  x=8..9
///   w4 (1,1): y_band=8..16, x_band=8..16
///     N from w2: strip y=7..8,   x=8..16
///     W from w3: strip y=8..16,  x=7..8
#[test]
fn positive_2x2_halo_1_corner_pair_shapes() {
    let acfg = build_2d_acfg_with_partition_and_halo(2, 2, 0..16, 0..16, 1, 1);
    let linked = empty_linked();
    let after = inject_transfers(&linked, acfg);

    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);

    // 4 corners * 2 pairs each = 8 pairs ⇒ 16 Xfers.
    assert_eq!(after.push_count(), 8, "8 Push (4 workers * 2 pairs)");
    assert_eq!(after.wait_count(), 8, "8 Wait (4 workers * 2 pairs)");
    assert_eq!(after.xfer_count(), 16);

    for w in 1..=4 {
        assert_eq!(
            count_pairs_for_worker(&after, WorkerId(w)),
            2,
            "w{w} corner gets 2 pairs"
        );
    }

    // Pin w1 (0,0): S from w3, E from w2.
    let s = unique_wait_tile(&after, WorkerId(3), WorkerId(1), data);
    assert_eq!(s.bounds, vec![(outer_iv, 8..9), (inner_iv, 0..8)]);
    let e = unique_wait_tile(&after, WorkerId(2), WorkerId(1), data);
    assert_eq!(e.bounds, vec![(outer_iv, 0..8), (inner_iv, 8..9)]);

    // Pin w4 (1,1): N from w2, W from w3.
    let n = unique_wait_tile(&after, WorkerId(2), WorkerId(4), data);
    assert_eq!(n.bounds, vec![(outer_iv, 7..8), (inner_iv, 8..16)]);
    let w = unique_wait_tile(&after, WorkerId(3), WorkerId(4), data);
    assert_eq!(w.bounds, vec![(outer_iv, 8..16), (inner_iv, 7..8)]);
}

// --------------------------------------------------------------------
// AC#3: empty partition_pairs short-circuit
// --------------------------------------------------------------------

/// AC#3: an ACFG with empty `partition_pairs` produces ZERO injected
/// Xfers from the halo-strip synthesis path — the additive-only
/// contract that keeps the existing 92/79/0/13/0 e2e baseline green.
///
/// Setup: same shape as `build_2d_acfg_with_partition_and_halo`
/// (partition_worker_ranges, halo_widths populated) but with
/// `partition_pairs` and `grid_shape_for_outer_iv` cleared. This
/// simulates the (impossible-by-construction-today, but defensive)
/// case where halo widths exist but no blocks2d directive ran. The
/// guard at the top of `inject_halo_strip_xfers` short-circuits on
/// empty pairs and emits no Xfers.
#[test]
fn empty_partition_pairs_emits_zero_halo_strip_xfers() {
    let mut acfg = build_2d_acfg_with_partition_and_halo(3, 3, 0..30, 0..30, 1, 1);
    // Clear the pair / grid sidecars — simulating the pre-cycle-113
    // shape (no blocks2d directive run) while keeping the halo widths
    // populated. The other sidecars (partition_worker_ranges,
    // halo_widths) are left as-is to stress that the short-circuit
    // keys ONLY off partition_pairs.
    acfg.partition_pairs.clear();
    acfg.grid_shape_for_outer_iv.clear();
    let linked = empty_linked();

    let after = inject_transfers(&linked, acfg);

    // Zero Xfers from the synthesis path. (The non-halo-strip paths
    // in inject_transfers also emit zero because data_producers is
    // empty in `linked`.)
    assert_eq!(
        after.xfer_count(),
        0,
        "AC#3 additive-only: empty partition_pairs ⇒ no halo-strip Xfers"
    );
}

// --------------------------------------------------------------------
// Determinism: two runs produce byte-identical ACFG
// --------------------------------------------------------------------

/// Two runs of `inject_transfers` on the same ACFG produce
/// byte-identical output. BTreeMap iteration discipline + a fixed
/// per-direction emit order (N, S, W, E) keep this so.
#[test]
fn halo_strip_synthesis_is_deterministic_across_runs() {
    let acfg = build_2d_acfg_with_partition_and_halo(3, 3, 0..30, 0..30, 1, 1);
    let linked = empty_linked();
    let a = inject_transfers(&linked, acfg.clone());
    let b = inject_transfers(&linked, acfg);
    assert_eq!(a, b, "two runs must produce identical ACFG");
}

// --------------------------------------------------------------------
// AC#2 placement (TASK-0290 cycle 114b): pairs land AFTER the producer
// --------------------------------------------------------------------

/// AC#2 (TASK-0290 cycle 114b): when the parent Sequence of the outer
/// Repeat contains a producing Operation (the host's load_image, in
/// the realistic case), the synthesised halo-strip Push/Wait pairs
/// land in the Sequence AFTER that Operation — not before.
///
/// **Why this matters**: a worker's emitted EventList orders its
/// Push/Wait pairs by source-tree order. Before TASK-0290 cycle 114b
/// the synthesised pairs were prepended to the front of the parent
/// Sequence, which placed each receiving worker's halo-strip `Wait`
/// BEFORE the host's `load_image` Operation — i.e. before the data
/// the matching `Push` is supposed to send had been produced on the
/// host. This was a real ordering defect (the bit-identical e2e cell
/// landing in the same cycle would have failed without this fix).
///
/// **Test shape**: same 2x2 grid as `positive_2x2_halo_1_corner_pair_shapes`,
/// but the root Sequence is `[load_op, outer_repeat]` instead of just
/// `[outer_repeat]`. After `inject_transfers`, the root Sequence must
/// have the synthesised Xfers BETWEEN load_op (index 0) and the outer
/// Repeat (now at index > number-of-synthesised-Xfers).
///
/// Pinned post-condition:
/// - root.children[0] is `load_op` (producer Operation)
/// - root.children[1..=8] are 8 synthesised Xfer nodes (4 workers * 2
///   pairs each; per-pair = Push+Wait, but they're both ACFGNode::Xfer
///   variants so we count = 16 Xfer entries — wait, that's wrong. Each
///   pair = 1 Push + 1 Wait = 2 Xfer nodes; 4 workers * 2 pairs = 8
///   pairs = 16 Xfer nodes).
/// - root.children.last() is the outer Repeat.
#[test]
fn positive_placement_after_producing_op() {
    // Build a 2x2 grid fixture with a top-level load_image-style
    // Operation that produces `data_id` on the host worker.
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let kernel_id = KernelId(42);
    let load_kernel_id = KernelId(43); // distinct from `kernel_id`
    let data_id = DataId(99);

    let host_worker = WorkerId(0);
    let body_workers: BTreeSet<WorkerId> = (1..=4u64).map(WorkerId).collect();

    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".to_string(), host_worker);
    for w in &body_workers {
        name_workers.insert(format!("w{}", w.0), *w);
    }
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("y".to_string(), outer_iv);
    name_iter_vars.insert("x".to_string(), inner_iv);
    let mut name_kernels: BTreeMap<String, KernelId> = BTreeMap::new();
    name_kernels.insert("blur3".to_string(), kernel_id);
    name_kernels.insert("load_image".to_string(), load_kernel_id);
    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("img".to_string(), data_id);

    // Producer Operation on host: writes `data_id`. Single host worker
    // in `workers` so `output_data` reports `Some(data_id)`.
    let mut host_only: BTreeSet<WorkerId> = BTreeSet::new();
    host_only.insert(host_worker);
    let load_op = ACFGNode::Operation(Operation {
        kernel: load_kernel_id,
        workers: host_only,
        dataflow: DataflowDag {
            // load_image: () -> img. data_in = []; data_out = Some(img).
            edges: vec![DataflowEdge::new(Vec::new(), load_kernel_id, Some(data_id))],
        },
    });

    let body_op = ACFGNode::Operation(Operation {
        kernel: kernel_id,
        workers: body_workers.clone(),
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(vec![data_id], kernel_id, None)],
        },
    });

    let inner = ACFGNode::Repeat {
        iter_var: inner_iv,
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![body_op])),
        block_tag: None,
    };
    let outer = ACFGNode::Repeat {
        iter_var: outer_iv,
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![inner])),
        block_tag: None,
    };

    // Partition sidecar: per-worker y-band + x-band, 2x2 grid over
    // 0..16 / 0..16, slice=8 each. Mirrors build_2d_acfg_with_partition_and_halo.
    let y_slice = 16 / 2;
    let x_slice = 16 / 2;
    let mut per_worker_y: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    let mut per_worker_x: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    for (i, &wid) in body_workers.iter().enumerate() {
        let row = (i / 2) as i64;
        let col = (i % 2) as i64;
        per_worker_y.insert(wid, (row * y_slice)..((row + 1) * y_slice));
        per_worker_x.insert(wid, (col * x_slice)..((col + 1) * x_slice));
    }
    let mut partition_worker_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> =
        BTreeMap::new();
    partition_worker_ranges.insert(outer_iv, per_worker_y);
    partition_worker_ranges.insert(inner_iv, per_worker_x);

    let mut partition_pairs: BTreeMap<IterVar, IterVar> = BTreeMap::new();
    partition_pairs.insert(outer_iv, inner_iv);
    let mut grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)> = BTreeMap::new();
    grid_shape_for_outer_iv.insert(outer_iv, (2, 2));

    let mut halo_widths: BTreeMap<KernelId, BTreeMap<IterVar, u64>> = BTreeMap::new();
    let mut per_iv: BTreeMap<IterVar, u64> = BTreeMap::new();
    per_iv.insert(outer_iv, 1);
    per_iv.insert(inner_iv, 1);
    halo_widths.insert(kernel_id, per_iv);

    // Root Sequence has the producer BEFORE the outer Repeat — this is
    // the ordering we want to validate the placement fix against.
    let acfg = ACFG {
        root: ACFGNode::Sequence(vec![load_op.clone(), outer]),
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges,
        pipeline_depth_for_seq: BTreeMap::new(),
        halo_widths,
        reuse_widths: BTreeMap::new(),
        partition_pairs,
        grid_shape_for_outer_iv,
    };

    let linked = empty_linked();
    let after = inject_transfers(&linked, acfg);

    // Pin: root is a Sequence with the producer at index 0, the
    // synthesised Xfers at indices 1..=N (where N = number of
    // synthesised Xfers), and the outer Repeat at the tail.
    let children = match &after.root {
        ACFGNode::Sequence(c) => c.clone(),
        other => panic!("root must be a Sequence; got {:?}", other),
    };
    assert!(
        matches!(&children[0], ACFGNode::Operation(op) if op.kernel == load_kernel_id),
        "root.children[0] must be the load_image producer Operation; got {:?}",
        &children[0]
    );
    assert!(
        matches!(children.last().unwrap(), ACFGNode::Repeat { iter_var, .. } if *iter_var == outer_iv),
        "root.children.last() must be the outer Repeat; got {:?}",
        children.last().unwrap()
    );

    // Count Xfers BETWEEN producer (index 0) and outer Repeat (tail).
    // 4 workers * 2 pairs each * 2 Xfer nodes per pair = 16.
    let xfer_count_in_middle = children[1..children.len() - 1]
        .iter()
        .filter(|c| matches!(c, ACFGNode::Xfer(_)))
        .count();
    assert_eq!(
        xfer_count_in_middle, 16,
        "expected 16 synthesised Xfer nodes between producer and outer Repeat \
         (4 workers * 2 pairs * 2 Xfer per pair); got {xfer_count_in_middle}"
    );
    // Every intermediate child must be an Xfer (no stray nodes).
    for (k, c) in children[1..children.len() - 1].iter().enumerate() {
        assert!(
            matches!(c, ACFGNode::Xfer(_)),
            "intermediate child at index {} must be an Xfer; got {:?}",
            k + 1,
            c
        );
    }

    // Per-pair STRUCTURAL invariants (TASK-0290 cycle 114b architect P1.1
    // hardening): the cardinality + outer-edge checks above are
    // necessary but not sufficient — a regression that swapped Push/Wait,
    // mis-routed neighbour ids, or corrupted strip tile bounds while
    // preserving the count would slip past. Mirror
    // `positive_2x2_halo_1_corner_pair_shapes` and pin (a) per-worker
    // pair count = 2 for every worker in the 2x2 grid, (b) exact
    // (src, dst, data, tile) for both pairs of two opposite corners
    // (w1 = (row=0, col=0) and w4 = (row=1, col=1)). 16 Xfer nodes
    // (8 pairs * 2 = 16) is the structural envelope, but the
    // per-strip-shape pins detect mis-routing.
    assert_eq!(after.push_count(), 8, "8 Push (4 workers * 2 pairs)");
    assert_eq!(after.wait_count(), 8, "8 Wait (4 workers * 2 pairs)");
    assert_eq!(after.xfer_count(), 16);
    for w in 1..=4 {
        assert_eq!(
            count_pairs_for_worker(&after, WorkerId(w)),
            2,
            "w{w} corner gets 2 pairs"
        );
    }
    // y-slice = 8, x-slice = 8 (2x2 grid over 0..16 x 0..16). Halo = 1.
    // w1 = (row=0, col=0): owns y in [0..8), x in [0..8). S-strip
    // received FROM w3 = (row=1, col=0): y ∈ [8..9), x ∈ [0..8).
    // E-strip received FROM w2 = (row=0, col=1): y ∈ [0..8), x ∈ [8..9).
    let s_w1 = unique_wait_tile(&after, WorkerId(3), WorkerId(1), data_id);
    assert_eq!(
        s_w1.bounds,
        vec![(outer_iv, 8..9), (inner_iv, 0..8)],
        "w1 S-strip tile from w3"
    );
    let e_w1 = unique_wait_tile(&after, WorkerId(2), WorkerId(1), data_id);
    assert_eq!(
        e_w1.bounds,
        vec![(outer_iv, 0..8), (inner_iv, 8..9)],
        "w1 E-strip tile from w2"
    );
    // w4 = (row=1, col=1): owns y in [8..16), x in [8..16). N-strip from
    // w2 = (row=0, col=1): y ∈ [7..8), x ∈ [8..16). W-strip from
    // w3 = (row=1, col=0): y ∈ [8..16), x ∈ [7..8).
    let n_w4 = unique_wait_tile(&after, WorkerId(2), WorkerId(4), data_id);
    assert_eq!(
        n_w4.bounds,
        vec![(outer_iv, 7..8), (inner_iv, 8..16)],
        "w4 N-strip tile from w2"
    );
    let w_w4 = unique_wait_tile(&after, WorkerId(3), WorkerId(4), data_id);
    assert_eq!(
        w_w4.bounds,
        vec![(outer_iv, 8..16), (inner_iv, 7..8)],
        "w4 W-strip tile from w3"
    );
}

// IDEMPOTENCE / RE-RUN CAVEAT (forward-carried to TASK-0290):
//
// `inject_transfers` claims idempotence ("re-running on the output
// yields the same tree structurally"). The pre-cycle-114a regular
// Push/Wait path satisfies this — pinned by
// `idempotent_on_synthetic_two_worker_case` in
// `tests/transfer_inject.rs`, which has empty partition_pairs and is
// short-circuited away from the halo-strip synthesis path by the
// AC#3 guard at the top of `inject_halo_strip_xfers`.
//
// The halo-strip synthesis path does NOT satisfy structural
// idempotence today:
//   - `rewrite_partition_tiles` clobbers strip tiles on the re-run
//     (its compute-worker rule replaces the strip with src's full
//     partition slice).
//   - `splice_pushes_for_waits` (in `inject_in_sequence`) splices a
//     NEW Push for every halo-strip Wait it sees in the second-pass
//     root sequence, because the existing Pushes are not in the
//     dedupe-window (they sit BEFORE the producer load Op, not
//     immediately after it).
//
// No production code path re-runs `inject_transfers` on its own
// output today (the driver pipeline calls it once). The
// shipped-schedule idempotence test stays green because
// partition_pairs is empty for every shipped schedule. The cleanup
// is forward-carried to TASK-0290 with two candidate fixes:
//   (a) make `rewrite_partition_tiles` skip Xfers where both
//       endpoints are partitioned workers (the halo-strip
//       signature) AND make `splice_pushes_for_waits` dedupe across
//       the full sequence instead of just the slot immediately
//       after the producer;
//   (b) tag synthesised halo-strip pairs structurally so subsequent
//       finalisation passes pass them through.
