//! Receiver-side gather emit shape for `Event::Wait` (TASK-0117 1D
//! leading-axis slice-paste + TASK-0294 2D row-loop slice-paste).
//!
//! These tests drive [`backend_common::multi_worker_walker::
//! render_worker_events`] with a single synthetic `Event::Wait`,
//! pinning the emit string for each of the three [`WaitSlice`] arms:
//!
//! 1. `whole_array_assign_when_tile_empty` — no tile bounds at all
//!    (a top-level data symbol with no enclosing iteration nest).
//!    Expected emit: bare `name = rhs;`. Pre-TASK-0117 single-pair
//!    behaviour; the regression footprint if the new dispatch
//!    accidentally falls through to a slice-paste branch.
//! 2. `flat_1d_slice_paste_for_partition_workers` — 1D
//!    leading-axis sub-range tile (`[(y, 1..8)]`) on a 2D image
//!    (dims `[16, 16]`). Expected emit: `name[16..128]
//!    .copy_from_slice(&_tmp[16..128])`. The TASK-0117 host-gather
//!    shape for `partition=workers` / `partition=rows`. Stays
//!    bit-identical to the pre-TASK-0294 emit so every shipped
//!    1D-partition e2e cell holds.
//! 3. `rows_2d_slice_paste_for_partition_blocks2d` — 2D rect tile
//!    (`[(y, 1..8), (x, 1..8)]`) on the same 2D image. Expected
//!    emit: the row-loop `for _y in 1..8 { let _r = _y * 16; ...
//!    copy_from_slice ... }`. The TASK-0294 fix — pre-fix this
//!    would have collapsed onto the 1D path and pasted the full
//!    y-band over adjacent workers' columns (the diagnostic
//!    evidence from TASK-0290 cycle 114b).
//! 4. `degenerate_2d_full_range_collapses_to_whole_array` — 2D tile
//!    where BOTH axes cover the full source range. Should collapse
//!    to the whole-array assign (defensive emit-identity guard so
//!    the new path stays additive-only; DEFENSIVE-ONLY — no
//!    shipped pass currently constructs such a (2-bound, full-
//!    range-on-both-axes) tile, since a partition that selects no
//!    worker's slice would not be a partition).
//! 5. `inner_axis_out_of_bounds_returns_contract_gap` — typed-error
//!    surface for the new validation (TASK-0294 inner-axis range
//!    check). Mirrors the leading-axis arm's pre-existing
//!    `ContractGap` on out-of-bounds.
//! 6. `leading_axis_out_of_bounds_returns_contract_gap` — typed-
//!    error surface for the leading-axis range check (TASK-0117
//!    arm, renamed under cycle-115 from `leading_axis_slice` to
//!    `wait_slice` — the renaming carried no behaviour change but
//!    the arm previously had NO test pinning the error path; this
//!    closes that test gap (TASK-0294 cycle-115 architect P3.2)).
//! 7. `rank_3_or_higher_tile_returns_contract_gap` — TASK-0294
//!    cycle-115 architect P2.1 fail-loud surface: a rank-3+ tile or
//!    rank-3+ data shape would slip silently into the 2D arm,
//!    consulting only the first two axes (the SAME HONEST-PARTIAL
//!    class the cycle-115 fix removed for 2-axis data). No shipped
//!    schedule constructs such a shape today (13-cnn-inference has
//!    rank-4 data but rank-1 tiles via partition=workers, which
//!    hits the 1D arm). This test pins the typed-error refusal so
//!    a future schedule that does construct one is flagged at
//!    compile time, not after an out-of-bounds gather.
//! 8. `task0316_inner_axis_leading_layout_emits_against_dim0` —
//!    backend-side CONSUMER pin for the `bounds[i] ↔ ty.dims[i]`
//!    positional contract that the TASK-0306 cycle-133 helper
//!    `order_halo_strip_bounds_by_data_dim` produces in
//!    `transfer_inject`. Feeds `render_wait_assign` directly with an
//!    inner-leading tile (`bounds[0] = inner_iv`,
//!    `bounds[1] = outer_iv` — what cycle-133's helper emits for
//!    data indexed `[inner_iv][outer_iv]`) on deliberately ASYMMETRIC
//!    data dims `[16, 8]` so the emitted slice arithmetic differs
//!    from the canonical `(outer_iv, inner_iv)` order. By transitive
//!    coverage (both cycle-133 `order_halo_strip_bounds_by_data_dim`
//!    and cycle-135 `rewrite_partition_tiles_inner` feed the same
//!    `IterTile.bounds` consumed by the same `wait_slice` dispatch),
//!    this pin also covers the TASK-0317 helper's output shape — no
//!    separate backend pin is required for the broadcast Push/Wait
//!    path. The producer-side end-to-end pin lives in
//!    `nucleus-compiler/tests/halo_strip_synth.rs` as
//!    `task0306_ac3/ac4/ac5` and exercises `inject_transfers`
//!    directly. **What this test catches**: a future `wait_slice`
//!    refactor that drops positional dim-mapping (e.g. switches to
//!    iv-name lookup) would silently re-permute the helper's
//!    dim-ordered output and slice-paste against the wrong dim — the
//!    test bites with asymmetric dims by asserting `outer_lo..outer_hi`
//!    = `bounds[0].1` and `row_stride` = `dims[1]`. **What this test
//!    does NOT catch**: producer-side cycle-133 helper rot — that
//!    lives in `task0306_ac3` (the helper would emit canonical-order
//!    bounds; this test fabricates dim-ordered bounds regardless of
//!    producer state). Closes TASK-0316 AC#1.
//! 9. `task0316_non_prefix_layout_empty_bounds_consumer_pin` —
//!    backend-side CONSUMER pin for the empty-bounds shape that
//!    cycle-133's `order_halo_strip_bounds_by_data_dim` produces for
//!    non-prefix data layouts (data indexed `[k][x]` where `k` is
//!    unpartitioned — the helper returns `Vec::new()` for safety).
//!    Distinct dispatch path from
//!    `whole_array_assign_when_tile_empty` above: that test passes
//!    an EMPTY `pair_tiles` map (`pair_tiles.get(...).is_none()`),
//!    skipping `wait_slice` entirely. This test passes a POPULATED
//!    `pair_tiles` map carrying an `IterTile::empty()` so dispatch
//!    enters `wait_slice` and hits the `tile.bounds.first()`
//!    else-branch early return. Both converge on the whole-array
//!    `name = rhs;` emit, but only one of these two tests
//!    pre-existed. Closes TASK-0316 AC#2.
//!
//! ## Scope drift: prefix-substitution pins (TASK-0321 / TASK-0322)
//!
//! Tests 10-11 below (`task0321_rendezvous_prefix_substituted_in_2d_
//! row_loop_arm` + `task0322_rendezvous_prefix_substituted_on_push_
//! emit`) pin the `WalkerCtx::rendezvous_prefix` substitution across
//! the three `render_worker_events`-using prefixes (`"ring"` /
//! `"slot"` / `"chan"`). They live in this file (rather than a new
//! `push_emit_prefix.rs`) for sibling-adjacency with the shared
//! `make_minimal_tables` + `one_pair` helpers and to keep the two
//! substitution sites (Wait inside `render_worker_events_inner` —
//! the `{prefix}{rendezvous_prefix}_{rid}.wait()` substitution arm;
//! Push inside the same function — the
//! `{prefix}{rendezvous_prefix}_{rid}.push(...)` substitution arm)
//! paired in one place. Grep witness:
//! `grep -nE '\{rendezvous_prefix\}_\{rid\}\.(push|wait)'
//! nucleus/backend-common/src/multi_worker_walker.rs` returns
//! exactly two production sites (Push + Wait substitution) — the
//! `{rid}` (not `{id}`) match-string excludes the docstring
//! examples at the top of the file.
//! TASK-0319 cycle-146 audit migrated former line citations
//! (`multi_worker_walker.rs:809` Wait, `:789` Push) to function-name
//! anchors per the cycle-138 forward-carry discipline. This stretches
//! the "Receiver-side gather emit shape for `Event::Wait`" name in
//! the module heading: the file now also covers Push-side prefix
//! emit. Acknowledged tradeoff, not a defect. Acceptance: TASK-0321
//! cycle-140 (Wait) + TASK-0322 cycle-141 (Push) close the
//! prefix-substitution sweep end-to-end on the
//! `render_worker_events` machinery.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use backend_common::multi_worker_walker::{render_worker_events, RendezvousId, WalkerCtx};
use backend_common::render::EmitError;

