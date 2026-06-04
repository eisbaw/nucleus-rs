//! TASK-0049.10.05 (BLOCKER 3 slice C2) — tests for the deterministic
//! multi-worker OUTPUT capture: each SAVER worker gets a DISTINCT capture
//! file, and the per-saver capture ORDER is sorted by
//! `NameSidecar::data_decl_order` (NOT alphabetical `DataId`) so the
//! captured bytes concatenate in `reference.bin` order.
//!
//! The load-bearing property (mirror of the INPUT side, `input_offsets.rs`):
//! the reference output layout is DECLARATION order
//! (ex14 `spk_out`@0 ++ `bt_out`@256; `spk_out` declared line 80 before
//! `bt_out` line 81), but `DataId` is assigned ALPHABETICALLY by
//! `acfg::build` — `bt_out`=DataId(1) < `spk_out`=DataId(4), the REVERSE. A
//! per-saver order derived from `DataId` would be backwards; these tests
//! construct exactly that REVERSED-DataId shape, assert the capture order
//! follows declaration order, then FLIP the decl-order signal and assert the
//! order flips with it — proving the computation bites on `data_decl_order`.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, DataId, DataSlice, Event, FireBinding, IterTile, KernelId, WorkerId,
};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::multimcu::{render_multimachine_resc, TransportPlan};

// ex14-shaped OUTPUT DataIds: assigned ALPHABETICALLY, so bt_out < spk_out
// (mirrors the real ex14 acfg assignment: bt_out=DataId(1), spk_out=DataId(4)).
const BT_OUT: DataId = DataId(1); // "bt_out"  -> drained by rf_transmit
const SPK_OUT: DataId = DataId(4); // "spk_out" -> drained by fe_emit

const FE: WorkerId = WorkerId(1); // saves spk_out (fe_emit)
const RF: WorkerId = WorkerId(2); // saves bt_out (rf_transmit)
const HOST: WorkerId = WorkerId(0); // 02-split-add single saver

/// An effectful-SAVE `Fire` (output-less) draining `data[idx]` — the shape
/// of `fe_emit(spk_out[frame])` / `rf_transmit(bt_out[frame])`. The drained
/// datum is the first `ArgBinding::Data` input.
fn save(data: DataId) -> Event {
    Event::fire(
        KernelId(9),
        IterTile::empty(),
        FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                // A concrete per-frame index; classification is by
                // output==None, so the index value is irrelevant here.
                indices: vec![IrExpr::IntLit(0)],
            })],
            output: None,
        },
    )
}

/// A sidecar carrying ex14-shaped i32[4][16] types for spk_out + bt_out with
/// the given declaration order. (Types/alloc_len are not consulted by the
/// output-capture ORDER computation — only `data_decl_order` is — but are
/// populated for fidelity to the real contract.)
fn sidecar_with_decl_order(decl_order: Vec<DataId>) -> NameSidecar {
    let mut s = NameSidecar::default();
    let i32_frame = ResolvedType {
        scalar: ScalarType::I32,
        dims: vec![4, 16],
    };
    s.data_types.insert(SPK_OUT, i32_frame.clone());
    s.data_types.insert(BT_OUT, i32_frame);
    s.data_decl_order = decl_order;
    s
}

/// NameTables naming the two ex14 savers (`fe`, `rf`).
fn fe_rf_names() -> NameTables {
    let mut n = NameTables::default();
    n.worker.insert(FE, "fe".to_string());
    n.worker.insert(RF, "rf".to_string());
    n
}

/// The two-saver ex14 output partition: fe saves spk_out, rf saves bt_out.
fn ex14_two_savers() -> BTreeMap<WorkerId, Vec<Event>> {
    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    pw.insert(FE, vec![save(SPK_OUT)]);
    pw.insert(RF, vec![save(BT_OUT)]);
    pw
}

/// Build the TransportPlan for the two-saver ex14 fixture under the given
/// declaration order.
fn ex14_plan(decl_order: Vec<DataId>) -> TransportPlan {
    let per_worker = ex14_two_savers();
    let names = fe_rf_names();
    let sidecar = sidecar_with_decl_order(decl_order);
    TransportPlan::build(&per_worker, &names, &sidecar).expect("ex14 two-saver plan builds")
}

#[test]
fn multi_saver_each_gets_a_distinct_capture_var_and_right_symbol() {
    // AC#1 + AC#2 emit-shape: with TWO savers, fe and rf get DISTINCT capture
    // file vars (NOT both `$uartFile`), and each OutputCapture names the
    // RIGHT drained symbol (fe -> spk_out, rf -> bt_out).
    let plan = ex14_plan(vec![SPK_OUT, BT_OUT]);

    // The OutputCapture list (single source of truth) pairs each saver's
    // DISTINCT var with the correct drained symbol.
    let cap_of = |w: WorkerId| {
        plan.output_captures
            .iter()
            .find(|c| c.worker == w)
            .unwrap_or_else(|| panic!("no OutputCapture for worker {w:?}"))
    };
    assert_eq!(cap_of(FE).data, SPK_OUT, "fe drains spk_out");
    assert_eq!(cap_of(RF).data, BT_OUT, "rf drains bt_out");
    assert_eq!(cap_of(FE).file_var, "feUart", "fe multi-saver var");
    assert_eq!(cap_of(RF).file_var, "rfUart", "rf multi-saver var");
    assert_ne!(
        cap_of(FE).file_var,
        cap_of(RF).file_var,
        "the two savers must NOT share one capture var"
    );
    // Both are recognised as savers.
    assert!(plan.workers[&FE].saves_output && plan.workers[&RF].saves_output);

    // The generated .resc gives the two savers DISTINCT CreateFileBackend
    // vars — no shared `$uartFile` write-collision.
    let resc = render_multimachine_resc(&plan);
    assert!(
        resc.contains("usart1 CreateFileBackend $feUart true"),
        "fe USART1 must capture to $feUart:\n{resc}"
    );
    assert!(
        resc.contains("usart1 CreateFileBackend $rfUart true"),
        "rf USART1 must capture to $rfUart:\n{resc}"
    );
    assert!(
        !resc.contains("$uartFile"),
        "multi-saver .resc must NOT reference the shared $uartFile:\n{resc}"
    );
}

