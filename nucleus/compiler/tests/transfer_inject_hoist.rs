//! Integration tests for per-tile Push/Wait hoisting in the
//! transfer-injection pass (TASK-0143).
//!
//! Background
//! ----------
//! PRD §6.3.3 says `block=N` -- "transfers happen per tile". The
//! block-transform pass (TASK-0030) rewrites a `for VAR : LO..HI`
//! into a two-level `(VAR__tile, VAR)` nest, but the transfer-
//! injection pass at TASK-0018 emits one Push/Wait pair per
//! intra-tile iteration -- N times more than necessary for the
//! pattern PRD §8 describes.
//!
//! This file pins the contract that transfer-injection now hoists
//! Push/Wait pairs out of the inner intra-tile loop body up to the
//! enclosing per-tile body sequence, so each transfer fires once
//! per tile.
//!
//! Strategy
//! --------
//! Build a synthetic ACFG matching the post-block-transform shape:
//!
//! ```text
//! Sequence(top)
//!   Operation(producer on host)        // produces D
//!   Repeat(y__tile in 0..4)
//!     Sequence(per-tile body)
//!       Repeat(y in 0..4, block_inner)
//!         Sequence(intra-tile body)
//!           Operation(consumer on w0)  // reads D
//! ```
//!
//! After `inject_transfers`, the Wait should be in the per-tile body
//! sequence (immediately before the inner Repeat), NOT inside the
//! inner Repeat's body sequence. The matching Push lands in the
//! top-level sequence after the producer Operation. Total Push/Wait
//! pairs: one (one Push at top level, one Wait at per-tile granularity)
//! -- NOT one per intra-tile iteration.
//!
//! Honest limitations the tests acknowledge
//! ---------------------------------------
//! - The hoist is structural, not access-pattern-aware: every
//!   non-locally-produced Wait inside a block-inner intra-tile loop
//!   hoists. A schedule that wanted intra-tile-granularity Push/Wait
//!   (e.g. to pipeline) would have to opt out of `block=`. PRD §6.3.3
//!   does not currently offer such an opt-out; if a real example
//!   demands it, the hoist needs an off-switch.
//! - 2D blocking strips trailing inner-block axes from the hoisted
//!   tile, but only contiguous trailing axes. Interleaved
//!   block/non-block axes are not supported (no example exercises
//!   that today).
//! - These tests do NOT pin the matching Push location for hoisted
//!   Waits whose producer lives further up than the immediate parent
//!   of the inner Repeat -- the existing transfer-injection pass
//!   only splices Pushes within the same sequence as the Wait. The
//!   hoist gets us one level up; a Push at the top level for a Wait
//!   in the per-tile body still requires the global-Push pass that
//!   the original transfer_inject docs flagged as a follow-up.
//!
//! What this file does NOT cover
//! ----------------------------
//! - Real example 05/07 byte-for-byte assertions; the e2e files cover
//!   those.
//! - Mixed `block=` and `vectorize=` or `unroll=`; those transforms
//!   don't exist yet.

use std::collections::{BTreeMap, BTreeSet};

use compiler::acfg::{
    ACFGNode, DataflowDag, DataflowEdge, Operation, XferPlaceholder, XferRole, ACFG,
};
use compiler::event::{DataId, IterVar, KernelId, WorkerId};
use compiler::link::{LinkedIR, WorkerEntity};
use compiler::passes::transfer_inject::inject_transfers;
use compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Synthetic helpers
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
            edges: vec![DataflowEdge {
                data_in: data_in.into_iter().map(DataId).collect(),
                kernel: kid,
                data_out: data_out.map(DataId),
            }],
        },
    })
}

