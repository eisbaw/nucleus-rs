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

use nucleus_compiler::algo::{CombineOp, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use backend_common::multi_worker_walker::{
    collect_accumulate_waits, render_wait_assign, render_worker_events, RendezvousId, WalkerCtx,
};
use backend_common::render::EmitError;

mod common;

type RendezvousIds = BTreeMap<(DataId, SeqTag), RendezvousId>;
type PairTiles = BTreeMap<(DataId, SeqTag), IterTile>;

/// Build a synthetic `(NameTables, NameSidecar)` for the 08-histogram
/// fan-in: 1 data symbol (`histogram`, `dims` of `scalar`), 1 host
/// receiver (`WorkerId(0)`), 4 sender workers (`WorkerId(1..=4)`).
///
/// Construction logic lives in the shared `common::Tables` builder
/// (TASK-0358); this thin file-local adapter names the histogram
/// fan-in fixture shape (one typed data symbol + the host/w0..w3
/// worker layout).
fn make_histogram_tables(
    data_id: DataId,
    dims: Vec<usize>,
    scalar: ScalarType,
) -> (NameTables, NameSidecar) {
    make_histogram_tables_combine(data_id, dims, scalar, CombineOp::Sum)
}

/// As [`make_histogram_tables`] but with an explicit combine identity
/// (TASK-0343.01.01) — the accumulate render path now resolves the op
/// from `sidecar.combine_for_data`, so a fan-in fixture must declare it.
fn make_histogram_tables_combine(
    data_id: DataId,
    dims: Vec<usize>,
    scalar: ScalarType,
    op: CombineOp,
) -> (NameTables, NameSidecar) {
    common::Tables::new()
        .with_data_typed(data_id, "histogram", scalar, dims)
        .with_combine_for_data(data_id, op)
        .with_worker(WorkerId(0), "host")
        .with_worker(WorkerId(1), "w0")
        .with_worker(WorkerId(2), "w1")
        .with_worker(WorkerId(3), "w2")
        .with_worker(WorkerId(4), "w3")
        .build()
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
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
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
        WalkerCtx::empty_let_at_wait_set(),
    )
    .expect_err("float accumulate MUST return ContractGap");

    match err {
        EmitError::ContractGap(msg) => {
            // TASK-0343.02 AC#4: the float-SUM reject must BITE with the
            // order-dependence / PRD §10.1 message (not a generic
            // "floats unsupported"). `make_histogram_tables` defaults to
            // CombineOp::Sum, so this exercises the non-associative arm.
            assert!(
                msg.contains("float")
                    && msg.contains("associative")
                    && msg.contains("PRD §10.1"),
                "float-SUM ContractGap must name the scalar class, the \
                 non-associativity, AND PRD §10.1 bit-identity; got: {msg}"
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
        WalkerCtx::empty_let_at_wait_set(),
    )
    .expect("scalar accumulate must render");

    assert_eq!(
        out, "histogram = histogram.wrapping_add(slot_0.wait());",
        "scalar accumulator must emit the direct `name = \
         name.wrapping_add(rhs);` shape (no element-wise loop); \
         got: {out}"
    );
}

/// Render one whole-array accumulate Wait for a given combine op and
/// return the emitted statement. Helper for the AC#3 per-op emit pins.
fn render_array_accumulate(op: CombineOp) -> String {
    let data = DataId(0);
    let (_, sidecar) = make_histogram_tables_combine(data, vec![16], ScalarType::I32, op);
    let seq = SeqTag(0);
    let mut tiles: PairTiles = BTreeMap::new();
    tiles.insert((data, seq), IterTile::empty());
    render_wait_assign(
        &sidecar,
        &tiles,
        "histogram",
        data,
        seq,
        "slot_0.wait()",
        true,
        WalkerCtx::empty_let_at_wait_set(),
    )
    .expect("accumulate must render")
}

#[test]
fn accumulate_emit_array_op_strings_per_combine_op() {
    // TASK-0343.01.01 AC#3: assert the exact emitted string for EACH
    // zero-identity combine op. `sum` is the method form
    // (`.wrapping_add(...)`); `or`/`xor` are the operator forms
    // (`|`/`^`) — a structurally distinct emit arm. If a future refactor
    // collapses the operator arm back to a method call (or swaps the
    // operator), one of these bites.
    assert_eq!(
        render_array_accumulate(CombineOp::Sum),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k].wrapping_add(_tmp[_k]); } }",
        "sum must emit the `.wrapping_add(...)` METHOD form"
    );
    assert_eq!(
        render_array_accumulate(CombineOp::Or),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k] | _tmp[_k]; } }",
        "or must emit the bitwise `|` OPERATOR form"
    );
    assert_eq!(
        render_array_accumulate(CombineOp::Xor),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k] ^ _tmp[_k]; } }",
        "xor must emit the bitwise `^` OPERATOR form"
    );
    // TASK-0343.01.02 non-zero-identity ops. min/max are the `.min`/
    // `.max` METHOD forms; `and` is the bitwise `&` OPERATOR form.
    assert_eq!(
        render_array_accumulate(CombineOp::Min),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k].min(_tmp[_k]); } }",
        "min must emit the `.min(...)` METHOD form"
    );
    assert_eq!(
        render_array_accumulate(CombineOp::Max),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k].max(_tmp[_k]); } }",
        "max must emit the `.max(...)` METHOD form"
    );
    assert_eq!(
        render_array_accumulate(CombineOp::And),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k] & _tmp[_k]; } }",
        "and must emit the bitwise `&` OPERATOR form"
    );
}

