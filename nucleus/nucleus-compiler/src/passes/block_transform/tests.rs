use super::*;
use crate::acfg::{DataflowDag, DataflowEdge, Operation};
use crate::event::{DataId, KernelId, WorkerId};
use std::collections::BTreeSet;

fn op() -> ACFGNode {
    let mut workers = BTreeSet::new();
    workers.insert(WorkerId(0));
    ACFGNode::Operation(Operation {
        kernel: KernelId(0),
        workers,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(
                vec![DataId(0)],
                KernelId(0),
                Some(DataId(1)),
            )],
        },
    })
}

#[test]
fn find_loop_range_walks_nested_seq() {
    let inner = ACFGNode::Repeat {
        iter_var: IterVar(7),
        range: 0..32,
        body: Box::new(ACFGNode::Sequence(vec![op()])),
        block_tag: None,
        break_cond: None,
    };
    let outer = ACFGNode::Sequence(vec![op(), inner]);
    assert_eq!(find_loop_range_by_id(&outer, IterVar(7)), Some((0, 32)));
    assert_eq!(find_loop_range_by_id(&outer, IterVar(99)), None);
}

#[test]
fn rewrite_node_passes_through_non_blocked() {
    let tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    let n = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![op()])),
        block_tag: None,
        break_cond: None,
    };
    let out = rewrite_node(n.clone(), &tile_map);
    assert_eq!(out, n);
}

#[test]
fn rewrite_node_blocks_match() {
    let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    tile_map.insert(IterVar(0), ("y__tile".to_string(), IterVar(5), 4));
    let n = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..16,
        body: Box::new(ACFGNode::Sequence(vec![op()])),
        block_tag: None,
        break_cond: None,
    };
    let out = rewrite_node(n, &tile_map);
    match out {
        ACFGNode::Repeat {
            iter_var: outer,
            range: outer_range,
            body,
            block_tag: outer_tag,
            break_cond: _,
        } => {
            assert_eq!(outer, IterVar(5));
            assert_eq!(outer_range, 0..4); // 16 / 4 = 4 tiles
                                           // The synthesised TILE loop carries no rebinding tag —
                                           // its variable never indexes the body (TASK-0180).
            assert_eq!(outer_tag, None, "tile loop must NOT be tagged");
            match *body {
                ACFGNode::Sequence(seq) => {
                    assert_eq!(seq.len(), 1);
                    match &seq[0] {
                        ACFGNode::Repeat {
                            iter_var: inner,
                            range: inner_range,
                            block_tag: inner_tag,
                            ..
                        } => {
                            assert_eq!(*inner, IterVar(0));
                            assert_eq!(*inner_range, 0..4); // chunk size
                                                            // The strip-mined INNER loop is tagged
                                                            // per-occurrence: divisible single nest
                                                            // => full (not partial), N=4, num_full=4
                                                            // (16/4). This is exactly what the
                                                            // backend rebinds `LO + tile*N + inner`
                                                            // from (TASK-0180 AC#1).
                            assert_eq!(
                                *inner_tag,
                                Some(BlockTag {
                                    block_n: 4,
                                    num_full: 4,
                                    is_partial: false,
                                }),
                                "divisible inner loop must carry full-nest BlockTag"
                            );
                        }
                        other => panic!("inner not Repeat: {other:?}"),
                    }
                }
                other => panic!("body not Sequence: {other:?}"),
            }
        }
        other => panic!("outer not Repeat: {other:?}"),
    }
}