fn synthetic_linked_ir(producers: &[(&str, &[&str])], transfers_src: &str) -> LinkedIR {
    let mut data_producers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    for (data_name, worker_names) in producers {
        let entity = WorkerEntity(worker_names.iter().map(|s| (*s).to_string()).collect());
        data_producers.insert((*data_name).to_string(), entity);
    }

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

/// Helper: count how many `Xfer` placeholders with the given role
/// occur directly as children of the given `ACFGNode::Sequence`. Does
/// NOT recurse into sub-Repeat / sub-Sequence bodies.
fn xfers_directly_in_seq(node: &ACFGNode, role: XferRole) -> usize {
    match node {
        ACFGNode::Sequence(children) => children
            .iter()
            .filter(|c| matches!(c, ACFGNode::Xfer(x) if x.role == role))
            .count(),
        _ => 0,
    }
}

/// Helper: drill into the shape
/// `Sequence(top) -> Repeat(outer) -> Sequence(per-tile) -> Repeat(inner) -> Sequence(intra-tile)`
/// and return references to (top_seq, per_tile_seq, intra_tile_seq).
fn split_blocked_shape(root: &ACFGNode) -> (&ACFGNode, &ACFGNode, &ACFGNode) {
    let top = root;
    let outer_repeat = match root {
        ACFGNode::Sequence(children) => children
            .iter()
            .find(|c| matches!(c, ACFGNode::Repeat { .. }))
            .expect("top sequence has an outer Repeat"),
        _ => panic!("root not Sequence"),
    };
    let per_tile = match outer_repeat {
        ACFGNode::Repeat { body, .. } => &**body,
        _ => panic!("outer not Repeat"),
    };
    let inner_repeat = match per_tile {
        ACFGNode::Sequence(children) => children
            .iter()
            .find(|c| matches!(c, ACFGNode::Repeat { .. }))
            .expect("per-tile seq has inner Repeat"),
        _ => panic!("per-tile not Sequence"),
    };
    let intra_tile = match inner_repeat {
        ACFGNode::Repeat { body, .. } => &**body,
        _ => panic!("inner not Repeat"),
    };
    (top, per_tile, intra_tile)
}

// --------------------------------------------------------------------
// Hoist test: Wait moves out of intra-tile loop body to per-tile body
// --------------------------------------------------------------------

#[test]
fn wait_hoists_out_of_block_inner_intra_tile_loop() {
    // Build by hand a post-block-transform ACFG:
    //   Sequence(top)
    //     producer on host (writes d)
    //     Repeat(y__tile=1 in 0..4)
    //       Sequence(per-tile body)
    //         Repeat(y=0 in 0..4, block_inner)
    //           Sequence(intra-tile body)
    //             consumer on w0 (reads d)
    let producer = op(&[0], 100, vec![], Some(0)); // host writes d
    let consumer = op(&[1], 101, vec![0], Some(1)); // w0 reads d

    let intra_tile_body = ACFGNode::Sequence(vec![consumer]);
    let inner_repeat = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(intra_tile_body),
    };
    let per_tile_body = ACFGNode::Sequence(vec![inner_repeat]);
    let outer_repeat = ACFGNode::Repeat {
        iter_var: IterVar(1),
        range: 0..4,
        body: Box::new(per_tile_body),
    };
    let root = ACFGNode::Sequence(vec![producer, outer_repeat]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("c".into(), DataId(1));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));
    // Critical: mark IterVar(0) (the inner intra-tile loop) so
    // transfer_inject hoists its Push/Wait. This is the marker the
    // block_transform pass installs in real ACFGs (TASK-0143).
    let mut inner_block_iter_vars: BTreeSet<IterVar> = BTreeSet::new();
    inner_block_iter_vars.insert(IterVar(0));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars,
    };

    let linked = synthetic_linked_ir(&[("d", &["host"])], "transfer d : sync;");
    let result = inject_transfers(&linked, acfg);

    // ---- Assertion 1: exactly one Wait globally.
    // Before hoisting we would have seen one Wait per intra-tile
    // iteration (one Wait inside the inner Repeat body). Hoisting
    // collapses that to one Wait at per-tile granularity.
    //
    // Push count is NOT asserted: the existing transfer-injection
    // pass only splices Pushes when the producer Op lives in the
    // same Sequence as the Wait (see transfer_inject.rs
    // `splice_pushes_for_waits`). With the consumer hoisted out of
    // the inner body and the producer at top level, the Push is left
    // un-spliced -- which matches the pre-TASK-0143 baseline for any
    // example where producer and consumer sit in different
    // sequences (e.g. example 02). Filed as a separate follow-up
    // (cross-sequence Push splicing); the pthreads-sync codegen
    // doesn't consume Push placeholders today.
    assert_eq!(
        result.wait_count(),
        1,
        "exactly one Wait (hoisted out of intra-tile body)"
    );

    // ---- Assertion 2: the Wait is in the per-tile body sequence,
    // NOT in the intra-tile body sequence.
    let (_top_seq, per_tile_seq, intra_tile_seq) = split_blocked_shape(&result.root);
    assert_eq!(
        xfers_directly_in_seq(intra_tile_seq, XferRole::Wait),
        0,
        "no Waits remain inside the intra-tile (inner) sequence"
    );
    assert_eq!(
        xfers_directly_in_seq(per_tile_seq, XferRole::Wait),
        1,
        "exactly one Wait in the per-tile (outer-tile body) sequence"
    );

    // ---- Assertion 3: the hoisted Wait's tile does NOT mention the
    // inner intra-tile iter var (IterVar(0)). It may still mention
    // the outer tile var (IterVar(1)).
    let xfers = result.root.collect_xfers();
    let wait = xfers
        .iter()
        .find(|x| x.role == XferRole::Wait)
        .expect("one Wait");
    for (iv, _) in &wait.tile.bounds {
        assert_ne!(
            *iv,
            IterVar(0),
            "hoisted Wait tile must not name the block-inner iter var"
        );
    }
}