type RendezvousIds = BTreeMap<(DataId, SeqTag), RendezvousId>;
type PairTiles = BTreeMap<(DataId, SeqTag), IterTile>;

/// Build a minimal `(NameTables, NameSidecar)` with one data symbol
/// `data_iv` named `data_name` typed as `dims` of `i32`, and two
/// workers: `WorkerId(0) -> "w0"` (the sender) + `WorkerId(1) ->
/// "host"` (the receiver in render_worker_events).
fn make_minimal_tables(
    data_id: DataId,
    data_name: &str,
    dims: Vec<usize>,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.data.insert(data_id, data_name.to_string());
    names.worker.insert(WorkerId(0), "w0".to_string());
    names.worker.insert(WorkerId(1), "host".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.data_types.insert(
        data_id,
        ResolvedType {
            scalar: ScalarType::I32,
            dims,
        },
    );
    (names, sidecar)
}

/// Build the rendezvous + pair-tile maps the walker reads. One
/// entry: `(data, seq) -> rendezvous_id` and `(data, seq) -> tile`.
fn one_pair(
    data: DataId,
    seq: SeqTag,
    rid: RendezvousId,
    tile: IterTile,
) -> (RendezvousIds, PairTiles) {
    let mut ids = BTreeMap::new();
    ids.insert((data, seq), rid);
    let mut tiles = BTreeMap::new();
    tiles.insert((data, seq), tile);
    (ids, tiles)
}

/// Render a single `Event::Wait` from `WorkerId(0)` on worker
/// `WorkerId(1)` (the host). Returns the emitted string.
fn render_one_wait(
    names: &NameTables,
    sidecar: &NameSidecar,
    rendezvous_ids: &RendezvousIds,
    pair_tiles: &PairTiles,
    data: DataId,
    seq: SeqTag,
    tile: IterTile,
) -> Result<String, EmitError> {
    let ctx = WalkerCtx {
        names,
        sidecar,
        rendezvous_prefix: "ring",
        rendezvous_ids,
        pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };
    let wait = Event::Wait {
        src: WorkerId(0),
        data,
        tile,
        seq,
    };
    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(1), &[wait], &mut out, 0, "")?;
    Ok(out)
}

