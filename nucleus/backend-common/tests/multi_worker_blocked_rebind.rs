//! Per-occurrence strip-mine rebinding on the SHARED multi-worker
//! walker (TASK-0181).
//!
//! These tests exercise [`backend_common::multi_worker_walker::
//! render_worker_events`] with synthetic `Event::Loop`s carrying a
//! `block_tag`. No tier-1 schedule blocks a multi-worker loop today,
//! so the rebinding path is structurally unreachable from `just e2e`
//! — these tests are the targeted lower-bound proof that the path
//! emits the contracted shape.
//!
//! Test surface:
//!
//! 1. `rebinds_full_nest_in_loop_header_and_fire_body` — the load-
//!    bearing case. A full (`is_partial == false`) nest wrapping an
//!    inner strip-mined loop whose body Fire uses the reused var; the
//!    Fire arg site MUST see the rebound `(LO + tile*N + inner)`
//!    expression, not the bare `inner`. This is the
//!    abs_subst-in-Fire-args verification (TASK-0181 plan §3 / the
//!    explicit review-gate finding).
//! 2. `rebinds_partial_nest_constant_base` — `is_partial == true`
//!    rebinds with `LO + num_full*N + inner`; tile var of the
//!    partial's own `0..1` loop is unused.
//! 3. `full_nest_without_enclosing_tile_returns_contract_gap` — a
//!    full nest with no enclosing tile loop is a malformed EventList;
//!    must fail loud (typed error, never panic).
//! 4. `non_blocked_loop_unchanged_partition_slice_path` — sanity:
//!    `block_tag == None` still drives the partition-slice / source-
//!    form precedence path unchanged. Guards against accidentally
//!    breaking the (currently exercised) non-blocked codegen.

use std::collections::BTreeMap;

use compiler::algo::{IrExpr, ResolvedType, ScalarType};
use compiler::event::{
    ArgBinding, BlockTag, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId,
    WorkerId,
};
use compiler::sidecar::{KernelSig, LoopBound, NameSidecar};
use compiler::NameTables;

use backend_common::multi_worker_walker::{render_worker_events, WalkerCtx};

/// Build a minimal `(NameTables, NameSidecar)` pair populated for a
/// single source loop variable `var` with `LO..HI` bounds, one
/// kernel `k` taking one `i64` scalar parameter, no output, and one
/// data symbol `d : i64` (unused by the kernel but lets the Fire
/// have a meaningful arg shape if needed).
///
/// Eight params is one over clippy's threshold; the alternative
/// (a synthetic options struct) would be unread setup ceremony that
/// hides what each test passes. Local allow.
#[allow(clippy::too_many_arguments)]
fn make_minimal_tables(
    src_iv: IterVar,
    src_var_name: &str,
    tile_iv: IterVar,
    tile_var_name: &str,
    lo: i64,
    hi: i64,
    kernel: KernelId,
    kernel_name: &str,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.iter_var.insert(src_iv, src_var_name.to_string());
    names.iter_var.insert(tile_iv, tile_var_name.to_string());
    names.kernel.insert(kernel, kernel_name.to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        src_iv,
        LoopBound {
            lo: IrExpr::IntLit(lo),
            hi: IrExpr::IntLit(hi),
        },
    );
    // Kernel signature: one scalar i64 param, unit return. The
    // walker's render_fire_args_pub joins on this; absence would
    // trip a different error than the one we're testing.
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I64,
                dims: vec![],
            }],
            ret: None,
        },
    );
    (names, sidecar)
}

type RendezvousIds = BTreeMap<(DataId, compiler::event::SeqTag), usize>;
type PairTiles = BTreeMap<(DataId, compiler::event::SeqTag), IterTile>;

/// Empty maps that the walker requires but the synthetic test has no
/// Push/Wait pairs to populate. Returning by value keeps the test
/// site short.
fn empty_walker_maps() -> (RendezvousIds, PairTiles) {
    (BTreeMap::new(), BTreeMap::new())
}

