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

use nucleus_compiler::event::{
    DataId, Event, FireBinding, IterTile, KernelId, SeqTag, SyncKind, SyncTag, WorkerId,
};
use nucleus_compiler::NameTables;

use crate::multimcu::verify_control_sync_subsumed;
use super::repo_root;

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

    let err = verify_control_sync_subsumed(&per_worker, &names)
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

    verify_control_sync_subsumed(&per_worker, &names)
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

    verify_control_sync_subsumed(&per_worker, &names)
        .expect("a single-participant barrier is vacuous and must be accepted");
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
    verify_control_sync_subsumed(&r.per_worker, &r.names)
        .expect("02-split-add/split is the shipped byte-exact path; the guard must accept it");
}