#[test]
fn whole_array_assign_when_tile_empty() {
    // No bounds + no pair-tile entry → whole-array assign (the
    // TASK-0117-pre single-pair shape, e.g. a top-level load_input
    // host transfer).
    let data = DataId(7);
    let seq = SeqTag(3);
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    // Empty pair_tiles: no tile registered → render_wait_assign's
    // `ctx.pair_tiles.get(...)` returns None → None slice → whole.
    let mut ids: RendezvousIds = BTreeMap::new();
    ids.insert((data, seq), 0usize);
    let tiles: PairTiles = BTreeMap::new();

    let out = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, IterTile::empty())
        .expect("empty-tile Wait must render");

    assert!(
        out.contains("img_in = ring_0.wait();"),
        "empty tile must emit whole-array assign `name = rhs;`; got:\n{out}"
    );
    assert!(
        !out.contains("copy_from_slice"),
        "empty tile must NOT fall through to slice-paste; got:\n{out}"
    );
}

#[test]
fn flat_1d_slice_paste_for_partition_workers() {
    // 1D leading-axis sub-range tile on a 2D image — the TASK-0117
    // shape for `partition=workers` / `partition=rows`. Pre-TASK-
    // 0294 the only slice-paste shape. Must stay bit-identical so
    // every shipped 1D-partition e2e cell holds.
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    let tile = IterTile::new(vec![(y_iv, 1..8)]);
    let (ids, tiles) = one_pair(data, seq, 5, tile.clone());

    let out = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect("1D slice-paste Wait must render");

    // Leading-axis: lo=1*16=16, hi=8*16=128. Match the EXACT pre-
    // TASK-0294 emit (regression pin).
    assert!(
        out.contains(
            "{ let _tmp = ring_5.wait(); \
             img_in[16usize..128usize].copy_from_slice(\
             &_tmp[16usize..128usize]); }"
        ),
        "1D leading-axis slice-paste must emit `name[lo..hi].copy_from_slice(\
         &_tmp[lo..hi])` with PRE-MULTIPLIED offsets (lo=1*16=16, hi=8*16=128); \
         got:\n{out}"
    );
    // The row-loop shape MUST NOT appear on a 1-bound tile.
    assert!(
        !out.contains("for _y in"),
        "1D-bound tile must NOT trigger the 2D row-loop path; got:\n{out}"
    );
}

