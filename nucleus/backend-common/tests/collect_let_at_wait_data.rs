//! TASK-0354 cycle 221: unit pins for the let-at-wait classifier
//! landed in cycle 220 (TASK-0349). Architect P2.2 follow-up: before
//! this file the two helpers had only INDIRECT coverage via the
//! e2e differential matrix. The project's behaviour-pin discipline
//! (TASK-0304, TASK-0310) requires direct unit tests for any newly
//! shipped public classifier.
//!
//! Helpers under test:
//! - `backend_common::multi_worker_walker::collect_let_at_wait_data`
//!   (public) — the entry point all tests drive directly.
//! - `is_whole_array_recv` (`pub(super)` in `wait.rs`, narrowed cycle
//!   220b per architect P2.2) — exercised INDIRECTLY through the
//!   call site at `collect.rs:392`. Visibility is intentionally
//!   `pub(super)`; we do not widen it.
//!
//! The 7 cases below map 1:1 to the numbered acceptance items in
//! `backlog task 0354 --plain`:
//!
//! 1. `mixed_whole_and_slice_waits_excludes_data` — one whole + one
//!    slice Wait on the same DataId leaves it OUT of the result.
//! 2. `accumulate_fan_in_data_excluded` — accumulate-fan-in DataId
//!    stays OUT even when every Wait on it is whole-array (the
//!    `wrapping_add` identity needs the zero-init to be live).
//!    Test drives the classifier via direct `accumulate_data`
//!    input-arg insertion (the upstream `collect_accumulate_waits`
//!    is not exercised here — that helper has its own tests).
//! 3. `indexed_input_data_excluded` — DataId in the indexed-input
//!    arg (computed upstream from indexed Fire-writes) stays OUT
//!    (the indexed assigns need the zero-init). Test drives the
//!    classifier via direct input-arg insertion, not by
//!    constructing an Event::Fire — the helper signature takes
//!    `indexed` as a precomputed input.
//! 4. `empty_waits_yields_empty_set` — no Wait events → empty set.
//! 5. `shape_error_on_wait_slice_excludes_data` — an out-of-bounds
//!    leading-axis range trips `wait_slice`'s guard
//!    (`wait.rs:269..278`). `is_whole_array_recv` propagates the
//!    `Err` upward; `collect_let_at_wait_inner` swallows it with
//!    `.unwrap_or(false)` (`collect.rs:392`), so the data ends up
//!    in `not_all_whole` and is excluded. This is the silent-
//!    defensive arm the architect specifically called out.
//! 6. `whole_array_wait_inside_event_loop_body_included` — the
//!    classifier descends into `Event::Loop` bodies; a whole-array
//!    Wait buried inside a loop body is still included.
//! 7. `scalar_data_no_dims_treated_as_whole_array` — a DataId typed
//!    `dims: vec![]` (scalar) hits the `ty.dims.is_empty()` arm of
//!    `wait_slice` (`wait.rs:265..267`) which returns `Ok(None)` →
//!    classified as whole-array → included.
//!
//! Cycle 223 (TASK-0357) extends the `is_whole_array_recv` guard
//! coverage that cycle 221's cases 1, 5, 7 left structurally
//! unexercised:
//!
//! 8. `rank_3_tile_shape_excludes_data` — a rank-3 tile (3 bounds
//!    entries) on rank-2 data trips `wait_slice`'s rank-3+ guard
//!    (`wait.rs:307`), which fires `Err(ContractGap)` because the
//!    2D row-loop slice-paste's supported shape is rank ≤ 2 on
//!    both. `is_whole_array_recv` propagates the Err; the
//!    classifier's `.unwrap_or(false)` arm classifies "not whole"
//!    → excluded. This is the cycle-209 16-jacobi/distributed
//!    next-layer blocker; TASK-0341.02.02.01.01 lifts to N-D
//!    nested-loop dispatch.
//! 9. `inner_axis_oob_excludes_data` — a 2-bound tile on rank-2
//!    data where the inner range exceeds `dims[1]` trips
//!    `wait_slice`'s inner-axis OOB guard (`wait.rs:324..333`),
//!    symmetric to test 5's leading-axis OOB. Err propagates →
//!    excluded.
//!
//! Why drive `collect_let_at_wait_data` rather than
//! `is_whole_array_recv` directly: visibility (above) AND the
//! classifier's combinator semantics (waited ∩ ¬not_all_whole ∩
//! ¬accumulate ∩ ¬indexed) are what each numbered case actually
//! pins; the wrapper's truth value alone is uninteresting in
//! isolation.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};