#[test]
fn accumulate_emit_scalar_op_strings_per_combine_op() {
    // AC#3 scalar arm: the zero-dim accumulator emit for each op.
    let render_scalar = |op: CombineOp| -> String {
        let data = DataId(0);
        let (_, sidecar) = make_histogram_tables_combine(data, vec![], ScalarType::I32, op);
        let seq = SeqTag(0);
        let mut tiles: PairTiles = BTreeMap::new();
        tiles.insert((data, seq), IterTile::empty());
        render_wait_assign(
            &sidecar,
            &tiles,
            "histogram",
            data,
            seq,
            "slot_0.wait()",
            true,
            WalkerCtx::empty_let_at_wait_set(),
        )
        .expect("scalar accumulate must render")
    };
    assert_eq!(
        render_scalar(CombineOp::Sum),
        "histogram = histogram.wrapping_add(slot_0.wait());"
    );
    assert_eq!(
        render_scalar(CombineOp::Or),
        "histogram = histogram | slot_0.wait();"
    );
    assert_eq!(
        render_scalar(CombineOp::Xor),
        "histogram = histogram ^ slot_0.wait();"
    );
    // TASK-0343.01.02 non-zero-identity scalar arms.
    assert_eq!(
        render_scalar(CombineOp::Min),
        "histogram = histogram.min(slot_0.wait());"
    );
    assert_eq!(
        render_scalar(CombineOp::Max),
        "histogram = histogram.max(slot_0.wait());"
    );
    assert_eq!(
        render_scalar(CombineOp::And),
        "histogram = histogram & slot_0.wait();"
    );
}

#[test]
fn accumulate_emit_no_combine_declared_returns_contract_gap() {
    // TASK-0343.01.01 AC#4 (render-path defence in depth): an accumulate
    // Wait on a data symbol with NO `combine_for_data` entry must fail
    // loud rather than silently assume sum. The driver gate catches this
    // earlier, but the render path stays fail-loud as belt-and-braces.
    let data = DataId(0);
    // Build WITHOUT a combine identity (plain Tables, no
    // with_combine_for_data).
    let (_, sidecar) = common::Tables::new()
        .with_data_typed(data, "histogram", ScalarType::I32, vec![16])
        .with_worker(WorkerId(0), "host")
        .build();
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
        true,
        WalkerCtx::empty_let_at_wait_set(),
    )
    .expect_err("accumulate with no declared combine identity MUST return ContractGap");

    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("combine") && msg.contains("sum|or|xor|min|max|and"),
                "no-combine ContractGap must mention the missing `combine` identity \
                 AND list the accepted ops; got: {msg}"
            );
        }
        other => panic!("expected ContractGap; got: {other:?}"),
    }
}

// ---------------------------------------------------------------------
// TASK-0343.02 — float / bool combine admit-reject matrix.
//
// `combine_form_for_scalar` is private to the wait module; these tests
// drive it through `render_wait_assign` (the same public surface the
// pre-existing AC#3 emit pins use). The admissibility predicate is
// ORDER-INDEPENDENCE (associative + commutative) on the scalar type —
// PRD §10.1 bit-identity requires the host fan-in to be reduction-order
// independent. Integer: all six ops. Float: min/max only. Bool:
// and/or/xor only.
// ---------------------------------------------------------------------

/// Render one whole-array accumulate Wait for `(scalar, op)` and return
/// the `Result` (Ok emit string or the typed reject). The scalar-typed
/// generalisation of `render_array_accumulate`.
fn render_array_accumulate_typed(
    scalar: &ScalarType,
    op: CombineOp,
) -> Result<String, EmitError> {
    let data = DataId(0);
    let (_, sidecar) = make_histogram_tables_combine(data, vec![16], scalar.clone(), op);
    let seq = SeqTag(0);
    let mut tiles: PairTiles = BTreeMap::new();
    tiles.insert((data, seq), IterTile::empty());
    render_wait_assign(
        &sidecar,
        &tiles,
        "histogram",
        data,
        seq,
        "slot_0.wait()",
        true,
        WalkerCtx::empty_let_at_wait_set(),
    )
}

#[test]
fn float_admits_min_max_method_form() {
    // f32 / f64 min/max are order-independent for distinct finite
    // non-NaN values → ADMITTED, emitted as the `.min`/`.max` METHOD
    // form (Rust `f32::min` / `f32::max`).
    for scalar in [&ScalarType::F32, &ScalarType::F64] {
        assert_eq!(
            render_array_accumulate_typed(scalar, CombineOp::Min).unwrap(),
            "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
             histogram[_k] = histogram[_k].min(_tmp[_k]); } }",
            "{scalar:?} min must emit the `.min(...)` METHOD form"
        );
        assert_eq!(
            render_array_accumulate_typed(scalar, CombineOp::Max).unwrap(),
            "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
             histogram[_k] = histogram[_k].max(_tmp[_k]); } }",
            "{scalar:?} max must emit the `.max(...)` METHOD form"
        );
    }
}

