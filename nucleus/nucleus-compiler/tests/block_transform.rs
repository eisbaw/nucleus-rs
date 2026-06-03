//! Integration tests for the `block=N` loop transformation pass
//! (TASK-0030).
//!
//! The pass is exercised at three levels:
//!
//! 1. **Synthetic ACFG** — hand-build a single-loop ACFG with a
//!    schedule carrying `block=N` and assert the resulting tree has
//!    the expected (outer tile-loop, inner intra-tile loop) shape.
//! 2. **End-to-end on an existing example with no `block=`** —
//!    re-use the e2e fixture for example 01 to assert the pass is a
//!    pure identity when no `block=` directive is present. This
//!    pins the "existing examples must stay bit-identical" claim
//!    (PRD §10 differential test).
//! 3. **Error cases** — non-divisible bounds and unknown loop vars
//!    both surface a `BlockTransformError` with the offending name.
//!
//! What this file deliberately does NOT cover (follow-ups):
//! - End-to-end build of example 05 (stencil) with `block=64` —
//!   example 05's algorithm currently fails to parse (TASK-0078).
//!   Once that lands, the AC #3 "EventList Push covers a 64-row
//!   band" assertion goes here.
//! - Interaction with `unroll=`, `reuse`, `pipeline=`, `partition=`
//!   — most of those transforms have landed since this file was
//!   written; `unroll=` itself is filed as TASK-0293 (future work).
//!   `vectorize=` was deliberately removed from the language
//!   (TASK-0292, 2026-05-25) — SIMD is delegated to the host Rust
//!   compiler. Once `unroll=` lands, this file gains
//!   "block + unroll" combination tests.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
use nucleus_compiler::algo::{lower_algo, parse_algo, AlgoIR};
use nucleus_compiler::apply_block_transforms;
use nucleus_compiler::event::{DataId, IterVar, KernelId, WorkerId};
use nucleus_compiler::link;
use nucleus_compiler::link::LinkedIR;
use nucleus_compiler::passes::block_transform::BlockTransformError;
use nucleus_compiler::sched::{
    lower_sched, parse_sched, ResolvedLoopDirective, ResolvedLoopOption, SchedIR,
};

// --------------------------------------------------------------------
// Helpers
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
    link(algo, sched).expect("link must succeed")
}

/// Build a minimal synthetic ACFG with one outer `for y : 0..H` loop
/// containing one Operation. The schedule on top has whatever
/// `loops` directives the caller injects via `with_block`.
///
/// Returns the linked IR (so the pass can read `sched.loops`) and the
/// matching pre-transform ACFG.
fn synthetic_one_loop(h: i64, with_block: Option<u64>) -> (LinkedIR, ACFG) {
    // We piggy-back on example 01's algorithm to avoid hand-rolling
    // an AlgoIR — it has an `i` loop over `0..256`. We just rebind
    // the schedule.
    let algo_ast =
        parse_algo(&read_example("01-elementwise-add/prog.algo.nuc")).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(
        "01-elementwise-add/schedules/naive.sched.nuc",
    ))
    .expect("sched parse");
    let mut sched: SchedIR = lower_sched(&sched_ast).expect("sched lower");
    if let Some(n) = with_block {
        // Drop a `loop i : block=N` directive into the schedule's
        // loops map. The lower-pass keyed it by var name; we use the
        // same key here.
        sched.loops.insert(
            "i".to_string(),
            ResolvedLoopDirective {
                var: "i".to_string(),
                options: vec![ResolvedLoopOption::Block(n)],
                // TASK-0099: hand-built test fixture has no source text.
                var_span: None,
            },
        );
    }
    let linked = link(algo, sched).expect("link must succeed");
    let acfg = nucleus_compiler::build_acfg(&linked).expect("build_acfg");

    // The synthetic example always uses the example-01 algo, so the
    // `h` parameter is informational only. We assert the loop bound
    // matches.
    let _ = h;
    (linked, acfg)
}

/// Collect (in tree order) every `Repeat` node whose `iter_var`
/// matches `target`, recursing through `Sequence` children and into
/// `Repeat` bodies that themselves don't match. Used to find the
/// full-tile and trailing-partial-tile nests regardless of where the
/// rewritten `Sequence` sits in the surrounding program tree.
fn collect_repeats_with_var<'a>(node: &'a ACFGNode, target: IterVar, out: &mut Vec<&'a ACFGNode>) {
    match node {
        ACFGNode::Repeat { iter_var, body, .. } => {
            if *iter_var == target {
                out.push(node);
                // Do not descend further: the inner intra-tile loop
                // carries a different iter var, so it can't match
                // `target` (the tile id) anyway.
            } else {
                collect_repeats_with_var(body, target, out);
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_repeats_with_var(c, target, out);
            }
        }
        _ => {}
    }
}