// --------------------------------------------------------------------
// Non-block loop: whole-symbol hoist (TASK-0136 / TASK-0139)
// --------------------------------------------------------------------

#[test]
fn loop_invariant_wait_hoists_out_of_plain_loops() {
    // Same shape as the block-inner test but NEITHER Repeat is marked
    // block-inner. The consumed datum `d` is produced by the
    // top-level producer and is NOT written inside either loop, so it
    // is loop-invariant: a whole-symbol transfer that must cross the
    // worker boundary ONCE, not once per (outer x inner) iteration.
    //
    // Before TASK-0136/0139 the Wait stayed trapped inside the inner
    // loop body with no matching Push at all — the net deadlocked
    // (acfg_to_petri unrolls the body, so N consumers raced one
    // absent producer). The fix (Pass A whole-symbol hoist + Pass B
    // global Push finaliser) lifts the single Wait to the top-level
    // sequence and splices the matching Push after the producer.
    let producer = op(&[0], 100, vec![], Some(0));
    let consumer = op(&[1], 101, vec![0], Some(1));

    let intra_body = ACFGNode::Sequence(vec![consumer]);
    let inner = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(intra_body),
    };
    let outer_body = ACFGNode::Sequence(vec![inner]);
    let outer = ACFGNode::Repeat {
        iter_var: IterVar(1),
        range: 0..4,
        body: Box::new(outer_body),
    };
    let root = ACFGNode::Sequence(vec![producer, outer]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("c".into(), DataId(1));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        // NOT marked: inner_block_iter_vars stays empty -> the
        // non-blocked whole-symbol hoist path applies.
        inner_block_iter_vars: BTreeSet::new(),
    };

    let linked = synthetic_linked_ir(&[("d", &["host"])], "transfer d : sync;");
    let result = inject_transfers(&linked, acfg);

    // Still exactly one Wait and one Push (whole-symbol: one crossing
    // for the loop-invariant datum, not one per iteration).
    assert_eq!(result.wait_count(), 1, "one whole-symbol Wait");
    assert_eq!(result.push_count(), 1, "one matching whole-symbol Push");

    let (top, per_tile_seq, intra_tile_seq) = split_blocked_shape(&result.root);
    // The Wait is hoisted clear out of BOTH loops to the top-level
    // sequence (its producer lives there).
    assert_eq!(
        xfers_directly_in_seq(top, XferRole::Wait),
        1,
        "loop-invariant Wait hoisted to the top-level sequence"
    );
    assert_eq!(
        xfers_directly_in_seq(intra_tile_seq, XferRole::Wait),
        0,
        "no Wait left inside the inner loop body"
    );
    assert_eq!(
        xfers_directly_in_seq(per_tile_seq, XferRole::Wait),
        0,
        "no Wait left at the outer loop body either"
    );
    // The Push sits at the top level too (right after the producer).
    assert_eq!(
        xfers_directly_in_seq(top, XferRole::Push),
        1,
        "matching Push spliced after the top-level producer"
    );
}

// --------------------------------------------------------------------
// Idempotence: re-running the pass on a hoisted ACFG is a no-op
// --------------------------------------------------------------------

#[test]
fn hoisting_is_idempotent() {
    let producer = op(&[0], 100, vec![], Some(0));
    let consumer = op(&[1], 101, vec![0], Some(1));
    let intra_body = ACFGNode::Sequence(vec![consumer]);
    let inner = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(intra_body),
    };
    let outer_body = ACFGNode::Sequence(vec![inner]);
    let outer = ACFGNode::Repeat {
        iter_var: IterVar(1),
        range: 0..4,
        body: Box::new(outer_body),
    };
    let root = ACFGNode::Sequence(vec![producer, outer]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("c".into(), DataId(1));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));
    let mut inner_block_iter_vars: BTreeSet<IterVar> = BTreeSet::new();
    inner_block_iter_vars.insert(IterVar(0));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars,
    };

    let linked = synthetic_linked_ir(&[("d", &["host"])], "transfer d : sync;");
    let once = inject_transfers(&linked, acfg);
    let push1 = once.push_count();
    let wait1 = once.wait_count();
    let twice = inject_transfers(&linked, once);
    assert_eq!(twice.push_count(), push1, "Push count stable on re-run");
    assert_eq!(twice.wait_count(), wait1, "Wait count stable on re-run");
}

