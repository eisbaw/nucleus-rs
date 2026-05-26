//! Overlapping-write accumulator fan-in emit shape pins (TASK-0343,
//! cycle 189).
//!
//! These tests pin the cycle-189 dispatch arm added to
//! [`backend_common::multi_worker_walker::render_wait_assign`] and the
//! sibling [`collect_accumulate_waits`] detector. They live in their
//! own file (not `wait_assign_slice.rs`) because the test surface is a
//! distinct cycle-189-introduced emit shape; sibling-adjacency would
//! dilute that file's "WaitSlice arm dispatch" framing.
//!
//! ## What this pins (AC#5 of TASK-0343)
//!
//! 1. `accumulate_emit_replaces_overwrite_for_array_fan_in` —
//!    end-to-end shape pin from a synthetic 4-Wait host event list to
//!    the emitted `wrapping_add` element-wise accumulate. Regression
//!    bites if the cycle-189 dispatch ever degenerates back to
//!    `name = rhs;` for the overlapping-write fan-in case. Pins the
//!    cycle-186 mismatch symptom semantically: the host's final
//!    `histogram` must reflect ALL 4 worker contributions, not the
//!    last worker's standalone partial.
//!
//! 2. `accumulate_detector_skips_single_wait` — the collector must
//!    NOT classify a single-Wait data as accumulate. A schedule
//!    where host receives one whole-array push (e.g. a non-fan-in
//!    transfer) must still emit the pre-cycle-189
//!    `name = rhs;` overwrite shape. Bites if the N>=2 guard is
//!    accidentally dropped.
//!
//! 3. `accumulate_detector_skips_disjoint_slice_paste` — the
//!    collector must NOT classify a fan-in whose tiles are
//!    slice-paste (the 03-reduction `partials[w]` shape). Each
//!    worker's Wait tile carries a per-worker slice; the existing
//!    `WaitSlice::Flat` arm handles them correctly. Bites if the
//!    "all whole-array" guard is accidentally relaxed to "any
//!    whole-array".
//!
//! 4. `accumulate_emit_float_returns_contract_gap` —
//!    `render_wait_assign` must refuse a float-scalar accumulate
//!    with a typed `EmitError::ContractGap` pointing to the
//!    TASK-0343 follow-up bucket. Bites if a future relaxation
//!    silently emits non-deterministic float `+` (which would
//!    collide with PRD §10.1 bit-identity).
//!
//! 5. `accumulate_emit_scalar_uses_wrapping_add_directly` — scalar
//!    (zero-dim) accumulator emits `name = name.wrapping_add(rhs);`
//!    without the element-wise loop. Defensive emit-identity pin so
//!    a scalar accumulator path is not silently mis-classified into
//!    the array form.
//!
//! ## What this does NOT pin
//!
//! - End-to-end e2e bit-identity of 08-histogram/distributed across
//!   the 4 tier-1 backends — that lives in
//!   `nucleus/e2e-matrix.toml` (promoted to `[[required]]` in
//!   cycle 189 as part of the same task) and is enforced by `just
//!   e2e`. The two layers (per-helper emit shape + end-to-end
//!   bit-identity) are independent regression footprints; both are
//!   required by AC#5.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use backend_common::multi_worker_walker::{
    collect_accumulate_waits, render_wait_assign, render_worker_events, RendezvousId, WalkerCtx,
};
use backend_common::render::EmitError;

type RendezvousIds = BTreeMap<(DataId, SeqTag), RendezvousId>;
type PairTiles = BTreeMap<(DataId, SeqTag), IterTile>;

