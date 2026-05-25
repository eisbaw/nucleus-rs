//! `collect_pair_tiles` — shared `(DataId, SeqTag) -> IterTile`
//! constructor used by all four tier-1 backends (TASK-0300 cycle 130
//! hoist from TASK-0296 cycle-116 architect P1.2).
//!
//! Before cycle 130 each backend (mp-tcp-bufsync, mp-tcp-event,
//! pthreads-async, pthreads-sync) inlined the same 4-line build:
//! initialise an empty `BTreeMap`, fold `collect_xfer_pairs` across
//! `per_worker.values()`. The risk this test pins down is that the
//! lifted helper preserves the original semantics so the four backends
//! stay byte-identical in their bit-identical e2e differential:
//!
//! 1. `empty_input_yields_empty_map` — defensive identity on the empty
//!    iterator (no workers projected yet).
//! 2. `single_push_wait_pair` — one Push/Wait pair across two workers
//!    yields exactly one entry; the tile is carried verbatim.
//! 3. `first_sighting_wins_on_conflicting_tiles` — if two workers emit
//!    differing tiles for the same `(DataId, SeqTag)` (would be a
//!    contract violation by XferPlaceholder construction (TASK-0018)
//!    but the helper must not silently overwrite), the FIRST sighting
//!    in `WorkerId`-ascending iteration order wins, mirroring the
//!    pre-cycle-130 inline behaviour.
//! 4. `push_nested_in_loop_is_collected` — the helper descends into
//!    `Event::Loop` bodies (via `collect_xfer_pairs`'s recursion), so
//!    a Push inside a loop body still appears in the output map.

use std::collections::BTreeMap;
use std::ops::Range;

use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};

use backend_common::multi_worker_walker::collect_pair_tiles;

fn tile_1d(iv: u64, range: Range<i64>) -> IterTile {
    IterTile::new(vec![(IterVar(iv), range)])
}

#[test]
fn empty_input_yields_empty_map() {
    let per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let pairs = collect_pair_tiles(per_worker.values());
    assert!(pairs.is_empty(), "empty input → empty pair_tiles");
}

#[test]
fn single_push_wait_pair() {
    let data = DataId(7);
    let seq = SeqTag(3);
    let tile = tile_1d(0, 0..8);

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(
        WorkerId(0),
        vec![Event::Push {
            dst: WorkerId(1),
            data,
            tile: tile.clone(),
            seq,
        }],
    );
    per_worker.insert(
        WorkerId(1),
        vec![Event::Wait {
            src: WorkerId(0),
            data,
            tile: tile.clone(),
            seq,
        }],
    );

    let pairs = collect_pair_tiles(per_worker.values());
    assert_eq!(pairs.len(), 1, "one Push/Wait pair → one map entry");
    assert_eq!(pairs.get(&(data, seq)).cloned(), Some(tile));
}

#[test]
fn first_sighting_wins_on_conflicting_tiles() {
    // Two workers project DIFFERENT IterTiles for the same (data, seq).
    // The XferPlaceholder invariant (TASK-0018) forbids this, but the
    // collector must not silently overwrite if it ever happens; it
    // keeps the FIRST sighting and drops later ones. `per_worker` is a
    // BTreeMap so `.values()` iterates in WorkerId-ascending order:
    // WorkerId(0)'s tile wins.
    let data = DataId(7);
    let seq = SeqTag(3);
    let tile_a = tile_1d(0, 0..8);
    let tile_b = tile_1d(0, 4..12); // would be a contract violation; pinned for honest behaviour

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(
        WorkerId(0),
        vec![Event::Push {
            dst: WorkerId(1),
            data,
            tile: tile_a.clone(),
            seq,
        }],
    );
    per_worker.insert(
        WorkerId(1),
        vec![Event::Wait {
            src: WorkerId(0),
            data,
            tile: tile_b,
            seq,
        }],
    );

    let pairs = collect_pair_tiles(per_worker.values());
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs.get(&(data, seq)).cloned(),
        Some(tile_a),
        "first sighting (WorkerId(0) Push) wins"
    );
}

#[test]
fn push_nested_in_loop_is_collected() {
    // A Push buried inside an Event::Loop body must still appear in
    // the output map (collect_xfer_pairs recurses into Loop bodies).
    let data = DataId(11);
    let seq = SeqTag(5);
    let tile = tile_1d(2, 0..16);

    let inner_push = Event::Push {
        dst: WorkerId(1),
        data,
        tile: tile.clone(),
        seq,
    };
    let outer_loop = Event::Loop {
        iter_var: IterVar(1),
        range: 0..4,
        body: vec![inner_push],
        block_tag: None,
        check_frame: None,
    };

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), vec![outer_loop]);

    let pairs = collect_pair_tiles(per_worker.values());
    assert_eq!(pairs.len(), 1, "Push nested in Loop body still collected");
    assert_eq!(pairs.get(&(data, seq)).cloned(), Some(tile));
}