// --------------------------------------------------------------------
// 2D blocking: nested block-inner loops still hoist
// --------------------------------------------------------------------

#[test]
fn nested_block_inner_hoists_to_outermost_per_tile() {
    // Shape modelling example 07's blocked schedule but with a
    // cross-worker dataflow added so transfers actually fire:
    //
    //   Sequence(top)
    //     producer on host (writes d)
    //     Repeat(i__tile)
    //       Sequence(per-i-tile body)
    //         Repeat(i, block_inner)
    //           Sequence(intra-i-tile body)
    //             Repeat(j__tile)
    //               Sequence(per-j-tile body)
    //                 Repeat(j, block_inner)
    //                   Sequence(intra-j-tile body)
    //                     consumer on w0 (reads d)
    //
    // The Wait should hoist out of BOTH block-inner loops to the
    // per-i-tile body OR higher. We assert it is no longer inside
    // the intra-j-tile or intra-i-tile bodies.
    let producer = op(&[0], 100, vec![], Some(0));
    let consumer = op(&[1], 101, vec![0], Some(1));

    let intra_j = ACFGNode::Sequence(vec![consumer]);
    let inner_j = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(intra_j),
    };
    let per_j_tile = ACFGNode::Sequence(vec![inner_j]);
    let outer_j = ACFGNode::Repeat {
        iter_var: IterVar(2),
        range: 0..4,
        body: Box::new(per_j_tile),
    };
    let intra_i = ACFGNode::Sequence(vec![outer_j]);
    let inner_i = ACFGNode::Repeat {
        iter_var: IterVar(1),
        range: 0..4,
        body: Box::new(intra_i),
    };
    let per_i_tile = ACFGNode::Sequence(vec![inner_i]);
    let outer_i = ACFGNode::Repeat {
        iter_var: IterVar(3),
        range: 0..4,
        body: Box::new(per_i_tile),
    };
    let root = ACFGNode::Sequence(vec![producer, outer_i]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("c".into(), DataId(1));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));
    let mut inner_block_iter_vars: BTreeSet<IterVar> = BTreeSet::new();
    inner_block_iter_vars.insert(IterVar(0)); // inner j
    inner_block_iter_vars.insert(IterVar(1)); // inner i

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars,
    };

    let linked = synthetic_linked_ir(&[("d", &["host"])], "transfer d : sync;");
    let result = inject_transfers(&linked, acfg);

    // Single Wait globally (per-i-tile granularity). Push count is
    // not asserted -- see the cross-sequence note in the simple
    // hoist test above.
    assert_eq!(
        result.wait_count(),
        1,
        "one Wait at per-tile granularity for 2D blocking"
    );

    // The hoisted Wait's tile must not name either inner-block iter
    // var.
    let xfers = result.root.collect_xfers();
    let wait = xfers
        .iter()
        .find(|x| x.role == XferRole::Wait)
        .expect("one Wait");
    for (iv, _) in &wait.tile.bounds {
        assert_ne!(*iv, IterVar(0), "Wait tile must not name inner j");
        assert_ne!(*iv, IterVar(1), "Wait tile must not name inner i");
    }
}

// --------------------------------------------------------------------
// Same-sequence Push/Wait: no hoist (in-sequence rendezvous)
// --------------------------------------------------------------------

#[test]
fn in_intra_tile_producer_consumer_is_not_hoisted() {
    // Both producer and consumer are inside the intra-tile body.
    // The Wait's data IS produced locally, so the hoist forwarding
    // rule (which only forwards Waits whose producer is NOT in the
    // current sequence) leaves the Wait in the inner body. This
    // pins the loop-invariance proxy: locally-produced data isn't
    // loop-invariant w.r.t. the inner loop.
    let producer = op(&[0], 100, vec![], Some(0)); // host writes d
    let consumer = op(&[1], 101, vec![0], None); // w0 reads d
    let intra = ACFGNode::Sequence(vec![producer, consumer]);
    let inner = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(intra),
    };
    let outer_body = ACFGNode::Sequence(vec![inner]);
    let outer = ACFGNode::Repeat {
        iter_var: IterVar(1),
        range: 0..4,
        body: Box::new(outer_body),
    };
    let root = ACFGNode::Sequence(vec![outer]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));
    let mut inner_block_iter_vars: BTreeSet<IterVar> = BTreeSet::new();
    inner_block_iter_vars.insert(IterVar(0));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars,
    };

    let linked = synthetic_linked_ir(&[("d", &["host"])], "transfer d : sync;");
    let result = inject_transfers(&linked, acfg);

    // The Wait stays in the intra-tile body because the producer is
    // also there.
    let (_, per_tile_seq, intra_tile_seq) = split_blocked_shape(&result.root);
    assert_eq!(
        xfers_directly_in_seq(intra_tile_seq, XferRole::Wait),
        1,
        "Wait stays inside inner body when its producer is co-resident"
    );
    assert_eq!(
        xfers_directly_in_seq(per_tile_seq, XferRole::Wait),
        0,
        "no hoist when producer is in the same intra-tile sequence"
    );
}