#[test]
fn rows_2d_slice_paste_for_partition_blocks2d() {
    // 2D rect tile on a 2D image — the TASK-0294 host-gather +
    // halo-strip shape for `partition=blocks2d`. The 1D leading-
    // axis path would emit `img_out[16..128].copy_from_slice(&_tmp
    // [16..128])` (rows 1..8 ALL cols), which under multi-worker
    // gather overwrites adjacent workers' columns with default-zero
    // values (the diagnostic from TASK-0290 cycle 114b: first
    // divergence at byte 68 = row 1 col 1 = first compute pixel).
    // The fix: per-row `copy_from_slice` over only the inner-axis
    // sub-range cols 1..8.
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let x_iv = IterVar(2);
    let (names, sidecar) = make_minimal_tables(data, "img_out", vec![16, 16]);
    let tile = IterTile::new(vec![(y_iv, 1..8), (x_iv, 1..8)]);
    let (ids, tiles) = one_pair(data, seq, 12, tile.clone());

    let out = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect("2D row-loop Wait must render");

    // row_stride = product(dims[1..]) = 16. inner_stride =
    // product(dims[2..]) = 1 (empty product). inner_lo_off =
    // 1*1=1; inner_hi_off = 8*1=8. outer_lo=1, outer_hi=8.
    assert!(
        out.contains(
            "{ let _tmp = ring_12.wait(); \
             for _y in 1usize..8usize { \
             let _r = _y * 16usize; \
             img_out[_r + 1usize.._r + 8usize].copy_from_slice(\
             &_tmp[_r + 1usize.._r + 8usize]); } }"
        ),
        "2D row-loop slice-paste must emit `for _y in outer_lo..outer_hi {{ \
         let _r = _y * row_stride; name[_r + inner_lo.._r + inner_hi]\
         .copy_from_slice(&_tmp[_r + inner_lo.._r + inner_hi]); }}` with \
         outer=(1,8), row_stride=16, inner=(1,8); got:\n{out}"
    );
    // ABSENCE: the pre-fix 1D collapse would emit `img_out[16usize..128usize]`
    // without any `for` loop — the TASK-0290 cycle 114b defect footprint.
    assert!(
        !out.contains("img_out[16usize..128usize].copy_from_slice"),
        "regression: 2D tile collapsed to 1D leading-axis paste; this is the \
         TASK-0290 cycle 114b defect footprint (host gather pastes full y-band, \
         overwriting adjacent workers' columns). Got:\n{out}"
    );
}

#[test]
fn degenerate_2d_full_range_collapses_to_whole_array() {
    // Defensive: 2D tile where both axes cover the data's full
    // source range. Should fall through to whole-array assign for
    // emit identity with the pre-TASK-0294 single-pair behaviour.
    // (Construction-wise this shape is rare — a producer that
    // sends the whole array under a 2D partition would not be
    // selecting any one worker's slice — but the dispatch must
    // collapse cleanly rather than emit a noop row-loop.)
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let x_iv = IterVar(2);
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    let tile = IterTile::new(vec![(y_iv, 0..16), (x_iv, 0..16)]);
    let (ids, tiles) = one_pair(data, seq, 0, tile.clone());

    let out = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect("full-range 2D Wait must render");

    assert!(
        out.contains("img_in = ring_0.wait();"),
        "2D tile covering full range on both axes must collapse to whole-array \
         assign; got:\n{out}"
    );
    assert!(
        !out.contains("for _y in") && !out.contains("copy_from_slice"),
        "full-range 2D must NOT emit either the row-loop or the 1D slice-paste; \
         got:\n{out}"
    );
}

#[test]
fn leading_axis_out_of_bounds_returns_contract_gap() {
    // TASK-0117 arm (renamed under cycle-115): a leading-axis range
    // exceeding `dims[0]` is a compiler-pass invariant violation.
    // The check has been there since TASK-0117 but had no dedicated
    // test pinning the error path; the rename to `wait_slice` is
    // load-bearing-equivalent and worth a 5-line pin.
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    // Leading range 0..32 exceeds leading_dim=16.
    let tile = IterTile::new(vec![(y_iv, 0..32)]);
    let (ids, tiles) = one_pair(data, seq, 0, tile.clone());

    let err = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect_err("leading-axis out-of-bounds must fail loud");
    let msg = format!("{err}");
    assert!(
        msg.contains("leading-axis range") && msg.contains("leading-dim 16"),
        "expected ContractGap mentioning the offending leading-axis range + \
         leading-dim; got: {msg}"
    );
}

#[test]
fn rank_3_or_higher_tile_returns_contract_gap() {
    // TASK-0294 cycle-115 architect P2.1: a rank-3+ tile or rank-
    // 3+ data shape would slip silently into the 2D arm, consulting
    // only the first two axes. The cycle-115 fix-loud guard rejects
    // it at compile time. Two sub-cases:
    //   (a) rank-3 tile on rank-2 data
    //   (b) rank-2 tile on rank-3 data
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let x_iv = IterVar(2);
    let z_iv = IterVar(3);

    // ---- (a) rank-3 tile on rank-2 data ----
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    let tile_3 = IterTile::new(vec![(y_iv, 1..8), (x_iv, 1..8), (z_iv, 0..4)]);
    let (ids, tiles) = one_pair(data, seq, 0, tile_3.clone());
    let err = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile_3)
        .expect_err("rank-3 tile must fail loud");
    let msg = format!("{err}");
    assert!(
        msg.contains("tile rank 3") && msg.contains("2D row-loop"),
        "expected ContractGap mentioning the rank-3 tile + 2D-only support; \
         got: {msg}"
    );

    // ---- (b) rank-2 tile on rank-3 data ----
    let (names, sidecar) = make_minimal_tables(data, "vol_in", vec![16, 16, 4]);
    let tile_2 = IterTile::new(vec![(y_iv, 1..8), (x_iv, 1..8)]);
    let (ids, tiles) = one_pair(data, seq, 0, tile_2.clone());
    let err = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile_2)
        .expect_err("rank-3 data must fail loud");
    let msg = format!("{err}");
    assert!(
        msg.contains("data dim rank 3") && msg.contains("2D row-loop"),
        "expected ContractGap mentioning the rank-3 data + 2D-only support; \
         got: {msg}"
    );
}