#[test]
fn capture_order_is_decl_order_spk_then_bt() {
    // AC#2 order: ex14's declaration order is [spk_out, bt_out] (line 80
    // before 81). The ordered capture list MUST be [spk_out(fe), bt_out(rf)]
    // — matching reference.bin spk_out@0 ++ bt_out@256 — NOT the DataId
    // reversal (bt_out=1 < spk_out=4) which would put bt_out first.
    let plan = ex14_plan(vec![SPK_OUT, BT_OUT]);
    let order: Vec<(DataId, WorkerId)> = plan
        .output_captures
        .iter()
        .map(|c| (c.data, c.worker))
        .collect();
    assert_eq!(
        order,
        vec![(SPK_OUT, FE), (BT_OUT, RF)],
        "capture order must follow declaration order (spk_out first), \
         not alphabetical DataId (which would put bt_out first)"
    );
}

#[test]
fn flipping_decl_order_flips_the_capture_order_the_bite() {
    // THE BITE (non-tautological): hold the DataIds FIXED and REVERSED
    // (bt_out=1 < spk_out=4, the real ex14 alphabetical assignment), and
    // assert the capture order TRACKS `data_decl_order` across BOTH possible
    // declaration orders. A DataId-derived order would be INVARIANT (always
    // bt_out-first, since 1 < 4) — so the two results would be EQUAL. They
    // are NOT: flipping decl order flips the capture order, which can only
    // happen if the computation reads `data_decl_order`, not `DataId`.
    let order = |decl: Vec<DataId>| -> Vec<DataId> {
        ex14_plan(decl)
            .output_captures
            .iter()
            .map(|c| c.data)
            .collect()
    };
    let spk_first = order(vec![SPK_OUT, BT_OUT]);
    let bt_first = order(vec![BT_OUT, SPK_OUT]);

    assert_eq!(
        spk_first,
        vec![SPK_OUT, BT_OUT],
        "decl order [spk_out, bt_out] -> capture order spk_out-first"
    );
    assert_eq!(
        bt_first,
        vec![BT_OUT, SPK_OUT],
        "decl order [bt_out, spk_out] -> capture order bt_out-first"
    );
    assert_ne!(
        spk_first, bt_first,
        "the capture order MUST differ between the two decl orders \
         (DataIds fixed) — proving it bites on data_decl_order, not DataId"
    );
}

#[test]
fn single_saver_keeps_uartfile_recipe_compatible() {
    // AC#1 + AC#3 regression: 02-split-add `host` is the SINGLE saver. Its
    // capture var MUST stay `uartFile` (byte-identical to pre-C2) so the
    // `just renode-multimcu` recipe (which injects/reads $uartFile) keeps
    // passing; the `.resc` must emit `CreateFileBackend $uartFile`.
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(HOST, vec![save(SPK_OUT)]);
    let mut names = NameTables::default();
    names.worker.insert(HOST, "host".to_string());
    let sidecar = sidecar_with_decl_order(vec![SPK_OUT, BT_OUT]);

    let plan = TransportPlan::build(&per_worker, &names, &sidecar)
        .expect("single-saver host plan builds");

    assert!(plan.workers[&HOST].saves_output, "host is the saver");
    assert_eq!(plan.output_captures.len(), 1);
    assert_eq!(
        plan.output_captures[0].worker, HOST,
        "the single capture is the host worker"
    );
    assert_eq!(
        plan.output_captures[0].file_var, "uartFile",
        "single saver must keep the recipe-compatible $uartFile var"
    );

    let resc = render_multimachine_resc(&plan);
    assert!(
        resc.contains("usart1 CreateFileBackend $uartFile true"),
        "single-saver .resc must keep CreateFileBackend $uartFile:\n{resc}"
    );
}

#[test]
fn saved_symbol_absent_from_decl_order_fails_loud() {
    // panic-not-diagnostic guard (mirror of the INPUT side): a saved symbol
    // absent from data_decl_order is a contract desync. compute_output_capture
    // must fail loud (typed EmitError naming the gap), not emit a wrong order.
    let per_worker = ex14_two_savers();
    let names = fe_rf_names();
    // decl_order omits bt_out entirely.
    let sidecar = sidecar_with_decl_order(vec![SPK_OUT]);

    let err = TransportPlan::build(&per_worker, &names, &sidecar)
        .expect_err("missing decl-order entry for a saved symbol must fail loud");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("data_decl_order"),
        "error should name the decl-order contract gap, got: {msg}"
    );
}