// --------------------------------------------------------------------
// block_transform pipeline: real end-to-end on example 02 with a
// synthetic `block=` directive on `i`
// --------------------------------------------------------------------

#[test]
fn block_transform_marks_inner_iter_var() {
    // Pipe example 02-split-add through link -> build_acfg ->
    // apply_block_transforms with a `block=128` directive on `i`
    // and assert the inner iter var is recorded in
    // `inner_block_iter_vars`. This pins block_transform's contract
    // with transfer_inject -- the marker must be set or the hoist
    // never fires.
    use compiler::algo::{lower_algo, parse_algo};
    use compiler::link;
    use compiler::sched::{
        lower_sched, parse_sched, ResolvedLoopDirective, ResolvedLoopOption, SchedIR,
    };
    use compiler::{apply_block_transforms, build_acfg};

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let algo_src =
        std::fs::read_to_string(repo_root.join("nuc-nucleus/examples/02-split-add/prog.algo.nuc"))
            .expect("read algo");
    let sched_src = std::fs::read_to_string(
        repo_root.join("nuc-nucleus/examples/02-split-add/schedules/split.sched.nuc"),
    )
    .expect("read sched");

    let algo = lower_algo(&parse_algo(&algo_src).expect("parse")).expect("lower");
    let mut sched: SchedIR = lower_sched(&parse_sched(&sched_src).expect("parse")).expect("lower");
    // 256 / 128 = 2 -> divisibility check passes.
    sched.loops.insert(
        "i".to_string(),
        ResolvedLoopDirective {
            var: "i".to_string(),
            options: vec![ResolvedLoopOption::Block(128)],
        },
    );
    let linked: LinkedIR = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked);
    let i_id = *acfg.name_iter_vars.get("i").expect("i iter var present");
    let after = apply_block_transforms(&linked, acfg).expect("block-transform OK");
    assert!(
        after.inner_block_iter_vars.contains(&i_id),
        "block_transform must mark the (inner) i iter var as block-inner"
    );

    // And after running transfer_inject, the marker should round-trip
    // unchanged (it is part of the ACFG state, not consumed).
    let after = inject_transfers(&linked, after);
    assert!(
        after.inner_block_iter_vars.contains(&i_id),
        "transfer_inject must forward the inner_block_iter_vars marker"
    );
}

// Silence unused-import warnings: XferPlaceholder is currently only
// referenced inside the test bodies via the field accesses, which
// rustc tracks through the type system. Explicit `use` keeps the
// file's dependency list visible for human review.
#[allow(dead_code)]
fn _silence_xfer_placeholder() -> Option<XferPlaceholder> {
    None
}

// --------------------------------------------------------------------
// TASK-0136 / TASK-0139: whole-symbol cross-scope finalisation on the
// real (ungated, non-block) path — structural idempotence + the
// example-02 shape end to end on a synthetic ACFG.
// --------------------------------------------------------------------

/// Build the example-02-split shape synthetically:
///
/// ```text
/// Sequence(top)
///   Operation(load_a  on host)            // writes a
///   Operation(load_b  on host)            // writes b
///   Repeat(i in 0..4)
///     Sequence
///       Operation(add on w0, reads a,b)   // writes c
///   Operation(save on host, reads c)
/// ```
///
/// `inner_block_iter_vars` is empty, so Pass A (whole-symbol Wait
/// hoist) and Pass B (global Push finaliser) actually run — this is
/// the path the gated synthetic hoist tests above never exercise.
fn example_02_shape() -> (ACFG, LinkedIR) {
    let load_a = op(&[0], 100, vec![], Some(0)); // host: a
    let load_b = op(&[0], 101, vec![], Some(1)); // host: b
    let add = op(&[1], 102, vec![0, 1], Some(2)); // w0: c <- add(a,b)
    let save = op(&[0], 103, vec![2], None); // host: save(c)

    let loop_i = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(ACFGNode::Sequence(vec![add])),
    };
    let root = ACFGNode::Sequence(vec![load_a, load_b, loop_i, save]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("a".into(), DataId(0));
    name_data.insert("b".into(), DataId(1));
    name_data.insert("c".into(), DataId(2));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars: BTreeSet::new(),
    };
    let linked = synthetic_linked_ir(
        &[("a", &["host"]), ("b", &["host"]), ("c", &["w0"])],
        "transfer a : sync;\n    transfer b : sync;\n    transfer c : sync;",
    );
    (acfg, linked)
}

