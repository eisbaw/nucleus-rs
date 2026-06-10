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

// --------------------------------------------------------------------
// TASK-0456: synthetic `<var>__tile` name collides with a user-declared
// iter var — typed error, not a panic, on this valid (if obscure)
// program. Driven END-TO-END through the real pass entry point
// (parse -> lower -> link -> run_pre_mediation_passes, which calls
// `apply_block_transforms`) via the shared `test_support` helper, so
// the diagnostic that is pinned is the one a real `nucleus build` would
// emit.
// --------------------------------------------------------------------

/// A complete, valid algorithm whose iteration variables are LITERALLY
/// `y` and `y__tile` (the latter is a legal identifier — `_` is allowed
/// anywhere after the first char). Tiling `y` with `block=` synthesises
/// an outer loop named `y__tile`, which the user already declared.
const COLLISION_ALGO: &str = r#"
const N : usize = 256;

data a : i32[N];
data c : i32[N];
data d : i32[N];

kernel inc        : (i32)    -> i32    pure;
kernel load_input : ()       -> i32[N] effectful;
kernel save_output: (i32[N]) -> ()     effectful;

a <-- load_input();

for y : 0 .. N {
    c[y] <-- inc(a[y]);
}

for y__tile : 0 .. N {
    d[y__tile] <-- inc(c[y__tile]);
}

save_output(d);
"#;

/// Schedule that puts `block=64` on loop `y`. Single worker so there
/// are no cross-worker transfer concerns to satisfy — the only thing
/// under test is the `block=` strip-mine on `y`, whose synthetic
/// `y__tile` name collides with the user's `y__tile` loop.
const COLLISION_SCHED: &str = r#"
schedule for "prog.algo.nuc" {
    workers = { host };

    place load_input  on host;
    place save_output on host;
    place inc         on host;

    loop y : block=64;
}
"#;

/// The same algorithm with the `block=` directive REMOVED. Proves the
/// program is GENUINELY VALID — only the `block=`-induced synthetic
/// name collision is the failure, not some unrelated parse/link defect.
const COLLISION_SCHED_NO_BLOCK: &str = r#"
schedule for "prog.algo.nuc" {
    workers = { host };

    place load_input  on host;
    place save_output on host;
    place inc         on host;
}
"#;

#[test]
fn block_synthetic_tile_var_collision_is_typed_error_not_panic() {
    // First: prove the program is VALID (the architecture-review claim
    // is "a valid, if obscure, program"). The IDENTICAL algorithm,
    // with the `block=` directive removed, compiles end-to-end clean —
    // so the only thing the collision test exercises is the strip-mine
    // name clash, not an unrelated parse/link error.
    crate::test_support::build_pre_mediation_acfg(COLLISION_ALGO, COLLISION_SCHED_NO_BLOCK)
        .expect("the y/y__tile program is valid without the block= directive");

    // End-to-end: the helper parses/lowers/links the source and runs
    // the pre-mediation chain, which includes `apply_block_transforms`.
    // It maps any `PreMediationError` via `{e:?}` (derived Debug), so
    // the returned Err string is the Debug form of
    // `PreMediationError::BlockTransform(SyntheticTileVarCollision{..})`.
    let err = crate::test_support::build_pre_mediation_acfg(COLLISION_ALGO, COLLISION_SCHED)
        .expect_err("y__tile collision must be a typed error, never a panic");

    // Pin BOTH variable names appear in the diagnostic so the user can
    // act on it: the `block=` target loop `y` and the colliding
    // synthetic/user name `y__tile`.
    assert!(
        err.contains("SyntheticTileVarCollision"),
        "expected the typed collision variant, got: {err}"
    );
    assert!(
        err.contains("tiled_var: \"y\""),
        "diagnostic must name the block= target loop `y`, got: {err}"
    );
    assert!(
        err.contains("tile_var: \"y__tile\""),
        "diagnostic must name the colliding synthetic var `y__tile`, got: {err}"
    );
}

/// Pin the user-facing `Display` string (what the driver prints as
/// `block-transform error: {e}`) for the collision — independent of the
/// `{e:?}` Debug form the test helper happens to surface. Constructs
/// the variant directly (the pass entry point is exercised end-to-end
/// by the test above; here we only assert the wording contract).
#[test]
fn synthetic_tile_var_collision_display_names_both_vars() {
    let e = BlockTransformError::SyntheticTileVarCollision {
        tiled_var: "y".to_string(),
        tile_var: "y__tile".to_string(),
    };
    let msg = e.to_string();
    assert!(msg.contains('`'), "diagnostic should quote the names");
    assert!(
        msg.contains("loop y : block="),
        "must name the block= target loop: {msg}"
    );
    assert!(
        msg.contains("y__tile"),
        "must name the colliding synthetic var: {msg}"
    );
    assert!(
        msg.contains("Rename"),
        "diagnostic must tell the user how to fix it: {msg}"
    );
}
