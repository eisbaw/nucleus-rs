//! TASK-0049.05.01 — tests for `multimcu::verify_control_sync_subsumed`,
//! the fail-loud guard against a control-only `Event::Sync` that the
//! multi-MCU `MultiMcuShim`'s no-op `irq_barrier` would silently
//! miscompile.
//!
//! The guard's correctness boundary is precise: a no-op barrier is
//! value-correct whenever its ordering is subsumed by a blocking data
//! edge (the common case — proven byte-exact for 02-split-add by
//! `just renode-multimcu`), and UNSAFE only when it orders two workers'
//! external IO side effects with no connecting data edge. These tests pin
//! both sides: the real shipped schedule is ACCEPTED, and a synthetic
//! standalone-barrier schedule is REJECTED, while a data-edge-subsumed
//! barrier and a single-participant barrier are ACCEPTED.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::algo::{IrExpr, Purity, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId, SeqTag, SyncKind, SyncTag,
    WorkerId,
};
use nucleus_compiler::sidecar::{KernelSig, NameSidecar};
use nucleus_compiler::NameTables;

use crate::multimcu::verify_control_sync_subsumed;
use super::repo_root;

/// An EMPTY sidecar for the synthetic guard tests. Sound because their
/// only IO firings are `io_save()` (output `None`), which `is_effectful_io`
/// classifies WITHOUT a purity lookup (the `output.is_none()` short-circuit
/// fires before the sidecar is consulted) — so no `KernelSig` is needed
/// (TASK-0049.10.04 threaded `sidecar` for the INDEXED-effectful arm, which
/// these synthetic save-only fixtures never exercise).
fn empty_sidecar() -> NameSidecar {
    NameSidecar::default()
}

/// An effectful SAVE firing (`save_output(..)`): output `None` — a
/// globally-observable external IO side effect, per `render_fire`.
fn io_save() -> Event {
    Event::fire(
        KernelId(99),
        IterTile::empty(),
        FireBinding {
            inputs: vec![],
            output: None,
        },
    )
}

fn barrier(tag: u64, participants: &[WorkerId]) -> Event {
    Event::Sync {
        participants: participants.iter().copied().collect::<BTreeSet<_>>(),
        kind: SyncKind::Barrier,
        sync: SyncTag(tag),
    }
}

fn push(dst: WorkerId, seq: u64) -> Event {
    Event::Push {
        dst,
        data: DataId(0),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

fn wait(src: WorkerId, seq: u64) -> Event {
    Event::Wait {
        src,
        data: DataId(0),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

/// Wrap a barrier in a single-iteration `Loop` — exercises the
/// `flatten_salients` loop-body inlining (an in-loop barrier, like
/// 02-split-add's per-iteration `barrier1`).
fn loop_barrier(tag: u64, participants: &[WorkerId]) -> Event {
    Event::loop_over(IterVar(0), 0..4, vec![barrier(tag, participants)])
}

fn names_for(workers: &[(WorkerId, &str)]) -> NameTables {
    let mut n = NameTables::default();
    for (w, name) in workers {
        n.worker.insert(*w, (*name).to_string());
    }
    n
}

const WA: WorkerId = WorkerId(1);
const WB: WorkerId = WorkerId(2);

/// REJECT: two workers each do external IO straddling a shared barrier
/// with NO data edge between them. The no-op `irq_barrier` would drop the
/// IO ordering silently, so emit must fail loud.
#[test]
fn standalone_control_barrier_ordering_cross_worker_io_is_rejected() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WA, vec![io_save(), barrier(7, &[WA, WB])]);
    per_worker.insert(WB, vec![barrier(7, &[WA, WB]), io_save()]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);

    let err = verify_control_sync_subsumed(&per_worker, &names, &empty_sidecar())
        .expect_err("a standalone control barrier ordering cross-worker IO must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("sync tag 7") && msg.contains("fe") && msg.contains("rf"),
        "rejection must name the offending barrier tag and both workers: {msg}"
    );
}

/// ACCEPT: the SAME straddling IO, but now a Push/Wait data edge from the
/// before-worker to the after-worker carries the ordering. The barrier is
/// subsumed, so the no-op is correct and emit must NOT reject.
#[test]
fn barrier_subsumed_by_a_data_edge_is_accepted() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // WA: IO, then push(seq5) to WB, then barrier.
    per_worker.insert(WA, vec![io_save(), push(WB, 5), barrier(7, &[WA, WB])]);
    // WB: barrier, then wait(seq5) from WA, then IO.
    per_worker.insert(WB, vec![barrier(7, &[WA, WB]), wait(WA, 5), io_save()]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);

    verify_control_sync_subsumed(&per_worker, &names, &empty_sidecar())
        .expect("a barrier whose IO ordering is carried by a data edge must be accepted");
}

/// ACCEPT: a barrier carried by only ONE worker orders nothing — vacuous,
/// never rejected even with external IO around it.
#[test]
fn single_participant_barrier_is_accepted() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WA, vec![io_save(), barrier(7, &[WA]), io_save()]);
    // WB participates in transport but not the barrier.
    per_worker.insert(WB, vec![wait(WA, 5)]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);

    verify_control_sync_subsumed(&per_worker, &names, &empty_sidecar())
        .expect("a single-participant barrier is vacuous and must be accepted");
}