#[test]
fn inner_axis_out_of_bounds_returns_contract_gap() {
    // TASK-0294 validation surface: an inner-axis range exceeding
    // `dims[1]` is a compiler-pass invariant violation. Must fail
    // loud with a typed `EmitError::ContractGap` rather than emit
    // an out-of-bounds slice.
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let x_iv = IterVar(2);
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    // Inner range 0..32 exceeds inner_dim=16.
    let tile = IterTile::new(vec![(y_iv, 1..8), (x_iv, 0..32)]);
    let (ids, tiles) = one_pair(data, seq, 0, tile.clone());

    let err = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect_err("inner-axis out-of-bounds must fail loud");
    let msg = format!("{err}");
    assert!(
        msg.contains("inner-axis range") && msg.contains("inner-dim 16"),
        "expected ContractGap mentioning the offending inner-axis range + \
         inner-dim; got: {msg}"
    );
}

/// TASK-0316 AC#1: backend-side CONSUMER pin for the
/// `bounds[i] ↔ ty.dims[i]` positional contract on the
/// inner-axis-leading bounds shape that cycle-133's
/// `order_halo_strip_bounds_by_data_dim` produces.
///
/// `order_halo_strip_bounds_by_data_dim` (transfer_inject.rs, cycle
/// 133) re-orders the halo-strip bounds vector so `bounds[i]` indexes
/// `ty.dims[i]`. This test pins the BACKEND side of that contract:
/// fed the inner-leading tile shape (`bounds[0] = inner_iv`,
/// `bounds[1] = outer_iv` — what the cycle-133 helper emits for
/// `data` indexed `[inner_iv][outer_iv]`), `render_wait_assign` MUST
/// drive the 2D row-loop with `outer_lo..outer_hi` = `bounds[0].1`
/// (the inner_iv's range, NOT the outer_iv's) and `row_stride` =
/// `dims[1]` (the outer_iv-dim stride, NOT `dims[0]`).
///
/// Asymmetric dims `[16, 8]` make the test bite: under the canonical
/// `(outer_iv, inner_iv)` order the 1D path's stride product is the
/// same 8 (it falls out from `dims[1..]`), but the slice ranges and
/// row-loop bounds differ — the row-loop's `outer_lo..outer_hi` is
/// fed from `bounds[0].1`, so a `wait_slice` that read bounds out
/// of dim-position order would yield `for _y in 4..5` instead of
/// `for _y in 0..8`.
///
/// What this test catches: a future `wait_slice` refactor that
/// drops the positional `bounds[i] ↔ ty.dims[i]` semantics (e.g.
/// switches to an iv-name lookup or sorts on a different key) would
/// silently re-permute cycle-133's dim-ordered output and slice-paste
/// against the wrong dim. This pin asserts the positional contract
/// directly with hand-constructed dim-ordered bounds.
///
/// What this test does NOT catch: producer-side cycle-133 helper
/// rot. The test fabricates an `IterTile` directly and never invokes
/// `inject_transfers` or the cycle-133 helper. If
/// `order_halo_strip_bounds_by_data_dim` regressed to emit
/// canonical-order bounds, this test would still pass — the producer-
/// side coverage lives in
/// `nucleus-compiler/tests/halo_strip_synth.rs::task0306_ac3`.
///
/// Transitive coverage: cycle-135's `rewrite_partition_tiles_inner`
/// helper feeds the same `IterTile.bounds` consumed by the same
/// `wait_slice` dispatch, so this pin also covers the TASK-0317
/// helper's backend-side positional contract.
#[test]
fn task0316_inner_axis_leading_layout_emits_against_dim0() {
    let data = DataId(99);
    let seq = SeqTag(3);
    // The cycle-133 fixture names: outer_iv = y = IterVar(7),
    // inner_iv = x = IterVar(8). Names carried for traceability; the
    // backend reads bounds positionally and is iv-name agnostic.
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    // Data layout: inner-axis-leading. dims[0] = inner_iv's range
    // (16), dims[1] = outer_iv's range (8). Asymmetric on purpose so
    // a misordered bounds vector emits a different string than the
    // dim-aligned bounds vector below.
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 8]);
    // The cycle-133 emit for an inner-leading S-strip (analogous to
    // AC#3 in halo_strip_synth.rs, adjusted for asymmetric dims):
    //   bounds[0] = (inner_iv, 0..8)  ↔ data dim 0 (inner_iv-dim 16)
    //   bounds[1] = (outer_iv, 4..5)  ↔ data dim 1 (outer_iv-dim 8)
    let tile = IterTile::new(vec![(inner_iv, 0..8), (outer_iv, 4..5)]);
    let (ids, tiles) = one_pair(data, seq, 7, tile.clone());

    let out = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect("inner-leading 2D Wait must render");

    // row_stride = product(dims[1..]) = 8. inner_stride =
    // product(dims[2..]) = 1 (empty product). outer_lo..outer_hi =
    // bounds[0].1 = 0..8. inner_lo_off..inner_hi_off =
    // bounds[1].1 * inner_stride = 4..5.
    assert!(
        out.contains(
            "{ let _tmp = ring_7.wait(); \
             for _y in 0usize..8usize { \
             let _r = _y * 8usize; \
             img_in[_r + 4usize.._r + 5usize].copy_from_slice(\
             &_tmp[_r + 4usize.._r + 5usize]); } }"
        ),
        "TASK-0316 AC#1: inner-leading bounds [(inner_iv, 0..8), \
         (outer_iv, 4..5)] on data dims [16, 8] MUST drive row-loop \
         outer=0..8 + row_stride=8 + inner=4..5; got:\n{out}"
    );
    // Negative-pin the canonical-order positional footprint: a
    // `wait_slice` refactor that read `bounds[0]` as the outer_iv
    // slot regardless of dim layout (i.e. dropped positional
    // dim-mapping) would emit `for _y in 4usize..5usize { let _r =
    // _y * 8usize; img_in[_r + 0usize.._r + 8usize]...`. Assert
    // that string is NOT present.
    assert!(
        !out.contains("for _y in 4usize..5usize"),
        "TASK-0316 AC#1: `wait_slice` appears to have dropped the \
         positional `bounds[i] ↔ ty.dims[i]` contract — the row-loop \
         bounds (4..5) come from the outer_iv slot rather than \
         `bounds[0].1` (0..8 for the inner-leading layout). The \
         cycle-133 helper's dim-ordered output is being silently \
         re-permuted by the consumer. Got:\n{out}"
    );
}