/// Build a synthetic `(NameTables, NameSidecar)` for the 08-histogram
/// fan-in: 1 data symbol (`histogram`, `dims` of `i32`), 1 host
/// receiver (`WorkerId(0)`), 4 sender workers (`WorkerId(1..=4)`).
fn make_histogram_tables(
    data_id: DataId,
    dims: Vec<usize>,
    scalar: ScalarType,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.data.insert(data_id, "histogram".to_string());
    names.worker.insert(WorkerId(0), "host".to_string());
    names.worker.insert(WorkerId(1), "w0".to_string());
    names.worker.insert(WorkerId(2), "w1".to_string());
    names.worker.insert(WorkerId(3), "w2".to_string());
    names.worker.insert(WorkerId(4), "w3".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar
        .data_types
        .insert(data_id, ResolvedType { scalar, dims });
    (names, sidecar)
}

#[test]
fn accumulate_emit_replaces_overwrite_for_array_fan_in() {
    // Synthesise the host-side Event list for 08-histogram/distributed:
    // 4 Waits on `histogram`, all whole-array tiles. The collector must
    // classify all 4 (data, seq) as accumulate; the walker's Wait emit
    // arm must emit `wrapping_add` element-wise — NOT the pre-cycle-189
    // `name = rhs;` overwrite.
    let data = DataId(0);
    let (names, sidecar) = make_histogram_tables(data, vec![16], ScalarType::I32);

    let mut events: Vec<Event> = Vec::new();
    let mut ids: RendezvousIds = BTreeMap::new();
    let mut tiles: PairTiles = BTreeMap::new();
    for i in 0u64..4 {
        let seq = SeqTag(i);
        let src = WorkerId(i + 1);
        events.push(Event::Wait {
            src,
            data,
            tile: IterTile::empty(),
            seq,
        });
        ids.insert((data, seq), i as usize);
        tiles.insert((data, seq), IterTile::empty());
    }

    // Detector: classify all 4 as accumulate.
    let accumulate = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert_eq!(
        accumulate.len(),
        4,
        "collect_accumulate_waits MUST classify all 4 (data, seq) Waits \
         in the synthetic 08-histogram fan-in as accumulate (N=4 >= 2 + \
         all tiles whole-array); got {} entries: {:?}",
        accumulate.len(),
        accumulate
    );

    // Wire the per-(worker, data, seq) view the walker consumes.
    let mut walker_accumulate: BTreeSet<(WorkerId, DataId, SeqTag)> = BTreeSet::new();
    for (d, s) in &accumulate {
        walker_accumulate.insert((WorkerId(0), *d, *s));
    }

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "slot",
        rendezvous_ids: &ids,
        pair_tiles: &tiles,
        accumulate_waits: &walker_accumulate,
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &events, &mut out, 0, "")
        .expect("accumulate fan-in must render");

    // Each Wait must emit element-wise wrapping_add over LEN=16.
    for i in 0..4 {
        let expected = format!(
            "{{ let _tmp = slot_{i}.wait(); for _k in 0..16usize {{ \
             histogram[_k] = histogram[_k].wrapping_add(_tmp[_k]); }} }}"
        );
        assert!(
            out.contains(&expected),
            "TASK-0343 cycle 189: Wait #{i} must emit the element-wise \
             wrapping_add accumulate form; got:\n{out}"
        );
    }

    // Symptom pin: the pre-cycle-189 last-write-wins shape
    // `histogram = slot_N.wait();` MUST NOT appear for any Wait. If
    // this assertion bites, the dispatch has regressed to the
    // overwrite emit for whole-array Waits and 08-histogram/
    // distributed would silently produce one worker's standalone
    // partial again.
    for i in 0..4 {
        let regressed = format!("histogram = slot_{i}.wait();");
        assert!(
            !out.contains(&regressed),
            "TASK-0343 cycle 189 REGRESSION: pre-cycle-189 last-write-wins \
             overwrite `{regressed}` re-appeared for Wait #{i} (the cycle-186 \
             mismatch symptom shape); the accumulate dispatch was bypassed. \
             Got:\n{out}"
        );
    }
}

#[test]
fn accumulate_detector_skips_single_wait() {
    // A single Wait must NOT be classified as accumulate. The N>=2
    // guard is load-bearing for emit-identity preservation: a
    // single-Wait whole-array transfer (e.g. host's load_input
    // pushed to one worker) must still emit `name = rhs;`.
    let data = DataId(0);
    let (_, sidecar) = make_histogram_tables(data, vec![16], ScalarType::I32);

    let seq = SeqTag(0);
    let events = vec![Event::Wait {
        src: WorkerId(1),
        data,
        tile: IterTile::empty(),
        seq,
    }];
    let mut tiles: PairTiles = BTreeMap::new();
    tiles.insert((data, seq), IterTile::empty());

    let accumulate = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert!(
        accumulate.is_empty(),
        "N=1 single-Wait MUST NOT be classified as accumulate (the N>=2 \
         guard preserves pre-cycle-189 emit identity for non-fan-in \
         single-pair transfers); got {accumulate:?}"
    );
}