/// ACCEPT: an IN-LOOP barrier (the 02-split-add `barrier1` analogue) whose
/// straddling cross-worker IO is carried by a data edge. Pins that
/// `flatten_salients` inlines the loop body so the before/after-barrier IO
/// straddle linearizes correctly (architect P3-2).
#[test]
fn loop_nested_barrier_subsumed_by_data_edge_is_accepted() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WA, vec![io_save(), push(WB, 5), loop_barrier(9, &[WA, WB])]);
    per_worker.insert(WB, vec![loop_barrier(9, &[WA, WB]), wait(WA, 5), io_save()]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);
    verify_control_sync_subsumed(&per_worker, &names, &empty_sidecar())
        .expect("an in-loop barrier subsumed by a data edge must be accepted");
}

/// REJECT: the same in-loop barrier, but with NO data edge connecting the
/// straddling IO — the linearization must still catch it.
#[test]
fn loop_nested_standalone_barrier_ordering_cross_worker_io_is_rejected() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WA, vec![io_save(), loop_barrier(9, &[WA, WB])]);
    per_worker.insert(WB, vec![loop_barrier(9, &[WA, WB]), io_save()]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);
    verify_control_sync_subsumed(&per_worker, &names, &empty_sidecar())
        .expect_err("an in-loop standalone control barrier ordering cross-worker IO must reject");
}

/// An INDEXED effectful LOAD firing (`mic_in[frame] <-- fe_capture()`):
/// effectful kernel `EFF_LOAD_K`, no inputs, output with NON-EMPTY indices.
/// Before TASK-0049.10.04 this was mis-classified as a pure indexed compute
/// (NOT IO); the purity-gated arm now recognises it.
const EFF_LOAD_K: KernelId = KernelId(50);
fn indexed_effectful_load() -> Event {
    Event::fire(
        EFF_LOAD_K,
        IterTile::empty(),
        FireBinding {
            inputs: vec![],
            output: Some(DataSlice {
                data: DataId(0),
                indices: vec![IrExpr::IntLit(0)],
            }),
        },
    )
}

/// A sidecar in which `EFF_LOAD_K` is declared `effectful` — the bit the
/// new `is_effectful_load` indexed arm consults.
fn sidecar_with_effectful_load() -> NameSidecar {
    let mut sc = NameSidecar::default();
    sc.kernel_sigs.insert(
        EFF_LOAD_K,
        KernelSig {
            params: vec![],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            }),
            purity: Purity::Effectful,
            combine: None,
        },
    );
    sc
}

/// REJECT (TASK-0049.10.04 classification pin): two workers each do an
/// INDEXED effectful LOAD straddling a shared barrier with NO data edge.
/// This is the silent-sibling of slice A: before the purity-gated arm,
/// indexed effectful loads were NOT counted as IO, so this pair would have
/// been (wrongly) ACCEPTED. With the fix they ARE cross-worker IO, so the
/// standalone barrier ordering them must fail loud — proving the guard now
/// sees an indexed effectful load as a globally-observable side effect.
#[test]
fn indexed_effectful_load_is_classified_as_cross_worker_io() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WA, vec![indexed_effectful_load(), barrier(7, &[WA, WB])]);
    per_worker.insert(WB, vec![barrier(7, &[WA, WB]), indexed_effectful_load()]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);
    let sc = sidecar_with_effectful_load();

    let err = verify_control_sync_subsumed(&per_worker, &names, &sc).expect_err(
        "an indexed effectful load must now count as cross-worker IO (TASK-0049.10.04 \
         silent-sibling of slice A); a standalone barrier ordering two of them must reject",
    );
    let msg = format!("{err:?}");
    assert!(
        msg.contains("sync tag 7") && msg.contains("fe") && msg.contains("rf"),
        "rejection must name the offending barrier tag + both workers: {msg}"
    );
}

/// ACCEPT (additivity pin): the SAME indexed effectful loads, but declared
/// PURE. A pure indexed compute is NOT observable across MCUs, so it is not
/// IO and the barrier orders nothing — must be accepted. This confirms the
/// new arm is purity-GATED, not a blanket "indexed output => IO".
#[test]
fn indexed_pure_compute_is_not_cross_worker_io() {
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WA, vec![indexed_effectful_load(), barrier(7, &[WA, WB])]);
    per_worker.insert(WB, vec![barrier(7, &[WA, WB]), indexed_effectful_load()]);
    let names = names_for(&[(WA, "fe"), (WB, "rf")]);
    // Same fixture but EFF_LOAD_K declared PURE.
    let mut sc = NameSidecar::default();
    sc.kernel_sigs.insert(
        EFF_LOAD_K,
        KernelSig {
            params: vec![],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            }),
            purity: Purity::Pure,
            combine: None,
        },
    );

    verify_control_sync_subsumed(&per_worker, &names, &sc).expect(
        "a PURE indexed compute is not cross-worker IO; the barrier orders nothing \
         and must be accepted (additivity of the purity-gated arm)",
    );
}

/// ACCEPT: the real shipped multi-MCU schedule. `02-split-add/split`
/// routes ALL external IO through `host` (`w0` only receives/computes/
/// pushes), so no barrier has two IO-bearing participants and the guard is
/// inert — this is the byte-exact path `just renode-multimcu` validates.
#[test]
fn real_02_split_add_multi_mcu_schedule_is_accepted() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("algo source");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("sched source");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: false,
        },
    );
    verify_control_sync_subsumed(&r.per_worker, &r.names, &r.sidecar)
        .expect("02-split-add/split is the shipped byte-exact path; the guard must accept it");
}
