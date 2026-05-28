//! Unified whole-array classifier semantics (TASK-0355, cycle 225).
//!
//! These tests pin the cycle-225 consolidation of two pre-unification
//! whole-array classifiers:
//!
//! - `is_whole_array_tile` (collect.rs, predates TASK-0349) — removed
//!   cycle 225.
//! - `is_whole_array_recv` (wait.rs, TASK-0349 cycle 220) — now the
//!   canonical classifier consulted by BOTH the accumulator detector
//!   (`collect_accumulate_waits`, this file's exercise surface) AND
//!   the let-at-wait classifier (`collect_let_at_wait_data`,
//!   exercised by `collect_let_at_wait_data.rs`).
//!
//! ## Why we test via `collect_accumulate_waits` rather than calling
//! `is_whole_array_recv` directly
//!
//! `is_whole_array_recv` is `pub(super)` (narrowed cycle 220b architect
//! P2.2); the visibility was deliberately closed to avoid out-of-crate
//! direct consumers. Tests/ files cannot reach `pub(super)` items.
//! These tests therefore exercise the classifier indirectly through
//! the migrated cycle-225 accumulator-detection call site
//! (`collect_accumulate_waits` is `pub`). The let-at-wait sibling site
//! is already pinned by `collect_let_at_wait_data.rs` (cycle 221).
//!
//! ## What this file pins (TASK-0355 AC#4)
//!
//! The edge-case shapes the cycle-220b architect P3.1 narrative
//! flagged as the pre-unification divergence surface, plus the one
//! genuine behaviour-divergence corner found by the cycle-225 architect
//! P2 review:
//!
//! 1. `whole_via_empty_bounds` — IterTile::empty() → whole.
//! 2. `whole_via_scalar_with_non_empty_bounds` — ty.dims=[] +
//!    non-empty tile → whole (wait_slice:265 early-return).
//! 3. `not_whole_via_partial_leading_range` — rank-1 slice → not whole.
//! 4. `whole_via_2d_both_axes_full` — rank-2 [full, full] → whole
//!    (wait_slice:337 both-axes-full → None).
//! 5. `not_whole_via_rank3_guard` — rank-3 shape → wait_slice ERR
//!    (wait_slice:307 rank-3+ guard), swallowed to `false` at the
//!    call site = not-whole-array, accumulate NOT detected. THE
//!    genuine pre-unification divergence pin (OLD is_whole_array_tile
//!    = true; NEW = Err→false).
//! 6. `not_whole_via_oob_leading_range_err_swallow_path` — leading
//!    range OOB → wait_slice ERR (wait_slice:269-278), swallowed to
//!    false. A PATH pin, not a divergence pin (OLD also returned false).
//! 7. `accumulate_classifies_empty_tile_even_when_data_type_absent` —
//!    DELIBERATE-DECISION pin for the ONE behaviour-divergence corner
//!    (cycle-225 architect P2): empty-tile fan-in on a data symbol with
//!    absent ResolvedType. OLD skipped it (silent `name = rhs;`
//!    overwrite); NEW classifies accumulate → render_wait_assign
//!    surfaces a LOUD ContractGap. Fail-loud preferred over
//!    silent-wrong.
//!
//! ## What this does NOT pin
//!
//! - End-to-end behaviour for shipped schedules — cycle-220b architect
//!   P3.1 established that no shipped schedule today exercises a tile
//!   shape that triggers the pre-unification divergence (the 5 wait_slice
//!   error arms). The e2e baseline (280/246/0/34/0 at cycle 225) is
//!   the orthogonal end-to-end pin. These tests target the formal
//!   semantic of the unified classifier at the unit-test layer so a
//!   future cycle that changes wait_slice's guard chain (e.g.
//!   TASK-0341.02.02.01.01 extending to N-D dispatch) will BREAK the
//!   tests in a precise, attributable way — not silently change
//!   accumulate-detection.

use std::collections::BTreeMap;
use std::ops::Range;

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use backend_common::multi_worker_walker::collect_accumulate_waits;

type PairTiles = BTreeMap<(DataId, SeqTag), IterTile>;

fn sidecar_with(data: DataId, scalar: ScalarType, dims: Vec<usize>) -> NameSidecar {
    let mut sidecar = NameSidecar::default();
    sidecar
        .data_types
        .insert(data, ResolvedType { scalar, dims });
    sidecar
}

fn tile_1d(iv: u64, range: Range<i64>) -> IterTile {
    IterTile::new(vec![(IterVar(iv), range)])
}