#[test]
fn example_02_shape_pairs_every_wait_with_a_push() {
    let (acfg, linked) = example_02_shape();
    let result = inject_transfers(&linked, acfg);

    // a, b (loop-invariant inputs) cross once; c (loop output read
    // after the loop) crosses once. Three matched whole-symbol pairs.
    assert_eq!(result.wait_count(), 3, "one Wait per crossing symbol");
    assert_eq!(result.push_count(), 3, "every Wait has a matching Push");

    // Every Push shares its seq with exactly one Wait (matched pair).
    let xs = result.root.collect_xfers();
    for x in xs.iter().filter(|x| x.role == XferRole::Push) {
        let mates = xs
            .iter()
            .filter(|y| {
                y.role == XferRole::Wait
                    && y.seq == x.seq
                    && y.src == x.src
                    && y.dst == x.dst
                    && y.data == x.data
            })
            .count();
        assert_eq!(
            mates, 1,
            "Push seq {:?} must have exactly one Wait peer",
            x.seq
        );
    }

    // a, b Waits are hoisted clear out of the loop to the top-level
    // sequence; no Wait remains inside the Repeat body.
    let top_waits = xfers_directly_in_seq(&result.root, XferRole::Wait);
    assert!(
        top_waits >= 2,
        "loop-invariant a,b Waits hoisted to top (got {top_waits})"
    );
}

#[test]
fn mixed_block_and_nonblock_program_pairs_the_nonblock_transfer() {
    // TASK-0151: the cross-scope finaliser is PER-SUBTREE, not a
    // whole-program switch. A program containing BOTH a block-governed
    // Repeat nest (transfer of `d`) AND an unrelated non-blocked `for`
    // loop with a loop-invariant cross-worker read (transfer of `e`)
    // must still fully pair `e`. The block nest stays opaque (its
    // per-tile Push is TASK-0149's job) and must NOT be collapsed.
    //
    //   Sequence(top)
    //     load_d on host                       (writes d)
    //     load_e on host                       (writes e)
    //     Repeat(tile j)                        outer tile (NOT block-inner)
    //       Repeat(inner k, BLOCK-INNER)
    //         consume_d on w0 reads d           (writes f)
    //     Repeat(plain i)                       plain non-block loop
    //       consume_e on w0 reads e             (writes g)
    let load_d = op(&[0], 100, vec![], Some(0)); // host: d
    let load_e = op(&[0], 101, vec![], Some(1)); // host: e
    let consume_d = op(&[1], 102, vec![0], Some(2)); // w0: f <- d
    let consume_e = op(&[1], 103, vec![1], Some(3)); // w0: g <- e

    let inner_block = ACFGNode::Repeat {
        iter_var: IterVar(3),
        range: 0..2,
        body: Box::new(ACFGNode::Sequence(vec![consume_d])),
    };
    let tile_loop = ACFGNode::Repeat {
        iter_var: IterVar(2),
        range: 0..2,
        body: Box::new(ACFGNode::Sequence(vec![inner_block])),
    };
    let plain_loop = ACFGNode::Repeat {
        iter_var: IterVar(4),
        range: 0..4,
        body: Box::new(ACFGNode::Sequence(vec![consume_e])),
    };
    let root = ACFGNode::Sequence(vec![load_d, load_e, tile_loop, plain_loop]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("e".into(), DataId(1));
    name_data.insert("f".into(), DataId(2));
    name_data.insert("g".into(), DataId(3));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));
    let mut inner_block_iter_vars: BTreeSet<IterVar> = BTreeSet::new();
    inner_block_iter_vars.insert(IterVar(3)); // only the inner k loop

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars,
    };
    let linked = synthetic_linked_ir(
        &[("d", &["host"]), ("e", &["host"])],
        "transfer d : sync;\n    transfer e : sync;",
    );
    let result = inject_transfers(&linked, acfg);
    let xs = result.root.collect_xfers();

    // The non-block transfer `e` (DataId 1) is fully paired: one Wait
    // and a matching Push sharing its seq.
    let e_wait = xs
        .iter()
        .find(|x| x.role == XferRole::Wait && x.data == DataId(1))
        .expect("e must have a Wait");
    let e_push = xs
        .iter()
        .find(|x| x.role == XferRole::Push && x.data == DataId(1));
    assert!(
        e_push.is_some(),
        "non-block transfer e must get a Push even though a block nest coexists"
    );
    assert_eq!(
        e_push.unwrap().seq,
        e_wait.seq,
        "e Push/Wait must be a matched pair"
    );

    // The block-governed transfer `d` (DataId 0) is left to TASK-0149:
    // Pass B must NOT have collapsed/finalised it (no whole-symbol
    // Push spliced for d). It still has its HoistSink-placed Wait.
    assert!(
        xs.iter()
            .any(|x| x.role == XferRole::Wait && x.data == DataId(0)),
        "d's block-path Wait must still be present"
    );
    assert!(
        !xs.iter()
            .any(|x| x.role == XferRole::Push && x.data == DataId(0)),
        "Pass B must NOT splice a whole-symbol Push for the block-governed d \
         (that is TASK-0149's per-tile job)"
    );
}