/// Find the first `Repeat` matching `iter_var` in the tree. Helper
/// for assertions on the post-transform shape.
fn first_repeat_with_var(node: &ACFGNode, target: IterVar) -> Option<&ACFGNode> {
    match node {
        ACFGNode::Repeat { iter_var, body, .. } => {
            if *iter_var == target {
                Some(node)
            } else {
                first_repeat_with_var(body, target)
            }
        }
        ACFGNode::Sequence(children) => children
            .iter()
            .find_map(|c| first_repeat_with_var(c, target)),
        _ => None,
    }
}

// --------------------------------------------------------------------
// 1. Synthetic ACFG: block=N rewrites a single loop into nest.
// --------------------------------------------------------------------

#[test]
fn block_rewrites_to_outer_tile_and_inner() {
    // example 01's algorithm loop is `for i : 0..256`. Block by 64
    // -> 4 tiles of 64.
    let (linked, acfg_before) = synthetic_one_loop(256, Some(64));

    let pre_repeats = acfg_before.repeat_count();
    let pre_depth = acfg_before.max_repeat_depth();
    assert_eq!(pre_repeats, 1, "pre-transform sanity");
    assert_eq!(pre_depth, 1);

    let acfg = apply_block_transforms(&linked, acfg_before).expect("block-transform OK");

    // Post-transform: nesting depth 2, two Repeat nodes.
    assert_eq!(acfg.repeat_count(), 2, "outer + inner Repeat");
    assert_eq!(acfg.max_repeat_depth(), 2, "two-level nest");

    // The original iter var `i` still exists; a new `i__tile` was
    // added.
    let i_id = *acfg.name_iter_vars.get("i").expect("i kept");
    let tile_id = *acfg.name_iter_vars.get("i__tile").expect("i__tile added");
    assert_ne!(i_id, tile_id);

    // The outer Repeat is the i__tile one; inner is `i`.
    let outer = first_repeat_with_var(&acfg.root, tile_id).expect("outer present");
    if let ACFGNode::Repeat {
        iter_var: outer_var,
        range: outer_range,
        body,
        ..
    } = outer
    {
        assert_eq!(*outer_var, tile_id);
        assert_eq!(*outer_range, 0..4, "256/64 = 4 tiles");
        // Walk into body to find inner Repeat with iter `i`.
        match &**body {
            ACFGNode::Sequence(children) => {
                assert_eq!(children.len(), 1, "outer body wraps single inner Repeat");
                match &children[0] {
                    ACFGNode::Repeat {
                        iter_var: inner_var,
                        range: inner_range,
                        ..
                    } => {
                        assert_eq!(*inner_var, i_id);
                        assert_eq!(*inner_range, 0..64, "intra-tile chunk size");
                    }
                    other => panic!("expected inner Repeat, got {other:?}"),
                }
            }
            other => panic!("outer body must be Sequence, got {other:?}"),
        }
    } else {
        panic!("outer node must be Repeat");
    }

    // Operation count is preserved (block transform doesn't drop ops).
    assert_eq!(
        acfg.operation_count(),
        4,
        "load, load_b, add(in loop), save"
    );
}

#[test]
fn block_is_identity_when_no_block_directive() {
    // No `block=` -> output ACFG must be the input ACFG.
    let (linked, acfg) = synthetic_one_loop(256, None);
    let before = acfg.clone();
    let after = apply_block_transforms(&linked, acfg).expect("identity OK");
    assert_eq!(before, after, "no block= -> identity transform");
}

// --------------------------------------------------------------------
// 2. End-to-end on existing examples (no block= used in M2 required
//    schedules).
// --------------------------------------------------------------------

#[test]
fn examples_01_02_03_unchanged_by_block_transform() {
    // The required M2 schedules of examples 01, 02, 03 have no
    // `block=` directives — the pass must be the identity on them.
    let cases: &[(&str, &str)] = &[
        (
            "01-elementwise-add/prog.algo.nuc",
            "01-elementwise-add/schedules/naive.sched.nuc",
        ),
        (
            "02-split-add/prog.algo.nuc",
            "02-split-add/schedules/naive.sched.nuc",
        ),
        (
            "02-split-add/prog.algo.nuc",
            "02-split-add/schedules/split.sched.nuc",
        ),
        (
            "03-reduction/prog.algo.nuc",
            "03-reduction/schedules/naive.sched.nuc",
        ),
    ];
    for (algo, sched) in cases {
        let linked = linked_from_paths(algo, sched);
        let acfg = nucleus_compiler::build_acfg(&linked).expect("build_acfg");
        let before = acfg.clone();
        let after =
            apply_block_transforms(&linked, acfg).expect("identity for examples without block=");
        assert_eq!(
            before, after,
            "{algo} + {sched}: pass must be identity (no block= present)"
        );
    }
}