fn tile_2d(iv0: u64, r0: Range<i64>, iv1: u64, r1: Range<i64>) -> IterTile {
    IterTile::new(vec![(IterVar(iv0), r0), (IterVar(iv1), r1)])
}

fn tile_3d(
    iv0: u64,
    r0: Range<i64>,
    iv1: u64,
    r1: Range<i64>,
    iv2: u64,
    r2: Range<i64>,
) -> IterTile {
    IterTile::new(vec![
        (IterVar(iv0), r0),
        (IterVar(iv1), r1),
        (IterVar(iv2), r2),
    ])
}

fn two_waits_with_tile(data: DataId, tile: IterTile) -> (Vec<Event>, PairTiles) {
    let mut events: Vec<Event> = Vec::new();
    let mut tiles: PairTiles = BTreeMap::new();
    for i in 0u64..2 {
        let seq = SeqTag(i);
        let src = WorkerId(i + 1);
        events.push(Event::Wait {
            src,
            data,
            tile: tile.clone(),
            seq,
        });
        tiles.insert((data, seq), tile.clone());
    }
    (events, tiles)
}

#[test]
fn whole_via_empty_bounds() {
    // IterTile::empty() — no enclosing iteration nest. wait_slice:256
    // early-returns Ok(None). is_whole_array_recv -> Ok(true).
    // collect_accumulate_waits classifies both Waits as accumulate.
    let data = DataId(0);
    let sidecar = sidecar_with(data, ScalarType::I32, vec![16]);
    let (events, tiles) = two_waits_with_tile(data, IterTile::empty());

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert_eq!(
        acc.len(),
        2,
        "IterTile::empty() MUST classify as whole-array (wait_slice:256 \
         early-return); both Waits should be accumulate. Got: {acc:?}"
    );
}

#[test]
fn whole_via_scalar_with_non_empty_bounds() {
    // Scalar ty.dims=[] + non-empty tile bounds. wait_slice:265 early-
    // return Ok(None) BEFORE the leading-axis read (which would have
    // panicked on ty.dims[0]). Ordering pin: empty-dims check MUST
    // precede ty.dims[0] read.
    let data = DataId(0);
    let sidecar = sidecar_with(data, ScalarType::I32, vec![]);
    let tile = tile_1d(0, 0..4);
    let (events, tiles) = two_waits_with_tile(data, tile);

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert_eq!(
        acc.len(),
        2,
        "Scalar data (dims=[]) MUST classify as whole-array even with a \
         non-empty tile (wait_slice:265 early-return); both Waits should be \
         accumulate. Got: {acc:?}"
    );
}

#[test]
fn not_whole_via_partial_leading_range() {
    // Rank-1 partial slice: tile [0..2] over data dims [4]. wait_slice
    // returns Ok(Some(WaitSlice::Flat{..})). is_whole_array_recv ->
    // Ok(false). Not classified as accumulate.
    let data = DataId(0);
    let sidecar = sidecar_with(data, ScalarType::I32, vec![4]);
    let tile = tile_1d(0, 0..2);
    let (events, tiles) = two_waits_with_tile(data, tile);

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert!(
        acc.is_empty(),
        "Partial-range tile (0..2 over dims [4]) MUST NOT classify as \
         whole-array; accumulate should NOT fire. Got: {acc:?}"
    );
}

#[test]
fn whole_via_2d_both_axes_full() {
    // Rank-2 [full, full] over data dims [4, 8]. wait_slice:337 returns
    // Ok(None) (degenerate-degenerate-whole-array assign for emit identity).
    // is_whole_array_recv -> Ok(true).
    let data = DataId(0);
    let sidecar = sidecar_with(data, ScalarType::I32, vec![4, 8]);
    let tile = tile_2d(0, 0..4, 1, 0..8);
    let (events, tiles) = two_waits_with_tile(data, tile);

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert_eq!(
        acc.len(),
        2,
        "Rank-2 [full, full] tile MUST classify as whole-array \
         (wait_slice:337 both-axes-full → None); both Waits should be \
         accumulate. Got: {acc:?}"
    );
}