#[test]
#[should_panic(expected = "no producing Operation")]
fn malformed_acfg_wait_without_producer_op_panics() {
    // TASK-0152: a cross-worker Wait is only emitted because the
    // schedule records a producer for the symbol, so a producing
    // Operation MUST exist in a well-formed ACFG. Feeding a tree where
    // the producer kernel was never lowered (consumer present, no
    // producer Operation) must fail LOUD with context, not silently
    // leave an unpaired Wait for a downstream pass to mis-diagnose.
    //
    //   Sequence(top)
    //     consume_d on w0 reads d   (no producer Operation for d)
    // data_producers says d is produced on host -> a cross-worker
    // Wait is emitted -> Pass B finds no producer -> panic.
    let consume_d = op(&[1], 102, vec![0], Some(1));
    let root = ACFGNode::Sequence(vec![consume_d]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("f".into(), DataId(1));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars: BTreeSet::new(),
    };
    let linked = synthetic_linked_ir(&[("d", &["host"])], "transfer d : sync;");
    // Should panic inside splice_pushes_global (TASK-0152 invariant).
    let _ = inject_transfers(&linked, acfg);
}

#[test]
fn block_nested_in_plain_loop_strands_the_invariant_wait() {
    // PINNING TEST for the documented TASK-0151 over-approximation
    // ("Block-entangled non-block transfers are stranded" in the
    // module docs). `contains_block_inner` taints a Repeat as soon as
    // its subtree CONTAINS a block-inner loop, so a non-block,
    // genuinely loop-invariant cross-worker Wait that lives inside the
    // SAME plain loop as a block nest is left unpaired (no whole-symbol
    // Push) — it falls to TASK-0149. This asserts the CURRENT
    // conservative behaviour on purpose: when TASK-0149/0150 makes the
    // classification per-Wait, this test must visibly flip (d gets a
    // Push) and should be updated then.
    //
    //   Sequence(top)
    //     load_d on host                       (writes d=0)
    //     load_e on host                       (writes e=1)
    //     Repeat(plain i)                       contains a block nest
    //       consume_d on w0 reads d            (writes f=2)  <- stranded
    //       Repeat(tile j)
    //         Repeat(inner k, BLOCK-INNER)
    //           consume_e on w0 reads e        (writes g=3)
    let load_d = op(&[0], 100, vec![], Some(0));
    let load_e = op(&[0], 101, vec![], Some(1));
    let consume_d = op(&[1], 102, vec![0], Some(2));
    let consume_e = op(&[1], 103, vec![1], Some(3));

    let inner_block = ACFGNode::Repeat {
        iter_var: IterVar(3),
        range: 0..2,
        body: Box::new(ACFGNode::Sequence(vec![consume_e])),
    };
    let tile_loop = ACFGNode::Repeat {
        iter_var: IterVar(2),
        range: 0..2,
        body: Box::new(ACFGNode::Sequence(vec![inner_block])),
    };
    let plain_loop = ACFGNode::Repeat {
        iter_var: IterVar(5),
        range: 0..4,
        body: Box::new(ACFGNode::Sequence(vec![consume_d, tile_loop])),
    };
    let root = ACFGNode::Sequence(vec![load_d, load_e, plain_loop]);

    let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
    name_data.insert("d".into(), DataId(0));
    name_data.insert("e".into(), DataId(1));
    name_data.insert("f".into(), DataId(2));
    name_data.insert("g".into(), DataId(3));
    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("host".into(), WorkerId(0));
    name_workers.insert("w0".into(), WorkerId(1));
    let mut inner_block_iter_vars: BTreeSet<IterVar> = BTreeSet::new();
    inner_block_iter_vars.insert(IterVar(3));

    let acfg = ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars,
    };
    let linked = synthetic_linked_ir(
        &[("d", &["host"]), ("e", &["host"])],
        "transfer d : sync;\n    transfer e : sync;",
    );
    let result = inject_transfers(&linked, acfg);
    let xs = result.root.collect_xfers();

    // d's Wait exists but is STRANDED (no whole-symbol Push) because
    // the enclosing plain loop is tainted by the block nest. Pinned
    // limitation — see module docs.
    assert!(
        xs.iter()
            .any(|x| x.role == XferRole::Wait && x.data == DataId(0)),
        "d's Wait should still be present (emitted, just not finalised)"
    );
    assert!(
        !xs.iter()
            .any(|x| x.role == XferRole::Push && x.data == DataId(0)),
        "TASK-0151 over-approximation: d is block-entangled so Pass B does \
         NOT pair it. If this flips, TASK-0149/0150 improved the gate — \
         update this pinning test."
    );
}