// --------------------------------------------------------------------
// 3. Error cases
// --------------------------------------------------------------------

/// TASK-0142: a non-divisible range is no longer rejected — it is
/// rewritten into `Sequence[ full-tile nest, trailing partial tile ]`.
///
/// example 01's loop is `0..256`; `block=100` -> 2 full tiles of 100
/// (`num_full = 256 / 100 = 2`) plus a trailing partial tile of
/// `256 % 100 = 56`.
#[test]
fn block_non_divisible_emits_trailing_partial_tile() {
    let (linked, acfg) = synthetic_one_loop(256, Some(100));
    let acfg = apply_block_transforms(&linked, acfg)
        .expect("non-divisible range is supported (TASK-0142)");

    let i_id = *acfg.name_iter_vars.get("i").expect("i kept");
    let tile_id = *acfg.name_iter_vars.get("i__tile").expect("i__tile added");

    // The rewritten `Sequence[full-nest, partial-nest]` replaces the
    // original `Repeat(i,...)` in place inside example-01's program
    // tree, so collect the tile nests recursively rather than
    // assuming they sit at the root.
    let mut tile_nests: Vec<&ACFGNode> = Vec::new();
    collect_repeats_with_var(&acfg.root, tile_id, &mut tile_nests);
    assert_eq!(
        tile_nests.len(),
        2,
        "full-tile nest + trailing partial tile, got {tile_nests:?}"
    );

    let assert_nest = |node: &ACFGNode, outer: std::ops::Range<i64>, inner_len: i64| {
        if let ACFGNode::Repeat {
            iter_var: ov,
            range: or,
            body,
            ..
        } = node
        {
            assert_eq!(*ov, tile_id);
            assert_eq!(*or, outer, "outer tile-loop range");
            match &**body {
                ACFGNode::Sequence(inner) => match &inner[0] {
                    ACFGNode::Repeat {
                        iter_var: iv,
                        range: ir,
                        ..
                    } => {
                        assert_eq!(*iv, i_id, "inner keeps the source iter var");
                        assert_eq!(*ir, 0..inner_len, "intra-tile chunk size");
                    }
                    o => panic!("inner not Repeat: {o:?}"),
                },
                o => panic!("nest body not Sequence: {o:?}"),
            }
        } else {
            panic!("nest not a Repeat: {node:?}");
        }
    };
    assert_nest(tile_nests[0], 0..2, 100); // 2 full tiles of 100
    assert_nest(tile_nests[1], 0..1, 56); // trailing partial tile

    // Operation count preserved (each nest carries a copy of the body).
    // example 01: load_a, load_b, add(in loop), save -> the in-loop op
    // is duplicated across the two nests, so 3 non-loop + 2 in-loop = 5.
    assert!(
        acfg.operation_count() >= 4,
        "block transform must not drop operations (got {})",
        acfg.operation_count()
    );
}