#[test]
fn rebinds_full_nest_in_loop_header_and_fire_body() {
    // SHAPE: `tile : 0..3 { inner : 0..4 { k(inner) } }` with
    // `inner` carrying a full-nest BlockTag {N=4, num_full=3,
    // is_partial=false}. Source `for src : 5 .. 17` (LO=5). The
    // strip-mine rebinding must produce `(5_i64 + (tile * 4_i64) +
    // inner)` at the Fire arg site — proving abs_subst reaches into
    // Fire body sites, not just the loop header.
    let src_iv = IterVar(1); // the strip-mined inner var (reuses src name)
    let tile_iv = IterVar(2); // enclosing tile loop
    let kernel = KernelId(7);
    let (names, sidecar) = make_minimal_tables(
        src_iv, "inner", tile_iv, "tile", 5, 17, kernel, "k",
    );
    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    // Inner loop body: `k(inner)` — the rebind target. Use the
    // src_iv's name "inner" as a bare `IrExpr::Ident` scalar arg;
    // render_int_expr consults abs_subst by NAME (the string), so
    // the rebound expression must replace the bare ident.
    let fire = Event::Fire {
        kernel,
        tile: IterTile::new(vec![]),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Scalar(IrExpr::Ident("inner".to_string()))],
            output: None,
        },
    };

    // Inner strip-mined loop: 0..4, tagged full-nest, body = [fire].
    let inner_loop = Event::loop_over_tagged(
        src_iv,
        0..4,
        vec![fire],
        BlockTag {
            block_n: 4,
            num_full: 3,
            is_partial: false,
        },
    );

    // Enclosing tile loop: 0..3, untagged source-looking loop. Its
    // name comes from NameTables only (no loop_bounds entry for the
    // tile var — falls into the synthesised-tile "concrete folded
    // range" branch, which renders `{n}_i64..{n}_i64`).
    let tile_loop = Event::loop_over(tile_iv, 0..3, vec![inner_loop]);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "slot",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[tile_loop], &mut out, 0, "")
        .expect("rebinding emit must succeed");

    // Header of the inner loop uses the CONCRETE folded range
    // (0_i64..4_i64), NOT the source-form (which would be 5..17)
    // and NOT a partition slice.
    assert!(
        out.contains("for inner in (0_i64)..(4_i64)"),
        "inner loop header should use concrete folded range; got:\n{out}"
    );
    // Tile loop header is untagged, no loop_bounds entry, so falls
    // into the synthesised-tile concrete-folded branch.
    assert!(
        out.contains("for tile in (0_i64)..(3_i64)"),
        "tile loop header should use concrete folded range; got:\n{out}"
    );
    // THE LOAD-BEARING ASSERTION: the Fire arg in the inner body
    // must show the rebound `(LO + tile*N + inner)` expression, NOT
    // the bare `inner`. This is the abs_subst-in-Fire-args proof.
    // `render_fire_arg` wraps a scalar Ident with `({rendered}) as
    // i64` (kernel param is scalar i64) → outer parens around the
    // already-parenthesised rebinding substitution.
    assert!(
        out.contains("k(((5_i64 + (tile * 4_i64) + inner)) as i64)"),
        "Fire arg must be the rebound absolute expression \
         `((5_i64 + (tile * 4_i64) + inner)) as i64`; got:\n{out}"
    );
    // ABSENCE-CHECK: the un-rebound `k((inner) as i64)` shape must
    // NOT appear (the exact accumulator-double-count footprint).
    assert!(
        !out.contains("k((inner) as i64)"),
        "un-rebound `k((inner) as i64)` must NOT appear; got:\n{out}"
    );
}

#[test]
fn rebinds_partial_nest_constant_base() {
    // SHAPE: trailing partial tile, `is_partial == true`. Its own
    // tile loop is `0..1` so the `tile*N` term would be 0 — the
    // constant `num_full*N` offset is used instead. Source LO = 5,
    // N = 4, num_full = 3 → rebound `(5_i64 + (3_i64 * 4_i64) +
    // inner)`. The partial's range is the remainder (e.g. 0..2,
    // for HI=19).
    let src_iv = IterVar(1);
    let tile_iv = IterVar(2);
    let kernel = KernelId(7);
    let (names, sidecar) = make_minimal_tables(
        src_iv, "inner", tile_iv, "p_tile", 5, 19, kernel, "k",
    );
    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    let fire = Event::Fire {
        kernel,
        tile: IterTile::new(vec![]),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Scalar(IrExpr::Ident("inner".to_string()))],
            output: None,
        },
    };
    let inner_partial = Event::loop_over_tagged(
        src_iv,
        0..2,
        vec![fire],
        BlockTag {
            block_n: 4,
            num_full: 3,
            is_partial: true,
        },
    );
    // The partial's own tile loop is `0..1` per block_transform; the
    // walker reads the partial tag, doesn't consult the tile name in
    // the partial branch, but the enclosing-loop wrapping must still
    // exist (mirrors how block_transform emits it). Include it for
    // structural fidelity.
    let partial_tile_loop = Event::loop_over(tile_iv, 0..1, vec![inner_partial]);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "slot",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
    };

    let mut out = String::new();
    render_worker_events(
        &ctx,
        WorkerId(0),
        &[partial_tile_loop],
        &mut out,
        0,
        "",
    )
    .expect("partial-nest rebinding emit must succeed");

    // Inner partial loop header: concrete folded range `0..2`.
    assert!(
        out.contains("for inner in (0_i64)..(2_i64)"),
        "partial inner header should use concrete folded range 0..2; got:\n{out}"
    );
    // The rebound Fire arg uses the constant base, NOT a tile var.
    // Same outer-paren wrapping as the full-nest case.
    assert!(
        out.contains("k(((5_i64 + (3_i64 * 4_i64) + inner)) as i64)"),
        "partial Fire arg must be `((5_i64 + (3_i64 * 4_i64) + inner)) as i64`; got:\n{out}"
    );
    // The partial path must NOT reference the enclosing tile var
    // (its `0..1` makes `p_tile*N` always zero, the wrong base).
    assert!(
        !out.contains("p_tile * 4_i64"),
        "partial-nest rebinding must use the constant base, not `p_tile*N`; got:\n{out}"
    );
}

