//! TASK-0049.05.03 — boot-order (machine-release order) tests.
//!
//! The generated `.resc` releases the co-simulated machines one at a time
//! (RX-enable-before-TX start-gating, TASK-0049.01: Renode's
//! `UARTBase.WriteChar` DROPS bytes that arrive before the receiver enables
//! RX). The OLD `compute_boot_order` sorted by `(waits_before_first_push
//! DESC, worker_id ASC)`; that tiebreak got ex14 WRONG by luck — it released
//! `fe` (a cascade trigger) before `rf` (a receiver of `dsp`'s `bt_out`), so
//! `dsp`'s frame-0 `bt_out` push hit `rf` while `rf`'s RX was still disabled
//! and the 256B were dropped, stalling `rf` one frame short (rfUart=192B).
//!
//! The fix simulates the staged release as a deterministic fixpoint
//! ([`crate::multimcu::compute_boot_order`]) so a worker's first `link_push`
//! to a peer P never precedes P's release. These tests pin that the ex14
//! analogue releases `dsp` first and `rf` BEFORE `fe`, and that the
//! 02-split-add 2-worker order is unchanged. The ex14 test BITES the old
//! tiebreak: under worker_id-ASC, `fe` (lower id) would precede `rf`.

use std::collections::BTreeMap;

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::multimcu::TransportPlan;

fn push(dst: WorkerId, data: u64, seq: u64) -> Event {
    Event::Push {
        dst,
        data: DataId(data),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

fn wait(src: WorkerId, data: u64, seq: u64) -> Event {
    Event::Wait {
        src,
        data: DataId(data),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

/// Wrap a worker's frame in a single rolled loop, mirroring how the real
/// per-frame schedule projects (`Event::Loop`). The boot-order simulation
/// must see through the loop (one pass = the frame shape).
fn framed(body: Vec<Event>) -> Vec<Event> {
    vec![Event::Loop {
        iter_var: nucleus_compiler::event::IterVar(0),
        range: 0..4,
        body,
        block_tag: None,
        check_frame: None,
        break_cond: None,
    }]
}

fn names_for(workers: &[(WorkerId, &str)]) -> NameTables {
    let mut n = NameTables::default();
    for (w, name) in workers {
        n.worker.insert(*w, (*name).to_string());
    }
    n
}

/// THE BITE. ex14's per-worker link structure (read off the generated
/// firmware), assigned worker ids so the OLD `worker_id ASC` tiebreak would
/// place `fe` before `rf`:
///   * `dsp` (id 1) is RECEIVE-GATED: its first action recvs `mic_in` from
///     `fe`, THEN pushes `bt_out` to `rf` (seq1), then recvs from fe/rf, then
///     pushes `spk_out` to fe.
///   * `fe` (id 2) is an EAGER SENDER: pushes `mic_in` to dsp (seq2, seq0)
///     before it recvs `spk_out` (seq4).
///   * `rf` (id 3) is an EAGER SENDER: pushes `bt_in` to dsp (seq3) before it
///     recvs `bt_out` (seq1).
///
/// `fe`<`rf` by worker id, so the OLD `(waits DESC, worker_id ASC)` sort —
/// both `fe` and `rf` tie at 0 waits-before-first-push — released `fe` before
/// `rf`, which is the byte-loss bug. The fixpoint must release `dsp` first
/// (receive-gated, harmless early) then `rf` (its only push targets the
/// already-released `dsp`, so releasing it is safe) then `fe` (releasing `fe`
/// earlier would let `dsp` push `bt_out` to the not-yet-released `rf`).
#[test]
fn ex14_releases_dsp_first_then_rf_before_fe() {
    const DSP: WorkerId = WorkerId(1);
    const FE: WorkerId = WorkerId(2);
    const RF: WorkerId = WorkerId(3);

    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // dsp: recv(0 mic_in<-fe), push(1 bt_out->rf), recv(2 mic_in<-fe),
    //      recv(3 bt_in<-rf), push(4 spk_out->fe).
    pw.insert(
        DSP,
        framed(vec![
            wait(FE, 0, 0),
            push(RF, 1, 1),
            wait(FE, 0, 2),
            wait(RF, 2, 3),
            push(FE, 3, 4),
        ]),
    );
    // fe: push(2 mic_in->dsp), push(0 mic_in->dsp), recv(4 spk_out<-dsp).
    pw.insert(
        FE,
        framed(vec![push(DSP, 0, 2), push(DSP, 0, 0), wait(DSP, 3, 4)]),
    );
    // rf: push(3 bt_in->dsp), recv(1 bt_out<-dsp).
    pw.insert(RF, framed(vec![push(DSP, 2, 3), wait(DSP, 1, 1)]));

    let names = names_for(&[(DSP, "dsp"), (FE, "fe"), (RF, "rf")]);
    let plan = TransportPlan::build(&pw, &names, &NameSidecar::default())
        .expect("ex14 analogue builds");

    assert_eq!(
        plan.boot_order,
        vec![DSP, RF, FE],
        "ex14 boot order must release dsp first, then rf BEFORE fe \
         (RX-enable rf before dsp's bt_out push); got {:?}. The old \
         worker_id-ASC tiebreak would give [DSP, FE, RF] — the byte-loss bug.",
        plan.boot_order
    );

    // Explicit anti-regression: rf MUST come before fe.
    let pos = |w: WorkerId| plan.boot_order.iter().position(|x| *x == w).unwrap();
    assert!(
        pos(RF) < pos(FE),
        "rf (a receiver of dsp's bt_out) must be released before fe (the \
         cascade trigger); boot_order={:?}",
        plan.boot_order
    );
}

/// Regression: the 02-split-add 2-worker `split` shape — `host` pushes `a`
/// (seq0) + `b` (seq1) to `w0` then recvs `c` (seq2); `w0` recvs a+b,
/// computes, pushes `c`. The receive-gated `w0` must boot first so `host`'s
/// opening pushes land on an RX-enabled peer. This order is UNCHANGED from
/// the old heuristic (w0 had 2 waits-before-push, host 0) — the renode
/// byte-exact 1024B path depends on it staying `[w0, host]`.
#[test]
fn split_two_worker_order_unchanged() {
    const HOST: WorkerId = WorkerId(0);
    const W0: WorkerId = WorkerId(1);

    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // host: push(0 a->w0), push(1 b->w0), recv(2 c<-w0).
    pw.insert(
        HOST,
        framed(vec![push(W0, 0, 0), push(W0, 1, 1), wait(W0, 2, 2)]),
    );
    // w0: recv(0 a<-host), recv(1 b<-host), push(2 c->host).
    pw.insert(
        W0,
        framed(vec![wait(HOST, 0, 0), wait(HOST, 1, 1), push(HOST, 2, 2)]),
    );

    let names = names_for(&[(HOST, "host"), (W0, "w0")]);
    let plan = TransportPlan::build(&pw, &names, &NameSidecar::default())
        .expect("02-split-add analogue builds");

    assert_eq!(
        plan.boot_order,
        vec![W0, HOST],
        "02-split-add must release the receive-gated w0 first, then host \
         (unchanged from the old heuristic); got {:?}",
        plan.boot_order
    );
}