/// TASK-0142: a non-divisible range produces `Sequence[ full-tile
/// nest, trailing-partial-tile nest ]`. Shape mirrors the
/// 05-stencil/blocked case (`for y : 1..15` is length 14, `block=4`
/// -> 3 full tiles of 4 + a trailing tile of 2).
#[test]
fn rewrite_node_emits_trailing_partial_tile() {
    let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    tile_map.insert(IterVar(0), ("y__tile".to_string(), IterVar(5), 4));
    // length 14 (e.g. range 1..15), block=4 -> num_full=3, rem=2.
    let n = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 1..15,
        body: Box::new(ACFGNode::Sequence(vec![op()])),
        block_tag: None,
        break_cond: None,
    };
    let out = rewrite_node(n, &tile_map);

    let seq = match out {
        ACFGNode::Sequence(s) => s,
        other => panic!("non-divisible must be a Sequence, got {other:?}"),
    };
    assert_eq!(seq.len(), 2, "full-tile nest + trailing partial tile");

    // Helper: assert a nest has outer (tile_id, outer_range), no
    // tag on the tile loop, and an inner (IterVar(0), 0..inner_len)
    // carrying exactly `expect_tag` (TASK-0180 / TASK-0173: the
    // non-divisible full and partial nests get DISTINCT
    // per-occurrence tags so the backend rebinds each correctly —
    // `LO+tile*N+inner` vs `LO+num_full*N+inner`).
    let check_nest = |node: &ACFGNode,
                      outer_range: std::ops::Range<i64>,
                      inner_len: i64,
                      expect_tag: BlockTag| {
        match node {
            ACFGNode::Repeat {
                iter_var: outer,
                range,
                body,
                block_tag: outer_tag,
                break_cond: _,
            } => {
                assert_eq!(*outer, IterVar(5), "outer is the tile var");
                assert_eq!(*range, outer_range);
                assert_eq!(*outer_tag, None, "tile loop must NOT be tagged");
                match &**body {
                    ACFGNode::Sequence(inner_seq) => {
                        assert_eq!(inner_seq.len(), 1);
                        match &inner_seq[0] {
                            ACFGNode::Repeat {
                                iter_var: inner,
                                range: ir,
                                block_tag: inner_tag,
                                ..
                            } => {
                                assert_eq!(*inner, IterVar(0), "inner keeps source var");
                                assert_eq!(*ir, 0..inner_len);
                                assert_eq!(
                                    *inner_tag,
                                    Some(expect_tag),
                                    "inner loop's per-occurrence BlockTag"
                                );
                            }
                            o => panic!("inner not Repeat: {o:?}"),
                        }
                    }
                    o => panic!("nest body not Sequence: {o:?}"),
                }
            }
            o => panic!("nest not Repeat: {o:?}"),
        }
    };

    // length 14, block=4 -> num_full=3, rem=2. Both nests carry
    // block_n=4 and num_full=3; only `is_partial` differs (it
    // selects the rebinding base in the backend).
    // Full tiles: 3 tiles of width 4 -> full-nest tag.
    check_nest(
        &seq[0],
        0..3,
        4,
        BlockTag {
            block_n: 4,
            num_full: 3,
            is_partial: false,
        },
    );
    // Trailing partial tile: a single (0..1) tile of width rem=2
    // -> partial tag (rebinds `LO + num_full*N + inner`).
    check_nest(
        &seq[1],
        0..1,
        2,
        BlockTag {
            block_n: 4,
            num_full: 3,
            is_partial: true,
        },
    );
}

/// TASK-0142 AC#2 verbatim: `block=64` on `0..100` produces an
/// outer loop of 2 tiles, an inner of 64 for tile 0, and an inner
/// of 36 for tile 1.
#[test]
fn rewrite_node_ac2_64_then_36() {
    let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    tile_map.insert(IterVar(0), ("y__tile".to_string(), IterVar(9), 64));
    let n = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..100,
        body: Box::new(ACFGNode::Sequence(vec![op()])),
        block_tag: None,
        break_cond: None,
    };
    let out = rewrite_node(n, &tile_map);
    let seq = match out {
        ACFGNode::Sequence(s) => s,
        o => panic!("expected Sequence (non-divisible), got {o:?}"),
    };
    // "an outer loop of 2 tiles": the full-tile nest (tile 0) and
    // the trailing partial tile (tile 1), in order.
    assert_eq!(seq.len(), 2, "an outer loop of 2 tiles");

    let inner = |node: &ACFGNode| -> std::ops::Range<i64> {
        match node {
            ACFGNode::Repeat {
                iter_var: ov, body, ..
            } => {
                assert_eq!(*ov, IterVar(9), "outer is the tile var");
                match &**body {
                    ACFGNode::Sequence(s) => match &s[0] {
                        ACFGNode::Repeat {
                            iter_var: iv,
                            range,
                            ..
                        } => {
                            assert_eq!(*iv, IterVar(0));
                            range.clone()
                        }
                        o => panic!("inner not Repeat: {o:?}"),
                    },
                    o => panic!("body not Sequence: {o:?}"),
                }
            }
            o => panic!("nest not Repeat: {o:?}"),
        }
    };
    // 100 / 64 = 1 full tile of 64 (tile 0); 100 % 64 = 36 (tile 1).
    assert_eq!(inner(&seq[0]), 0..64, "inner of 64 for tile 0");
    assert_eq!(inner(&seq[1]), 0..36, "inner of 36 for tile 1");
}