/// TASK-0316 AC#2: backend-side CONSUMER pin for the empty-bounds
/// shape that cycle-133's `order_halo_strip_bounds_by_data_dim`
/// produces for non-prefix data layouts (data indexed by an iv NOT
/// covered by the partition).
///
/// `order_halo_strip_bounds_by_data_dim` (and the sibling
/// `compute_partition_bounds_with_dim_prefix` /
/// `rewrite_partition_tiles_inner` guards from cycle 134-135) return
/// an EMPTY bounds vector when the data's leading dim's iv is not in
/// `partition_worker_ranges` — the safe default is whole-array drop
/// rather than mis-mapping `bounds[0]` to a partitioned iv that does
/// not index dim 0.
///
/// This pins that the backend dispatches such an empty-bounds tile
/// to the whole-array `name = rhs;` arm. DISTINCT dispatch path
/// from `whole_array_assign_when_tile_empty` above: that test passes
/// an EMPTY `pair_tiles` map (`render_wait_assign`'s
/// `pair_tiles.get(...).is_none()` short-circuit fires before
/// `wait_slice` is called). This test passes a POPULATED
/// `pair_tiles` map carrying an `IterTile::empty()` so dispatch
/// enters `wait_slice` and hits the `tile.bounds.first()`
/// else-branch early return. Both arms converge on the same emit
/// but the two assertions cover the two distinct receiver-side
/// dispatches.
#[test]
fn task0316_non_prefix_layout_empty_bounds_consumer_pin() {
    let data = DataId(99);
    let seq = SeqTag(3);
    let (names, sidecar) = make_minimal_tables(data, "img_in", vec![16, 16]);
    // Cycle-133 / cycle-134 / cycle-135 emit shape for non-prefix
    // data layouts: empty bounds vector (whole-array drop).
    let tile = IterTile::empty();
    let (ids, tiles) = one_pair(data, seq, 0, tile.clone());

    let out = render_one_wait(&names, &sidecar, &ids, &tiles, data, seq, tile)
        .expect("empty-bounds Wait must render");

    assert!(
        out.contains("img_in = ring_0.wait();"),
        "TASK-0316 AC#2: empty bounds via populated `pair_tiles` \
         (non-prefix data layout) MUST dispatch through `wait_slice`'s \
         `bounds.first()` early-return arm to the whole-array assign \
         `name = rhs;`; got:\n{out}"
    );
    assert!(
        !out.contains("copy_from_slice") && !out.contains("for _y in"),
        "TASK-0316 AC#2: empty bounds MUST NOT fall through to a \
         slice-paste arm (a `wait_slice` regression that interpreted \
         empty bounds as a partial-slice would undo cycle-133's \
         defensive whole-array drop). Got:\n{out}"
    );
}