#[test]
fn full_nest_without_enclosing_tile_returns_contract_gap() {
    // A full-nest BlockTag (`is_partial == false`) with no
    // enclosing tile loop is a malformed EventList: block_transform
    // ALWAYS wraps a full inner in its tile. The walker must fail
    // loud with a typed ContractGap (mirrors the single-worker
    // pthreads-sync path, never panic). Bare-top-level inner loop
    // -> no enclosing.
    let src_iv = IterVar(1);
    let tile_iv = IterVar(2); // declared in NameTables but not used as a wrapping loop
    let kernel = KernelId(7);
    let (names, sidecar) = make_minimal_tables(
        src_iv, "inner", tile_iv, "tile", 5, 17, kernel, "k",
    );
    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    let fire = Event::Fire {
        kernel,
        tile: IterTile::new(vec![]),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Scalar(IrExpr::Ident("inner".to_string()))],
            output: None,
        },
    };
    let bare_inner = Event::loop_over_tagged(
        src_iv,
        0..4,
        vec![fire],
        BlockTag {
            block_n: 4,
            num_full: 3,
            is_partial: false,
        },
    );

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "slot",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
    };

    let mut out = String::new();
    let err = render_worker_events(&ctx, WorkerId(0), &[bare_inner], &mut out, 0, "")
        .expect_err("full nest with no enclosing tile must fail loud");
    let msg = format!("{err}");
    assert!(
        msg.contains("no enclosing tile loop") && msg.contains("block_tag"),
        "expected ContractGap mentioning the missing enclosing tile; got: {msg}"
    );
}

#[test]
fn non_blocked_loop_unchanged_partition_slice_path() {
    // Regression guard: a non-blocked (`block_tag == None`) loop
    // must still drive the existing partition-slice / source-form
    // precedence path. Set up a partition_worker_ranges entry for
    // worker 0 on the loop var and assert the header uses the
    // concrete per-worker slice, NOT the source bound.
    let src_iv = IterVar(1);
    let unused_tile = IterVar(2);
    let kernel = KernelId(7);
    let (names, mut sidecar) = make_minimal_tables(
        src_iv, "i", unused_tile, "_unused", 0, 8, kernel, "k",
    );
    // Per-worker slice override: worker 0 takes 0..4.
    let mut per_worker: BTreeMap<WorkerId, std::ops::Range<i64>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), 0..4);
    sidecar
        .partition_worker_ranges
        .insert(src_iv, per_worker);
    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    let fire = Event::Fire {
        kernel,
        tile: IterTile::new(vec![]),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Scalar(IrExpr::Ident("i".to_string()))],
            output: None,
        },
    };
    let loop_ev = Event::loop_over(src_iv, 0..8, vec![fire]);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "slot",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[loop_ev], &mut out, 0, "")
        .expect("non-blocked loop emit must succeed");

    // Partition slice takes precedence over source bound for this
    // worker.
    assert!(
        out.contains("for i in (0_i64)..(4_i64)"),
        "expected per-worker partition slice 0..4; got:\n{out}"
    );
    // The Fire arg is the bare `i` (cast to i64 by the scalar-arg
    // cast in render_fire_arg) — abs_subst is empty in this path.
    assert!(
        out.contains("k((i) as i64)"),
        "non-blocked Fire arg should be bare `(i) as i64`; got:\n{out}"
    );
    // ABSENCE: no rebinding artifact should leak into a non-blocked
    // path.
    assert!(
        !out.contains("tile *") && !out.contains("__tile"),
        "non-blocked loop should produce no rebinding tokens; got:\n{out}"
    );
}

/// Suppress the unused-import warning on `DataSlice` — referenced via
/// type only by the helper signature.
#[allow(dead_code)]
fn _force_use(_d: DataSlice) {}
