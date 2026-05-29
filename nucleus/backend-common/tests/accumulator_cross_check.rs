//! Algorithm-level cross-check for the structural overlapping-write
//! accumulator detector (TASK-0343.03; hardens the cycle-189 structural
//! landing TASK-0343).
//!
//! [`backend_common::multi_worker_walker::collect_accumulate_waits`]
//! classifies the overlapping-write accumulator fan-in PURELY
//! STRUCTURALLY (per worker, `N>=2` whole-array `Wait`s on one data ⇒
//! element-wise sum combine). For every shipped schedule that structural
//! shape coincides with the algorithm-level accumulator shape
//! (LHS-appears-in-RHS, e.g. 08-histogram's `histogram[b] <-- bin_inc(
//! histogram[b], ...)`). An exotic schedule that emits multiple
//! whole-array pushes for NON-accumulator semantics would be silently
//! mis-combined as a sum — a silent miscompile.
//!
//! [`backend_common::multi_worker_walker::check_accumulator_consistency`]
//! closes that gap: it consults the algorithm-IR and FAILS LOUD
//! ([`EmitError::AccumulatorShapeMismatch`]) when the structural
//! accumulate pattern fires on a data symbol whose algorithm-level shape
//! is NOT an accumulator.
//!
//! ## Why these are HAND-BUILT unit inputs, not a real `.sched.nuc`
//!
//! The reject path requires a data symbol that is BOTH (a) structurally
//! classified as accumulate (`>=2` whole-array Waits on one worker) AND
//! (b) NOT an algorithm-level accumulator (LHS not in RHS). The parent
//! task (TASK-0343) records "no obvious [real schedule] example today":
//! for every constructible shipped schedule the structural pattern only
//! arises for genuine accumulators (the LHS-in-RHS shape is exactly what
//! makes every worker push the FULL output array). Driving the reject
//! through a real `nucleus build` would require synthesising a
//! `.algo.nuc` + `.sched.nuc` whose `transfer_inject` produces `>=2`
//! whole-array Waits for a non-accumulator — which would either be
//! impractical or risk perturbing the locked e2e baseline
//! (308/246/0/62/0). So the reject path is exercised here with
//! hand-built inputs to `check_accumulator_consistency` directly: a
//! synthetic non-accumulator `AlgoIR` + a `per_worker` map with `>=2`
//! whole-array Waits on that data + the matching sidecar / name table.
//! This is the form TASK-0343.03's AC endorses ("fall back to a UNIT
//! TEST on `check_accumulator_consistency` with HAND-BUILT inputs").
//!
//! The POSITIVE case (08-histogram shape — LHS-in-RHS, `>=2` whole-array
//! Waits) IS exercised end-to-end by the e2e matrix (08-histogram/
//! distributed is `[[required]]` across all 4 tier-1 backends), so the
//! `accumulator_ok_for_lhs_in_rhs_accumulator` test below is a fast
//! unit-level guard that the cross-check does not OVER-reject the real
//! shipped shape.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, WorkerId};

use backend_common::multi_worker_walker::check_accumulator_consistency;
use backend_common::render::EmitError;

mod common;

/// `data_name <-- callee(args...)` dataflow statement (LHS unindexed for
/// simplicity — the cross-check tests on the LHS NAME, not its indices).
fn dataflow(lhs_name: &str, callee: &str, args: Vec<IrExpr>) -> IrStmt {
    IrStmt::Dataflow {
        lhs: IndexedRef {
            name: lhs_name.to_string(),
            indices: vec![],
        },
        rhs: IrExpr::Call {
            callee: callee.to_string(),
            args,
        },
    }
}

/// `DataRef(name)` — an unindexed read of a data symbol.
fn data_ref(name: &str) -> IrExpr {
    IrExpr::DataRef(IndexedRef {
        name: name.to_string(),
        indices: vec![],
    })
}

/// Host event list with `n` whole-array `Wait`s on `data` (the
/// structural accumulator fan-in shape — each Wait carries an empty
/// tile, i.e. whole-array).
fn host_with_n_whole_array_waits(data: DataId, n: u64) -> Vec<Event> {
    (0..n)
        .map(|i| Event::Wait {
            src: WorkerId(i + 1),
            data,
            tile: IterTile::empty(),
            seq: SeqTag(i),
        })
        .collect()
}

