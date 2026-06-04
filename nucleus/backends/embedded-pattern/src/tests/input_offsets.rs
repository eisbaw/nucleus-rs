//! TASK-0049.10.06 — tests for `multimcu::compute_input_offsets`, the
//! per-worker `input.bin` byte-offset computation for the cross-worker
//! input partition (BLOCKER 2 root fix).
//!
//! The load-bearing property: the global `input.bin` layout is ordered by
//! data-DECLARATION order (`NameSidecar::data_decl_order`), NOT by
//! `DataId`. `DataId` is assigned ALPHABETICALLY by `acfg::build`, which
//! for ex14 yields `bt_in`=DataId(0) BEFORE `mic_in`=DataId(2) — the
//! REVERSE of the reference generator's hand-written layout (mic at byte
//! 0, bt at byte 256). These tests construct exactly that REVERSED-DataId
//! shape and assert the offsets follow declaration order, then FLIP the
//! decl-order signal and assert the offsets flip with it — proving the
//! computation bites on `data_decl_order`, not on `DataId`.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, DataSlice, Event, FireBinding, IterTile, KernelId, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use crate::multimcu::compute_input_offsets;

// ex14-shaped DataIds: assigned ALPHABETICALLY, so bt_in < mic_in.
const BT_IN: DataId = DataId(0); // "bt_in"  -> DataId(0)
const MIC_IN: DataId = DataId(2); // "mic_in" -> DataId(2)

// ex14 frame shape: N_FRAMES(4) * SAMPLES_PER_FRAME(16) = 64 i32 = 256 bytes.
const FRAME_ELEMS: usize = 4 * 16;
const FRAME_BYTES: usize = FRAME_ELEMS * 4; // i32 = 4 bytes

const FE: WorkerId = WorkerId(1); // loads mic_in
const RF: WorkerId = WorkerId(2); // loads bt_in

/// A whole-array effectful LOAD `Fire` of `data` (output present, EMPTY
/// indices — the STRUCTURAL load shape `is_effectful_load` recognises
/// without a purity lookup, so the fixtures need no `KernelSig`).
fn load(data: DataId) -> Event {
    Event::fire(
        KernelId(7),
        IterTile::empty(),
        FireBinding {
            inputs: vec![],
            output: Some(DataSlice {
                data,
                indices: vec![],
            }),
        },
    )
}

/// A sidecar carrying ex14-shaped i32[4][16] types for mic_in + bt_in,
/// with the given declaration order.
fn sidecar_with_decl_order(decl_order: Vec<DataId>) -> NameSidecar {
    let mut s = NameSidecar::default();
    let i32_frame = ResolvedType {
        scalar: ScalarType::I32,
        dims: vec![4, 16],
    };
    s.data_types.insert(MIC_IN, i32_frame.clone());
    s.data_types.insert(BT_IN, i32_frame);
    s.data_decl_order = decl_order;
    s
}

/// The two-loader ex14 input partition: fe loads mic_in, rf loads bt_in.
fn ex14_two_loaders() -> BTreeMap<WorkerId, Vec<Event>> {
    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    pw.insert(FE, vec![load(MIC_IN)]);
    pw.insert(RF, vec![load(BT_IN)]);
    pw
}

#[test]
fn decl_order_mic_then_bt_gives_fe0_rf256() {
    // Declaration order [mic_in, bt_in] — matching ex14's source. NOTE the
    // DataIds are REVERSED (bt_in=0 < mic_in=2): a DataId-order computation
    // would (wrongly) put bt_in first.
    let sidecar = sidecar_with_decl_order(vec![MIC_IN, BT_IN]);
    let per_worker = ex14_two_loaders();
    let used = vec![FE, RF];

    let offsets = compute_input_offsets(&used, &per_worker, &sidecar)
        .expect("ex14 two-loader offsets compute");

    assert_eq!(
        offsets.get(&FE).copied(),
        Some(0),
        "fe loads mic_in, declared FIRST -> byte offset 0"
    );
    assert_eq!(
        offsets.get(&RF).copied(),
        Some(FRAME_BYTES),
        "rf loads bt_in, declared SECOND -> byte offset {FRAME_BYTES} (256)"
    );
}

#[test]
fn flipping_decl_order_flips_the_offsets_the_bite() {
    // FLIP declaration order to [bt_in, mic_in]. If the computation read
    // DataId order instead of decl order, the offsets would NOT move (DataId
    // order is fixed: bt_in=0 < mic_in=2 in BOTH cases). They DO move, which
    // proves the computation bites on `data_decl_order`.
    let sidecar = sidecar_with_decl_order(vec![BT_IN, MIC_IN]);
    let per_worker = ex14_two_loaders();
    let used = vec![FE, RF];

    let offsets = compute_input_offsets(&used, &per_worker, &sidecar)
        .expect("flipped two-loader offsets compute");

    assert_eq!(
        offsets.get(&FE).copied(),
        Some(FRAME_BYTES),
        "after flip: fe loads mic_in, now declared SECOND -> byte offset 256"
    );
    assert_eq!(
        offsets.get(&RF).copied(),
        Some(0),
        "after flip: rf loads bt_in, now declared FIRST -> byte offset 0"
    );
}

#[test]
fn single_loader_stays_offset_zero() {
    // 02-split-add-shaped: ONE worker loads everything. Must early-return 0,
    // byte-identical to pre-TASK-0049.10.04 (the renode 02-split-add path).
    let sidecar = sidecar_with_decl_order(vec![MIC_IN, BT_IN]);
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(FE, vec![load(MIC_IN), load(BT_IN)]);
    let used = vec![FE];

    let offsets =
        compute_input_offsets(&used, &per_worker, &sidecar).expect("single-loader offsets compute");

    assert_eq!(offsets.get(&FE).copied(), Some(0), "single loader -> offset 0");
    assert_eq!(offsets.len(), 1);
}

#[test]
fn loaded_symbol_absent_from_decl_order_fails_loud() {
    // decl_order omits bt_in entirely: the layout cannot be computed (a
    // loaded symbol has no position). Must fail loud, not emit a wrong
    // offset.
    let sidecar = sidecar_with_decl_order(vec![MIC_IN]); // bt_in missing
    let per_worker = ex14_two_loaders();
    let used = vec![FE, RF];

    let err = compute_input_offsets(&used, &per_worker, &sidecar)
        .expect_err("missing decl-order entry must fail loud");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("data_decl_order"),
        "error should name the decl-order contract gap, got: {msg}"
    );
}