use backend_common::multi_worker_walker::collect_let_at_wait_data;

mod common;
use common::{empty_accumulate_and_indexed, make_minimal_tables, tile_1d, tile_2d, tile_3d};

#[test]
fn mixed_whole_and_slice_waits_excludes_data() {
    // Two Wait events on the same DataId: one whole-array (no entry
    // in pair_tiles → `wait_slice` returns Ok(None) → whole), one
    // slice-paste (a 1D tile NOT covering the full leading-dim).
    // The classifier requires EVERY Wait on a data to be whole; one
    // slice arm alone puts the data into `not_all_whole` →
    // excluded.
    let data = DataId(7);
    let seq_whole = SeqTag(1);
    let seq_slice = SeqTag(2);
    let dim: usize = 16;
    let (_names, sidecar) = make_minimal_tables(data, "img", vec![dim]);

    // pair_tiles registers a SLICE tile for `seq_slice` only.
    // `seq_whole` has NO entry → the `None` arm of
    // `collect.rs:389-390` → whole-array.
    let mut pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    pair_tiles.insert((data, seq_slice), tile_1d(0, 0..(dim as i64) / 2));

    let events = vec![
        Event::Wait {
            src: WorkerId(0),
            data,
            tile: IterTile::empty(),
            seq: seq_whole,
        },
        Event::Wait {
            src: WorkerId(0),
            data,
            tile: tile_1d(0, 0..(dim as i64) / 2),
            seq: seq_slice,
        },
    ];

    let (acc, indexed) = empty_accumulate_and_indexed();
    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        !result.contains(&data),
        "mixed whole+slice on same data must exclude it; got: {result:?}"
    );
}

#[test]
fn accumulate_fan_in_data_excluded() {
    // Even when every Wait on the data is whole-array, membership
    // in `accumulate_data` pulls the DataId OUT of the result —
    // the wrapping_add accumulate identity needs the zero-init to
    // remain live (`collect.rs:367-369`).
    let data = DataId(11);
    let seq = SeqTag(3);
    let (_names, sidecar) = make_minimal_tables(data, "hist", vec![8]);

    // Empty pair_tiles → all Waits resolve to whole-array.
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let events = vec![Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    }];

    let mut accumulate_data: BTreeSet<DataId> = BTreeSet::new();
    accumulate_data.insert(data);
    let indexed: BTreeSet<DataId> = BTreeSet::new();

    let result =
        collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &accumulate_data, &indexed);
    assert!(
        !result.contains(&data),
        "accumulate-fan-in data must be excluded; got: {result:?}"
    );
}

#[test]
fn indexed_input_data_excluded() {
    // Symmetric to `accumulate_fan_in_data_excluded`: a whole-array
    // Wait that would otherwise qualify is excluded purely because
    // the data is in the `indexed` input arg (computed upstream from
    // indexed Fire-writes by `collect_pre_init_sets`; `collect.rs:
    // 370-372`). Test drives the classifier via direct input-arg
    // insertion, not by building an Event::Fire — the helper's
    // signature takes `indexed` as precomputed input.
    let data = DataId(13);
    let seq = SeqTag(5);
    let (_names, sidecar) = make_minimal_tables(data, "out", vec![8]);

    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let events = vec![Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    }];

    let accumulate_data: BTreeSet<DataId> = BTreeSet::new();
    let mut indexed: BTreeSet<DataId> = BTreeSet::new();
    indexed.insert(data);

    let result =
        collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &accumulate_data, &indexed);
    assert!(
        !result.contains(&data),
        "indexed-input data must be excluded; got: {result:?}"
    );
}