/// TASK-0321 (cycle-139 TASK-0295 AC#3 gap closure): the 2D row-loop
/// slice-paste arm in `wait_slice` builds its `_tmp = X.wait()` rhs by
/// substituting `WalkerCtx::rendezvous_prefix` inside the Wait
/// match-arm of `render_worker_events_inner` (grep-witness:
/// `grep -nE '\{rendezvous_prefix\}_\{rid\}\.wait'
/// nucleus/backend-common/src/multi_worker_walker.rs` returns the
/// single Wait substitution site) — a single `format!("{prefix}\
/// {rendezvous_prefix}_{rid}.wait()")` call with no prefix-conditional
/// branches in the 2D dispatch. The four tier-1 backends use four
/// distinct prefix values: `"slot"` (pthreads-sync), `"ring"`
/// (pthreads-async), `"chan"` (mp-tcp-event); mp-tcp-bufsync bypasses
/// `render_worker_events` entirely and calls `render_wait_assign`
/// directly with no prefix involvement.
///
/// All other 2D-tile pins in this file feed `rendezvous_prefix: "ring"`
/// via the shared `render_one_wait` helper. If a future refactor
/// hardcoded `"ring_"` inside the 2D arm (or the `_tmp = X.wait()`
/// builder), the existing pins would still pass because they
/// already feed `"ring"`; pthreads-sync and mp-tcp-event would
/// silently emit wrong rendezvous identifiers for partition=blocks2d.
///
/// This test pins the prefix substitution on the 2D row-loop arm
/// across all three `render_worker_events`-using prefixes.
#[test]
fn task0321_rendezvous_prefix_substituted_in_2d_row_loop_arm() {
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let x_iv = IterVar(2);
    let (names, sidecar) = make_minimal_tables(data, "img_out", vec![16, 16]);
    let tile = IterTile::new(vec![(y_iv, 1..8), (x_iv, 1..8)]);
    let (ids, tiles) = one_pair(data, seq, 12, tile.clone());

    for prefix in ["ring", "slot", "chan"] {
        let ctx = WalkerCtx {
            names: &names,
            sidecar: &sidecar,
            rendezvous_prefix: prefix,
            rendezvous_ids: &ids,
            pair_tiles: &tiles,
            accumulate_waits: WalkerCtx::empty_accumulate_set(),
            let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
        };
        let wait = Event::Wait {
            src: WorkerId(0),
            data,
            tile: tile.clone(),
            seq,
        };
        let mut out = String::new();
        render_worker_events(&ctx, WorkerId(1), &[wait], &mut out, 0, "")
            .expect("2D row-loop Wait must render under each prefix");

        // The `_tmp = X.wait()` builder MUST substitute the configured
        // prefix verbatim. A regression that hardcoded `"ring_"`
        // (or any other prefix string) inside the 2D arm would emit
        // the wrong rendezvous identifier here.
        let expected_rhs = format!("let _tmp = {prefix}_12.wait();");
        assert!(
            out.contains(&expected_rhs),
            "TASK-0321: 2D row-loop arm MUST substitute \
             `rendezvous_prefix = {prefix:?}` into the `_tmp` builder; \
             expected substring `{expected_rhs}`; got:\n{out}"
        );

        // Defensive: a regression that hardcoded `"ring_"` would
        // emit a `ring_12.wait()` even when the configured prefix is
        // `slot` or `chan`. Assert the WRONG prefixes are NOT present.
        for other in ["ring", "slot", "chan"] {
            if other == prefix {
                continue;
            }
            let unexpected = format!("{other}_12.wait()");
            assert!(
                !out.contains(&unexpected),
                "TASK-0321: 2D row-loop arm under \
                 `rendezvous_prefix = {prefix:?}` MUST NOT emit the \
                 unrelated prefix substring `{unexpected}` (would \
                 indicate a hardcoded `{other}_` in the 2D arm); \
                 got:\n{out}"
            );
        }

        // Sanity: the row-loop shape itself MUST be present (we are
        // pinning the 2D arm; if dispatch fell through to whole-array
        // or 1D, the prefix substitution would also be wrong).
        assert!(
            out.contains("for _y in 1usize..8usize"),
            "TASK-0321: must dispatch to 2D row-loop arm regardless of \
             prefix (`rendezvous_prefix = {prefix:?}`); got:\n{out}"
        );
    }
}