#[test]
fn accumulator_rejects_structural_fan_in_on_non_accumulator() {
    // THE NEGATIVE TEST (AC core): structural detector fires (4
    // whole-array Waits on `out`) but the algorithm-IR shows `out` is
    // NOT an accumulator (`out <-- compute(input)` — LHS `out` does NOT
    // appear in the RHS). The cross-check MUST reject loudly.
    let data = DataId(0);
    let (names, sidecar) = common::Tables::new()
        .with_data_typed(data, "out", ScalarType::I32, vec![16])
        .with_worker(WorkerId(0), "host")
        .with_worker(WorkerId(1), "w0")
        .with_worker(WorkerId(2), "w1")
        .with_worker(WorkerId(3), "w2")
        .with_worker(WorkerId(4), "w3")
        .build();

    // Algorithm: `out <-- compute(input)`. NON-accumulator: `out` is
    // not read on the RHS.
    let mut algo = AlgoIR::default();
    algo.stmts
        .push(dataflow("out", "compute", vec![data_ref("input")]));

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), host_with_n_whole_array_waits(data, 4));

    let err = check_accumulator_consistency(&algo, &per_worker, &sidecar, &names.data).expect_err(
        "structural accumulate fan-in on a NON-accumulator data symbol MUST be rejected; \
             element-wise summing 4 independently-computed full arrays would be a silent \
             miscompile",
    );

    match err {
        EmitError::AccumulatorShapeMismatch(msg) => {
            assert!(
                msg.contains("out"),
                "AccumulatorShapeMismatch must NAME the offending data symbol `out`; got: {msg}"
            );
            assert!(
                msg.contains("TASK-0343.03"),
                "AccumulatorShapeMismatch must reference the tightening task TASK-0343.03 so the \
                 reject is traceable; got: {msg}"
            );
        }
        other => panic!("expected EmitError::AccumulatorShapeMismatch; got: {other:?}"),
    }
}

#[test]
fn accumulator_ok_for_lhs_in_rhs_accumulator() {
    // THE POSITIVE TEST: 08-histogram shape — `histogram <-- bin_inc(
    // histogram, input)`. LHS `histogram` DOES appear in the RHS ⇒
    // algorithm-level accumulator. Structural detector fires (4
    // whole-array Waits) AND the algorithm agrees ⇒ the cross-check
    // must NOT reject. Guards against over-rejection of the real
    // shipped 08-histogram/distributed shape.
    let data = DataId(0);
    let (names, sidecar) = common::Tables::new()
        .with_data_typed(data, "histogram", ScalarType::I32, vec![16])
        .with_worker(WorkerId(0), "host")
        .with_worker(WorkerId(1), "w0")
        .with_worker(WorkerId(2), "w1")
        .with_worker(WorkerId(3), "w2")
        .with_worker(WorkerId(4), "w3")
        .build();

    // `histogram <-- bin_inc(histogram, input)`: LHS-appears-in-RHS.
    let mut algo = AlgoIR::default();
    algo.stmts.push(dataflow(
        "histogram",
        "bin_inc",
        vec![data_ref("histogram"), data_ref("input")],
    ));

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), host_with_n_whole_array_waits(data, 4));

    check_accumulator_consistency(&algo, &per_worker, &sidecar, &names.data).expect(
        "the LHS-in-RHS accumulator (08-histogram shape) MUST be accepted — the cross-check \
         must not over-reject the real shipped shape",
    );
}

#[test]
fn accumulator_ok_when_structural_pattern_does_not_fire() {
    // No structural accumulate pattern: a SINGLE whole-array Wait (N=1
    // < 2). Even though `out` is a non-accumulator, the structural
    // detector does not fire, so there is nothing to cross-check and the
    // gate is a no-op. Pins that the cross-check only inspects what the
    // detector actually classified (it does not independently re-derive
    // an opinion on data the detector left alone).
    let data = DataId(0);
    let (names, sidecar) = common::Tables::new()
        .with_data_typed(data, "out", ScalarType::I32, vec![16])
        .with_worker(WorkerId(0), "host")
        .with_worker(WorkerId(1), "w0")
        .build();

    let mut algo = AlgoIR::default();
    algo.stmts
        .push(dataflow("out", "compute", vec![data_ref("input")]));

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), host_with_n_whole_array_waits(data, 1));

    check_accumulator_consistency(&algo, &per_worker, &sidecar, &names.data).expect(
        "a single whole-array Wait does not trigger the structural detector, so the cross-check \
         must be a no-op even on a non-accumulator data symbol",
    );
}