#[test]
fn empty_waits_yields_empty_set() {
    // No Wait events anywhere in `events` → `waited` is empty →
    // the per-data filter loop runs zero iterations → empty
    // output. Defensive identity on the no-multi-worker-rendezvous
    // shape.
    let data = DataId(17);
    let (_names, sidecar) = make_minimal_tables(data, "tmp", vec![4]);

    let events: Vec<Event> = vec![];
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let (acc, indexed) = empty_accumulate_and_indexed();

    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        result.is_empty(),
        "empty events → empty result; got: {result:?}"
    );
}

#[test]
fn shape_error_on_wait_slice_excludes_data() {
    // `wait_slice` (`wait.rs:269..278`) returns
    // `Err(ContractGap)` when the leading-axis range is out of
    // bounds for the data dims. `is_whole_array_recv`
    // (`wait.rs:381`) propagates the Err via `?`. The classifier's
    // inner loop (`collect.rs:392`) swallows the Err with
    // `.unwrap_or(false)` — so the data is classified "not whole"
    // and ends up in `not_all_whole` → excluded.
    //
    // Trigger: dims=[8], tile leading range 0..1024 → far past
    // leading-dim (8) → `wait_slice` Err. We choose 0..1024
    // (a constant > leading-dim) rather than a small overshoot
    // because it makes the test's intent obvious to a reader and
    // is robust to any future cycle that broadens the dim.
    //
    // Rejected approach: omitting the `data_types` entry would
    // instead trip `wait_slice:259-263` (`NameSidecar::data_type`
    // returns None → ContractGap). That ALSO propagates as Err →
    // same `.unwrap_or(false)` arm, so observationally equivalent
    // for THIS classifier; using out-of-bounds range pins the
    // dim-bounds guard specifically.
    let data = DataId(19);
    let seq = SeqTag(7);
    let (_names, sidecar) = make_minimal_tables(data, "buf", vec![8]);

    let mut pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    pair_tiles.insert((data, seq), tile_1d(0, 0..1024));

    let events = vec![Event::Wait {
        src: WorkerId(0),
        data,
        tile: tile_1d(0, 0..1024),
        seq,
    }];

    let (acc, indexed) = empty_accumulate_and_indexed();
    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        !result.contains(&data),
        "out-of-bounds tile → wait_slice Err → unwrap_or(false) → \
         excluded; got: {result:?}"
    );
}

#[test]
fn whole_array_wait_inside_event_loop_body_included() {
    // The classifier descends into `Event::Loop.body` via
    // `collect_let_at_wait_inner` (`collect.rs:399-401`). A
    // whole-array Wait buried inside an Event::Loop body still
    // reaches the `waited` set, never lands in `not_all_whole`,
    // and is included in the result.
    let data = DataId(23);
    let seq = SeqTag(11);
    let (_names, sidecar) = make_minimal_tables(data, "row", vec![16]);

    // No pair_tiles entry → whole-array.
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();

    let inner_wait = Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    };
    let outer_loop = Event::Loop {
        iter_var: IterVar(0),
        range: 0..4,
        body: vec![inner_wait],
        block_tag: None,
        check_frame: None,
    };
    let events = vec![outer_loop];

    let (acc, indexed) = empty_accumulate_and_indexed();
    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        result.contains(&data),
        "whole-array Wait nested in Event::Loop body must be \
         included; got: {result:?}"
    );
}