#[test]
fn accumulate_detector_skips_disjoint_slice_paste() {
    // 03-reduction/distributed shape: 4 Waits on `partials`, each
    // with a per-worker slice tile (`partials[w]`). The existing
    // WaitSlice::Flat arm handles them correctly; the accumulate
    // dispatch MUST NOT fire on slice-paste tiles (every Wait's
    // tile is partial, not whole-array).
    let data = DataId(0);
    let (_, sidecar) = make_histogram_tables(data, vec![4], ScalarType::I32);

    let iv = IterVar(0);
    let mut events: Vec<Event> = Vec::new();
    let mut tiles: PairTiles = BTreeMap::new();
    for i in 0u64..4 {
        let seq = SeqTag(i);
        let src = WorkerId(i + 1);
        // Per-worker partial slice — partials[w] where w is the
        // partition variable. Tile is `[(iv, w..w+1)]`, NOT
        // whole-array.
        let tile = IterTile::new(vec![(iv, (i as i64)..(i as i64 + 1))]);
        events.push(Event::Wait {
            src,
            data,
            tile: tile.clone(),
            seq,
        });
        tiles.insert((data, seq), tile);
    }

    let accumulate = collect_accumulate_waits(&events, &sidecar, &tiles);
    assert!(
        accumulate.is_empty(),
        "disjoint slice-paste fan-in (03-reduction `partials[w]` shape) \
         MUST NOT classify as accumulate — every Wait's tile is partial. \
         A relaxation that fires here would silently mis-combine \
         disjoint-write slice gathers as element-wise sum. Got: {accumulate:?}"
    );
}

#[test]
fn accumulate_emit_float_returns_contract_gap() {
    // Float-scalar accumulator: typed EmitError::ContractGap pointing
    // to the TASK-0343 follow-up bucket. Sum identity for floats
    // collides with PRD §10.1 bit-identity (sum order is not
    // associative-stable).
    let data = DataId(0);
    let (_, sidecar) = make_histogram_tables(data, vec![16], ScalarType::F32);

    let seq = SeqTag(0);
    let mut tiles: PairTiles = BTreeMap::new();
    tiles.insert((data, seq), IterTile::empty());

    let err = render_wait_assign(
        &sidecar,
        &tiles,
        "histogram",
        data,
        seq,
        "slot_0.wait()",
        true, // accumulate
    )
    .expect_err("float accumulate MUST return ContractGap");

    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("float") && msg.contains("TASK-0343"),
                "float-accumulate ContractGap must name the scalar class \
                 AND the follow-up bucket (TASK-0343); got: {msg}"
            );
        }
        other => panic!("expected ContractGap; got: {other:?}"),
    }
}

#[test]
fn accumulate_emit_scalar_uses_wrapping_add_directly() {
    // Scalar (zero-dim) accumulator — emit `name = name.wrapping_add(rhs);`
    // without the element-wise loop. Defensive pin so a future
    // refactor doesn't accidentally route a scalar through the array
    // form (which would emit `for _k in 0..1usize { ... }` — works,
    // but is dead-loop noise).
    let data = DataId(0);
    let (_, sidecar) = make_histogram_tables(data, vec![], ScalarType::I32);

    let seq = SeqTag(0);
    let mut tiles: PairTiles = BTreeMap::new();
    tiles.insert((data, seq), IterTile::empty());

    let out = render_wait_assign(
        &sidecar,
        &tiles,
        "histogram",
        data,
        seq,
        "slot_0.wait()",
        true,
    )
    .expect("scalar accumulate must render");

    assert_eq!(
        out, "histogram = histogram.wrapping_add(slot_0.wait());",
        "scalar accumulator must emit the direct `name = \
         name.wrapping_add(rhs);` shape (no element-wise loop); \
         got: {out}"
    );
}
