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

    let out = render_one_wait(
        &names,
        &sidecar,
        &ids,
        &tiles,
        data,
        seq,
        IterTile::empty(),
    )
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