#[test]
fn scalar_data_no_dims_treated_as_whole_array() {
    // Scalar-typed data (`ty.dims.is_empty()`) hits the early
    // return at `wait.rs:265..267` → `Ok(None)` →
    // `is_whole_array_recv` returns `Ok(true)` → data stays out of
    // `not_all_whole` → included in the result.
    //
    // We construct a tile with a non-empty bounds vector to prove
    // the early-return precedes the leading-dim check: the empty-
    // `dims` arm fires BEFORE `ty.dims[0]` is read (which would
    // panic on a scalar if reached). Tile range is anything.
    let data = DataId(29);
    let seq = SeqTag(13);
    let (_names, sidecar) = make_minimal_tables(data, "n", vec![]); // scalar

    let mut pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    // Non-empty tile bounds — the scalar guard must short-circuit
    // before the dim-bounds check on this tile.
    pair_tiles.insert((data, seq), tile_1d(0, 0..1));

    let events = vec![Event::Wait {
        src: WorkerId(0),
        data,
        tile: tile_1d(0, 0..1),
        seq,
    }];

    let (acc, indexed) = empty_accumulate_and_indexed();
    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        result.contains(&data),
        "scalar (dims=[]) data must be classified whole-array → \
         included; got: {result:?}"
    );
}

#[test]
fn rank_3_tile_shape_excludes_data() {
    // `wait_slice`'s rank-3+ guard at `wait.rs:307` fires
    // `Err(ContractGap)` when either the tile or the data has rank
    // > 2 (the 2D row-loop slice-paste's supported shape).
    // `is_whole_array_recv` propagates the Err via `?`; the
    // classifier's `.unwrap_or(false)` at `collect.rs:392` swallows
    // it and the data lands in `not_all_whole` → excluded.
    //
    // Trigger: rank-3 tile (3 bounds entries) on rank-2 data
    // (`dims=[16, 16]`). The condition at wait.rs:307 is
    // `tile.bounds.len() > 2 || (tile.bounds.len() >= 2 &&
    // ty.dims.len() > 2)` — the left disjunct fires here.
    //
    // Sibling case (rank-3 data + rank-2 tile) is structurally
    // equivalent (right disjunct fires); we cover the left disjunct
    // here because 16-jacobi/distributed (cycle 209) is the
    // first in-tree schedule constructing a rank-3 tile, making
    // it the more load-bearing arm.
    let data = DataId(31);
    let seq = SeqTag(17);
    let (_names, sidecar) = make_minimal_tables(data, "field3d", vec![16, 16]);

    let tile = tile_3d(0, 0..2, 1, 0..16, 2, 0..16);
    let mut pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    pair_tiles.insert((data, seq), tile.clone());

    let events = vec![Event::Wait {
        src: WorkerId(0),
        data,
        tile,
        seq,
    }];

    let (acc, indexed) = empty_accumulate_and_indexed();
    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        !result.contains(&data),
        "rank-3 tile shape → wait_slice rank-3+ guard Err → \
         unwrap_or(false) → excluded; got: {result:?}"
    );
}

#[test]
fn inner_axis_oob_excludes_data() {
    // `wait_slice`'s inner-axis OOB guard at `wait.rs:324..333`
    // fires `Err(ContractGap)` when the tile's inner-axis range
    // is out of bounds for `ty.dims[1]`. Symmetric to test 5's
    // leading-axis OOB; that test 5 exercises wait.rs:269..278.
    //
    // Trigger: dims=[8, 8]; tile leading range 0..8 (full, in-
    // bounds) + tile inner range 0..1024 (far past inner-dim 8).
    // The leading-axis guard (wait.rs:269) passes; the inner-axis
    // guard (wait.rs:324) fires Err.
    //
    // Rejected approach: setting inner range start >= end would
    // ALSO trip the guard (degenerate-range arm) but obscures
    // which guard fires. The OOB form pins the dim-bounds check
    // specifically.
    let data = DataId(37);
    let seq = SeqTag(19);
    let (_names, sidecar) = make_minimal_tables(data, "mat", vec![8, 8]);

    let tile = tile_2d(0, 0..8, 1, 0..1024);
    let mut pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    pair_tiles.insert((data, seq), tile.clone());

    let events = vec![Event::Wait {
        src: WorkerId(0),
        data,
        tile,
        seq,
    }];

    let (acc, indexed) = empty_accumulate_and_indexed();
    let result = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        !result.contains(&data),
        "inner-axis OOB tile → wait_slice inner-axis guard Err → \
         unwrap_or(false) → excluded; got: {result:?}"
    );
}