/// TASK-0173 AC#3 shape: the exact strip-mine emitted for
/// 04-prefix-sum/blocked-nondiv's `loop j : block=6` over Pass 3's
/// within-block-scan ACCUMULATION axis `for j : 0 .. BS` (BS=64).
///
/// 64 is NOT divisible by 6 (64 = 6*10 + 4): `num_full = 10`,
/// `rem = 4`. This pins the per-occurrence `BlockTag`s the backend
/// rebinds from for a NON-IDEMPOTENT accumulator axis — the full
/// nest gets `is_partial=false` (backend emits abs
/// `LO + j__tile*6 + j`) and the trailing partial tile gets
/// `is_partial=true` (backend emits the CONSTANT base abs
/// `LO + num_full*6 + j` = `LO + 10*6 + j`). A wrong tag here
/// would make the non-divisible accumulator e2e cell diverge from
/// `reference.bin`; this is the structural companion to that e2e
/// differential proof.
#[test]
fn rewrite_node_prefix_sum_nondiv_j_block6() {
    let mut tile_map: BTreeMap<IterVar, (String, IterVar, u64)> = BTreeMap::new();
    tile_map.insert(IterVar(0), ("j__tile".to_string(), IterVar(7), 6));
    // 04-prefix-sum Pass-3 `for j : 0 .. 64`, block=6.
    let n = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..64,
        body: Box::new(ACFGNode::Sequence(vec![op()])),
        block_tag: None,
        break_cond: None,
    };
    let out = rewrite_node(n, &tile_map);

    let seq = match out {
        ACFGNode::Sequence(s) => s,
        other => panic!("non-divisible must be a Sequence, got {other:?}"),
    };
    assert_eq!(
        seq.len(),
        2,
        "block=6 over 0..64 -> full-tile nest + trailing partial tile"
    );

    // Reuse the same nest-shape contract as the 05-stencil shape
    // test: outer is the tile var (untagged), inner keeps the
    // source var and carries exactly the expected per-occurrence
    // BlockTag.
    let check_nest = |node: &ACFGNode,
                      outer_range: std::ops::Range<i64>,
                      inner_len: i64,
                      expect_tag: BlockTag| {
        match node {
            ACFGNode::Repeat {
                iter_var: outer,
                range,
                body,
                block_tag: outer_tag,
                break_cond: _,
            } => {
                assert_eq!(*outer, IterVar(7), "outer is the tile var");
                assert_eq!(*range, outer_range);
                assert_eq!(*outer_tag, None, "tile loop must NOT be tagged");
                match &**body {
                    ACFGNode::Sequence(inner_seq) => {
                        assert_eq!(inner_seq.len(), 1);
                        match &inner_seq[0] {
                            ACFGNode::Repeat {
                                iter_var: inner,
                                range: ir,
                                block_tag: inner_tag,
                                ..
                            } => {
                                assert_eq!(*inner, IterVar(0), "inner keeps source var");
                                assert_eq!(*ir, 0..inner_len);
                                assert_eq!(
                                    *inner_tag,
                                    Some(expect_tag),
                                    "inner loop's per-occurrence BlockTag"
                                );
                            }
                            o => panic!("inner not Repeat: {o:?}"),
                        }
                    }
                    o => panic!("nest body not Sequence: {o:?}"),
                }
            }
            o => panic!("nest not Repeat: {o:?}"),
        }
    };

    // 64 / 6 = 10 full tiles of width 6 -> full-nest tag
    // (backend: abs j = LO + j__tile*6 + j).
    check_nest(
        &seq[0],
        0..10,
        6,
        BlockTag {
            block_n: 6,
            num_full: 10,
            is_partial: false,
        },
    );
    // 64 % 6 = 4: a single (0..1) trailing tile of width 4 ->
    // partial tag. Backend rebinds the CONSTANT base
    // abs j = LO + num_full*6 + j = LO + 10*6 + j (NOT
    // tile*6 which would be 0 — the wrong base for an
    // accumulator).
    check_nest(
        &seq[1],
        0..1,
        4,
        BlockTag {
            block_n: 6,
            num_full: 10,
            is_partial: true,
        },
    );
}