#[test]
fn accumulator_rejects_non_accumulator_detected_inside_loop_statement() {
    // The algorithm-accumulator walk must descend into `IrStmt::For`
    // bodies. Here the only `<--` for `out` is inside a `for` loop and
    // is a non-accumulator. (Note: the STRUCTURAL detector's whole-array
    // predicate naturally excludes in-Loop *Waits*, but the host's
    // Waits here are top-level — this test exercises the algorithm-side
    // For-body descent, not the event-side loop handling.)
    let data = DataId(0);
    let (names, sidecar) = common::Tables::new()
        .with_data_typed(data, "out", ScalarType::I32, vec![16])
        .with_worker(WorkerId(0), "host")
        .with_worker(WorkerId(1), "w0")
        .with_worker(WorkerId(2), "w1")
        .build();

    let mut algo = AlgoIR::default();
    algo.stmts.push(IrStmt::For {
        var: "i".to_string(),
        lo: IrExpr::IntLit(0),
        hi: IrExpr::IntLit(4),
        body: vec![dataflow("out", "compute", vec![data_ref("input")])],
    });

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), host_with_n_whole_array_waits(data, 2));

    let err = check_accumulator_consistency(&algo, &per_worker, &sidecar, &names.data)
        .expect_err("non-accumulator `out` written inside a `for` body must still be rejected");

    assert!(
        matches!(err, EmitError::AccumulatorShapeMismatch(_)),
        "expected AccumulatorShapeMismatch from the For-body-descended non-accumulator; got: {err:?}"
    );
}

#[test]
fn accumulator_contract_gap_when_dataid_missing_from_name_table() {
    // Defensive: the structural detector classified a DataId that has no
    // entry in the DataId->name table. This is a contract regression
    // (NameTables::data should map every projected DataId), so the
    // cross-check must surface a typed ContractGap rather than silently
    // skip (CLAUDE.md no-workarounds). We build the sidecar WITH the
    // data type (so the structural detector classifies it) but OMIT the
    // `names.data` entry.
    let data = DataId(0);
    // sidecar has the data type; name table deliberately empty for it.
    let (_names, sidecar) = common::Tables::new()
        .with_data_typed(data, "out", ScalarType::I32, vec![16])
        .with_worker(WorkerId(0), "host")
        .build();
    let empty_names: BTreeMap<DataId, String> = BTreeMap::new();

    // Algorithm is irrelevant here — the ContractGap fires before the
    // accumulator-name lookup. Use an empty program.
    let algo = AlgoIR::default();

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), host_with_n_whole_array_waits(data, 3));

    let err = check_accumulator_consistency(&algo, &per_worker, &sidecar, &empty_names).expect_err(
        "a structurally-classified DataId missing from the name table is a contract gap",
    );

    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("NameTables::data"),
                "ContractGap must name the missing table (NameTables::data); got: {msg}"
            );
        }
        other => panic!("expected EmitError::ContractGap on the missing-name path; got: {other:?}"),
    }
}

#[test]
fn accumulator_ok_for_disjoint_slice_paste_non_accumulator() {
    // 03-reduction `partials[w]` shape: N>=2 Waits on one data, but each
    // Wait's tile is a per-worker SLICE (not whole-array). The
    // structural detector does NOT classify these (whole-array predicate
    // fails), so the cross-check has nothing to inspect and must accept
    // — even though `partials <-- accumulate(input)` is a non-accumulator
    // here. This pins that the cross-check inherits the detector's
    // slice-vs-whole-array discrimination (it does not independently
    // flag disjoint fan-ins).
    let data = DataId(0);
    let (names, sidecar) = common::Tables::new()
        .with_data_typed(data, "partials", ScalarType::I32, vec![4])
        .with_worker(WorkerId(0), "host")
        .with_worker(WorkerId(1), "w0")
        .with_worker(WorkerId(2), "w1")
        .with_worker(WorkerId(3), "w2")
        .with_worker(WorkerId(4), "w3")
        .build();

    let iv = IterVar(0);
    let host_events: Vec<Event> = (0u64..4)
        .map(|i| Event::Wait {
            src: WorkerId(i + 1),
            data,
            // Per-worker partial slice — NOT whole-array.
            tile: IterTile::new(vec![(iv, (i as i64)..(i as i64 + 1))]),
            seq: SeqTag(i),
        })
        .collect();

    let mut algo = AlgoIR::default();
    algo.stmts
        .push(dataflow("partials", "accumulate", vec![data_ref("input")]));

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), host_events);

    check_accumulator_consistency(&algo, &per_worker, &sidecar, &names.data).expect(
        "disjoint slice-paste fan-in must NOT trip the cross-check — the structural detector \
         does not classify slice tiles as accumulate, so there is nothing to reject",
    );
}