/// TASK-0322 (cycle-141 sibling closure to TASK-0321 cycle-140
/// architect P2): the Push-side substitution site (grep-witness:
/// `grep -nE '\{rendezvous_prefix\}_\{rid\}\.push'
/// nucleus/backend-common/src/multi_worker_walker.rs` returns the
/// single Push substitution site inside `render_worker_events_inner`)
/// is structurally identical to the Wait site documented above — a
/// single `format!("{prefix}\
/// {rendezvous_prefix}_{rid}.push(...)")` call with no prefix-
/// conditional branches in the Push branch. TASK-0321 pinned the
/// Wait site across `{"ring", "slot", "chan"}` but constructs only
/// an `Event::Wait`, so the Push site is uncovered by that test.
///
/// A regression that hardcoded `"ring_"` (or any other prefix
/// string) inside the Push emit would silently break partition=
/// blocks2d (and any other multi-worker) Push emit for pthreads-
/// sync (`"slot"`) and mp-tcp-event (`"chan"`); pthreads-async
/// (`"ring"`) would coincidentally still pass because the
/// hardcoded value matches its configured prefix.
///
/// This test pins the Push-side substitution across the same three
/// `render_worker_events`-using prefixes used by TASK-0321. Shape
/// (`DataId` / tile / `RendezvousId`) is held identical to
/// `task0321_rendezvous_prefix_substituted_in_2d_row_loop_arm` so
/// the two tests differ ONLY in the event constructor (`Wait` vs
/// `Push`) — sibling-adjacency by construction.
#[test]
fn task0322_rendezvous_prefix_substituted_on_push_emit() {
    let data = DataId(7);
    let seq = SeqTag(3);
    let y_iv = IterVar(1);
    let x_iv = IterVar(2);
    let (names, sidecar) = make_minimal_tables(data, "img_out", vec![16, 16]);
    let tile = IterTile::new(vec![(y_iv, 1..8), (x_iv, 1..8)]);
    let (ids, tiles) = one_pair(data, seq, 12, tile.clone());

    for prefix in ["ring", "slot", "chan"] {
        let ctx = WalkerCtx {
            names: &names,
            sidecar: &sidecar,
            rendezvous_prefix: prefix,
            rendezvous_ids: &ids,
            pair_tiles: &tiles,
            accumulate_waits: WalkerCtx::empty_accumulate_set(),
            let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
        };
        let push = Event::Push {
            dst: WorkerId(1),
            data,
            tile: tile.clone(),
            seq,
        };
        let mut out = String::new();
        // Render on WorkerId(0) ("w0"): the sender. WorkerId(1) is
        // the dst ("host" from `make_minimal_tables`), so the
        // emitted comment will read `// send `img_out` to host`.
        render_worker_events(&ctx, WorkerId(0), &[push], &mut out, 0, "")
            .expect("Push must render under each prefix");

        // The Push emit MUST substitute the configured prefix
        // verbatim. A regression that hardcoded `"ring_"` (or any
        // other prefix string) inside the Push branch would emit
        // the wrong rendezvous identifier here.
        let expected_push = format!("{prefix}_12.push(img_out.clone());");
        assert!(
            out.contains(&expected_push),
            "TASK-0322: Push emit MUST substitute \
             `rendezvous_prefix = {prefix:?}` into the `.push(...)` \
             call; expected substring `{expected_push}`; got:\n{out}"
        );

        // Defensive: a regression that hardcoded any other prefix
        // would emit, say, `ring_12.push(` even when the configured
        // prefix is `slot` or `chan`. Assert the WRONG prefixes are
        // NOT present on the Push call. Match on the full
        // `{other}_12.push(` substring rather than just the prefix
        // word (which can legitimately appear in unrelated text
        // like the rendezvous-ident, were any present).
        for other in ["ring", "slot", "chan"] {
            if other == prefix {
                continue;
            }
            let unexpected = format!("{other}_12.push(");
            assert!(
                !out.contains(&unexpected),
                "TASK-0322: Push emit under \
                 `rendezvous_prefix = {prefix:?}` MUST NOT emit the \
                 unrelated prefix substring `{unexpected}` (would \
                 indicate a hardcoded `{other}_` in the Push branch); \
                 got:\n{out}"
            );
        }

        // Sanity: confirm the Push branch was entered (not falling
        // through to some other event handler — e.g. a future
        // refactor that accidentally routed Push through `wait_slice`).
        // The `// send `{name}` to {to}` comment is emitted ONLY by
        // the Push branch inside `render_worker_events_inner`.
        assert!(
            out.contains("// send `img_out` to host"),
            "TASK-0322: must enter the Push branch (which emits the \
             `// send ... to ...` comment); `rendezvous_prefix = \
             {prefix:?}`; got:\n{out}"
        );
    }
}