#[test]
fn not_whole_via_rank3_guard() {
    // Rank-3 tile + rank-3 data dims. wait_slice:307 rank-3+ guard
    // returns Err(ContractGap). is_whole_array_recv propagates Err;
    // collect_accumulate_waits's call site swallows with
    // `.unwrap_or(false)` (cycle-225 unified-classifier site convention).
    // Net: NOT classified as accumulate.
    //
    // Pre-cycle-225 divergence pin: the removed `is_whole_array_tile`
    // would have classified [full, full, full] over dims [4, 4, 4] as
    // whole-array (its loop checked each consulted bound against the
    // corresponding dim without an explicit rank-3+ guard). The unified
    // classifier surfaces the rank-3+ shape as Err — conservative
    // semantic, breaks accumulate-detection on this shape.
    //
    // Today no shipped schedule trips this (cycle-220b architect P3.1
    // narrative); a future cycle that extends wait_slice to N-D dispatch
    // (TASK-0341.02.02.01.01) would lift the Err and this test would
    // need to be updated alongside that lift.
    let data = DataId(0);
    let sidecar = sidecar_with(data, ScalarType::I32, vec![4, 4, 4]);
    let tile = tile_3d(0, 0..4, 1, 0..4, 2, 0..4);
    let (events, tiles) = two_waits_with_tile(data, tile);

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert!(
        acc.is_empty(),
        "Rank-3 tile + rank-3 data MUST NOT classify as whole-array \
         (wait_slice:307 rank-3+ guard returns Err; cycle-225 call site \
         swallows to false). Got: {acc:?}"
    );
}

#[test]
fn not_whole_via_oob_leading_range_err_swallow_path() {
    // OOB leading range: tile [0..8] over data dims [4]. wait_slice:269-278
    // returns Err(ContractGap). Cycle-225 unified-classifier call site
    // swallows with `.unwrap_or(false)`. Net: NOT classified as accumulate.
    //
    // NOT a pre-unification divergence pin (cf. `not_whole_via_rank3_guard`,
    // which IS): the removed `is_whole_array_tile` ALSO returned false here
    // (its loop checked `range.start != 0 || range.end != dim_len` — range
    // 0..8 has end=8 != 4 → false). Same end-result on this shape; the test
    // pins the new Err-swallow PATH, not a behaviour change. Named
    // `_err_swallow_path` to avoid implying divergence (cycle-225 architect
    // P3.3).
    let data = DataId(0);
    let sidecar = sidecar_with(data, ScalarType::I32, vec![4]);
    let tile = tile_1d(0, 0..8);
    let (events, tiles) = two_waits_with_tile(data, tile);

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert!(
        acc.is_empty(),
        "OOB leading range (0..8 over dims [4]) MUST NOT classify as \
         whole-array (wait_slice:269-278 returns Err; cycle-225 call site \
         swallows to false). Got: {acc:?}"
    );
}

#[test]
fn accumulate_classifies_empty_tile_even_when_data_type_absent() {
    // DELIBERATE-DECISION PIN (cycle-225 architect P2): empty-tile fan-in
    // on a data symbol with NO ResolvedType in the sidecar (a contract
    // gap — build_sidecar is contracted to carry every ACFG data symbol,
    // so this is structurally unreachable for a link-valid program).
    //
    // This is the ONE corner where the cycle-225 unification changed
    // behaviour vs the removed `is_whole_array_tile`:
    //   - OLD: collect_accumulate_waits did `match sidecar.data_type(data)
    //     { None => continue }` BEFORE the tile classification, so a
    //     None-data-type group was skipped → NOT accumulate → emit fell
    //     to the silent `name = rhs;` last-write-wins overwrite.
    //   - NEW: is_whole_array_recv → wait_slice checks `tile.bounds.first()`
    //     FIRST (empty tile → Ok(None) → whole) BEFORE the sidecar lookup
    //     (wait.rs:256 precedes wait.rs:259), so the group IS classified
    //     accumulate. render_wait_assign then surfaces the missing
    //     ResolvedType as EmitError::ContractGap (wait.rs:106-110) —
    //     fail-LOUD instead of silent last-write-wins.
    //
    // The new behaviour is deliberately preferred: fail-loud on a
    // contract gap beats silently emitting a wrong overwrite. This test
    // PINS that decision so a future cycle that re-introduces the
    // None-data-type early-skip does so consciously (and updates this
    // test), rather than silently reverting to the silent-overwrite path.
    let data = DataId(0);
    let sidecar = NameSidecar::default(); // no data_type entry for `data`

    let (events, tiles) = two_waits_with_tile(data, IterTile::empty());

    let acc = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert_eq!(
        acc.len(),
        2,
        "Empty-tile fan-in with absent ResolvedType is classified as \
         accumulate post-cycle-225 (wait_slice's empty-tile early-return \
         precedes the sidecar lookup); the downstream render_wait_assign \
         surfaces the missing type as a LOUD ContractGap. Got: {acc:?}"
    );
}