#[test]
fn mixed_block_nonblock_tree_is_structurally_idempotent() {
    // Finding 3 of the TASK-0151 review: idempotence must hold on the
    // mixed block+nonblock tree, not only the pure non-block path.
    let build = || {
        let load_d = op(&[0], 100, vec![], Some(0));
        let load_e = op(&[0], 101, vec![], Some(1));
        let consume_d = op(&[1], 102, vec![0], Some(2));
        let consume_e = op(&[1], 103, vec![1], Some(3));
        let inner_block = ACFGNode::Repeat {
            iter_var: IterVar(3),
            range: 0..2,
            body: Box::new(ACFGNode::Sequence(vec![consume_d])),
        };
        let tile_loop = ACFGNode::Repeat {
            iter_var: IterVar(2),
            range: 0..2,
            body: Box::new(ACFGNode::Sequence(vec![inner_block])),
        };
        let plain_loop = ACFGNode::Repeat {
            iter_var: IterVar(4),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![consume_e])),
        };
        let root = ACFGNode::Sequence(vec![load_d, load_e, tile_loop, plain_loop]);
        let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
        name_data.insert("d".into(), DataId(0));
        name_data.insert("e".into(), DataId(1));
        name_data.insert("f".into(), DataId(2));
        name_data.insert("g".into(), DataId(3));
        let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
        name_workers.insert("host".into(), WorkerId(0));
        name_workers.insert("w0".into(), WorkerId(1));
        let mut ibiv: BTreeSet<IterVar> = BTreeSet::new();
        ibiv.insert(IterVar(3));
        ACFG {
            root,
            name_kernels: BTreeMap::new(),
            name_data,
            name_workers,
            name_iter_vars: BTreeMap::new(),
            inner_block_iter_vars: ibiv,
        }
    };
    let linked = synthetic_linked_ir(
        &[("d", &["host"]), ("e", &["host"])],
        "transfer d : sync;\n    transfer e : sync;",
    );
    let once = inject_transfers(&linked, build());
    let twice = inject_transfers(&linked, once.clone());
    let thrice = inject_transfers(&linked, twice.clone());
    assert_eq!(
        once, twice,
        "mixed tree: inject_transfers must be idempotent"
    );
    assert_eq!(twice, thrice, "mixed tree: idempotence stable on re-run");
}

#[test]
fn whole_symbol_finalisation_is_structurally_idempotent() {
    // AC#3: re-running inject_transfers yields a structurally
    // identical tree. This exercises the ungated Pass A + Pass B
    // recursive paths on re-entry (Repeat present, block markers
    // empty) — the case the existing flat / block-gated idempotence
    // tests do NOT cover.
    let (acfg, linked) = example_02_shape();
    let once = inject_transfers(&linked, acfg);
    let twice = inject_transfers(&linked, once.clone());
    let thrice = inject_transfers(&linked, twice.clone());

    assert_eq!(
        once, twice,
        "inject_transfers must be idempotent on the whole-symbol path"
    );
    assert_eq!(
        twice, thrice,
        "idempotence must be stable across further re-runs"
    );
}