#[test]
fn float_rejects_sum_with_non_associativity_message() {
    // float SUM is non-associative → REJECT citing PRD §10.1. The
    // reject must BITE (AC#4) for BOTH f32 and f64.
    for scalar in [&ScalarType::F32, &ScalarType::F64] {
        let err = render_array_accumulate_typed(scalar, CombineOp::Sum)
            .expect_err("float SUM accumulate MUST reject");
        match err {
            EmitError::ContractGap(msg) => assert!(
                msg.contains("associative") && msg.contains("PRD §10.1"),
                "{scalar:?} SUM reject must cite non-associativity + PRD §10.1; got: {msg}"
            ),
            other => panic!("expected ContractGap; got: {other:?}"),
        }
    }
}

#[test]
fn float_rejects_bitwise_ops() {
    // or / xor / and are undefined on float → REJECT.
    for scalar in [&ScalarType::F32, &ScalarType::F64] {
        for op in [CombineOp::Or, CombineOp::Xor, CombineOp::And] {
            let err = render_array_accumulate_typed(scalar, op)
                .unwrap_err_or_else_msg(scalar, op);
            assert!(
                err.contains("bitwise") && err.contains("undefined on float"),
                "{scalar:?} {op:?} reject must say bitwise undefined on float; got: {err}"
            );
        }
    }
}

#[test]
fn bool_admits_and_or_xor_operator_form() {
    // bool and/or/xor are associative + commutative → ADMITTED as the
    // `&`/`|`/`^` OPERATOR form.
    assert_eq!(
        render_array_accumulate_typed(&ScalarType::Bool, CombineOp::And).unwrap(),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k] & _tmp[_k]; } }",
        "bool and must emit the `&` OPERATOR form"
    );
    assert_eq!(
        render_array_accumulate_typed(&ScalarType::Bool, CombineOp::Or).unwrap(),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k] | _tmp[_k]; } }",
        "bool or must emit the `|` OPERATOR form"
    );
    assert_eq!(
        render_array_accumulate_typed(&ScalarType::Bool, CombineOp::Xor).unwrap(),
        "{ let _tmp = slot_0.wait(); for _k in 0..16usize { \
         histogram[_k] = histogram[_k] ^ _tmp[_k]; } }",
        "bool xor must emit the `^` OPERATOR form"
    );
}

#[test]
fn bool_rejects_sum_and_minmax() {
    // bool sum has no canonical identity → REJECT; min/max are
    // ambiguous on bool → REJECT (steer to and/or).
    let err_sum = match render_array_accumulate_typed(&ScalarType::Bool, CombineOp::Sum) {
        Err(EmitError::ContractGap(m)) => m,
        other => panic!("bool SUM must reject ContractGap; got: {other:?}"),
    };
    assert!(
        err_sum.contains("no canonical sum"),
        "bool SUM reject must say no canonical sum; got: {err_sum}"
    );
    for op in [CombineOp::Min, CombineOp::Max] {
        let err = match render_array_accumulate_typed(&ScalarType::Bool, op) {
            Err(EmitError::ContractGap(m)) => m,
            other => panic!("bool {op:?} must reject ContractGap; got: {other:?}"),
        };
        assert!(
            err.contains("combine=and") && err.contains("combine=or"),
            "bool {op:?} reject must steer to and/or; got: {err}"
        );
    }
}

#[test]
fn integer_matrix_unchanged_all_six_ops_admit() {
    // Regression guard: the integer path must keep admitting ALL six
    // ops (TASK-0343.02 must not narrow the integer surface).
    for op in [
        CombineOp::Sum,
        CombineOp::Or,
        CombineOp::Xor,
        CombineOp::Min,
        CombineOp::Max,
        CombineOp::And,
    ] {
        assert!(
            render_array_accumulate_typed(&ScalarType::I32, op).is_ok(),
            "integer {op:?} must still admit"
        );
        assert!(
            render_array_accumulate_typed(&ScalarType::U64, op).is_ok(),
            "unsigned {op:?} must still admit"
        );
    }
}

/// Tiny test-local extension: unwrap the ContractGap message or panic
/// with the `(scalar, op)` context. Keeps the reject-matrix asserts
/// terse without leaking a `match` into every call site.
trait UnwrapErrMsg {
    fn unwrap_err_or_else_msg(self, scalar: &ScalarType, op: CombineOp) -> String;
}

impl UnwrapErrMsg for Result<String, EmitError> {
    fn unwrap_err_or_else_msg(self, scalar: &ScalarType, op: CombineOp) -> String {
        match self {
            Err(EmitError::ContractGap(m)) => m,
            Ok(s) => panic!("{scalar:?} {op:?} expected reject, admitted: {s}"),
            Err(other) => panic!("{scalar:?} {op:?} expected ContractGap; got: {other:?}"),
        }
    }
}