/// TASK-0142 AC#2 (structure, on the real `apply_block_transforms`
/// pipeline): a single non-divisible `block=N` produces an outer
/// "tile" sequence of `num_full + 1` nests — full tiles first, then
/// one trailing partial tile whose inner length is the remainder.
///
/// `synthetic_one_loop` is pinned to example-01's `0..256`. With
/// `block=200`: `num_full = 256 / 200 = 1` full tile of 200 plus a
/// trailing partial tile of `256 % 200 = 56` — exactly the AC#2
/// shape ("outer loop of 2 tiles, inner X for tile 0, inner Y for
/// tile 1"). The exact `block=64`-on-`0..100` -> 64-then-36 numbers
/// are pinned by the `rewrite_node_ac2_64_then_36` unit test in
/// `passes::block_transform` (it can use an arbitrary range; this
/// helper cannot).
#[test]
fn block_non_divisible_two_tiles_full_then_partial() {
    let (linked, acfg) = synthetic_one_loop(256, Some(200));
    let acfg = apply_block_transforms(&linked, acfg).expect("non-divisible supported (TASK-0142)");

    let i_id = *acfg.name_iter_vars.get("i").expect("i kept");
    let tile_id = *acfg.name_iter_vars.get("i__tile").expect("i__tile added");

    let mut nests: Vec<&ACFGNode> = Vec::new();
    collect_repeats_with_var(&acfg.root, tile_id, &mut nests);
    assert_eq!(
        nests.len(),
        2,
        "an outer loop of 2 tiles (1 full + 1 partial)"
    );

    let inner_of = |node: &ACFGNode| -> (std::ops::Range<i64>, std::ops::Range<i64>) {
        if let ACFGNode::Repeat {
            range: or, body, ..
        } = node
        {
            if let ACFGNode::Sequence(inner) = &**body {
                if let ACFGNode::Repeat {
                    iter_var: iv,
                    range: ir,
                    ..
                } = &inner[0]
                {
                    assert_eq!(*iv, i_id, "inner keeps source iter var");
                    return (or.clone(), ir.clone());
                }
            }
        }
        panic!("unexpected nest shape: {node:?}");
    };
    let (o0, i0) = inner_of(nests[0]);
    let (o1, i1) = inner_of(nests[1]);
    assert_eq!(o0, 0..1, "tile 0: 1 full tile");
    assert_eq!(i0, 0..200, "inner of 200 for the full tile");
    assert_eq!(o1, 0..1, "tile 1: single trailing partial tile");
    assert_eq!(i1, 0..56, "inner of 56 for the partial tile (256 - 200)");
}

#[test]
fn block_rejects_unknown_loop_var() {
    // Construct: take example 01's linked IR but inject a `block=`
    // for a variable that doesn't exist in the algorithm.
    let algo_ast = parse_algo(&read_example("01-elementwise-add/prog.algo.nuc")).unwrap();
    let algo = lower_algo(&algo_ast).unwrap();
    let sched_ast = parse_sched(&read_example(
        "01-elementwise-add/schedules/naive.sched.nuc",
    ))
    .unwrap();
    let mut sched = lower_sched(&sched_ast).unwrap();
    sched.loops.insert(
        "no_such_var".to_string(),
        ResolvedLoopDirective {
            var: "no_such_var".to_string(),
            options: vec![ResolvedLoopOption::Block(8)],
            // TASK-0099: hand-built test fixture has no source text.
            var_span: None,
        },
    );
    // The link pass would normally reject this; we bypass it by
    // calling `link` directly and acknowledging the pass must fail
    // closed if invoked on a desynced pair. NB: `link` here may
    // itself reject — if it does, this test trivially passes the
    // "doesn't reach the block-transform pass" path.
    let link_result = link(algo, sched);
    if let Ok(linked) = link_result {
        let acfg = nucleus_compiler::build_acfg(&linked).expect("build_acfg");
        let err = apply_block_transforms(&linked, acfg).expect_err("unknown loop var -> reject");
        match err {
            BlockTransformError::UnknownLoopVar { var } => {
                assert_eq!(var, "no_such_var");
            }
            other => panic!("expected UnknownLoopVar, got {other:?}"),
        }
    }
    // If link rejected, the pass never runs — also acceptable
    // behaviour (linker catches it earlier).
}

// --------------------------------------------------------------------
// 4. Determinism
// --------------------------------------------------------------------

#[test]
fn block_transform_is_deterministic() {
    let (linked1, acfg1) = synthetic_one_loop(256, Some(64));
    let (linked2, acfg2) = synthetic_one_loop(256, Some(64));
    let out1 = apply_block_transforms(&linked1, acfg1).unwrap();
    let out2 = apply_block_transforms(&linked2, acfg2).unwrap();
    assert_eq!(out1, out2, "same input -> same output");
}

// --------------------------------------------------------------------
// 5. Use unused helpers / imports so clippy doesn't flag them.
// --------------------------------------------------------------------

#[test]
fn smoke_synthetic_acfg_is_buildable() {
    // Helper exercised here so it doesn't go dead.
    let mut workers = BTreeSet::new();
    workers.insert(WorkerId(0));
    let op = ACFGNode::Operation(Operation {
        kernel: KernelId(0),
        workers,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(
                vec![DataId(0)],
                KernelId(0),
                Some(DataId(1)),
            )],
        },
    });
    let body = ACFGNode::Sequence(vec![op]);
    let repeat = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..8,
        body: Box::new(body),
        block_tag: None,
        break_cond: None,
    };
    assert_eq!(repeat.count_repeats(), 1);

    // Quiet unused warnings for items genuinely used only here.
    let _ = AlgoIR::default();
    let _: BTreeMap<String, ()> = BTreeMap::new();
}
