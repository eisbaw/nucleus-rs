//! Falsifier pin for the multi-worker `for..until` early-exit guard
//! (epic S7, TASK-0341.02.01.08).
//!
//! ## What this pins, and why it is load-bearing
//!
//! The `for..until` break EMIT (the runtime `if <cond> { break; }` plus
//! the break-generation capture, the runtime final-read, and the cap-hit
//! observability) is implemented for the SINGLE-WORKER sequential path
//! ONLY (`backend_common::single_worker_main`, epics S4/S5). The shared
//! MULTI-worker walker (`render_worker_events`, used by pthreads-sync /
//! pthreads-async / mp-tcp-event / mp-uds-event / openmp-rs multi-worker
//! arms) does NOT yet emit a multi-worker break.
//!
//! A multi-worker break is NOT a mechanical lift of the single-worker
//! emit: per the S7 collective-break invariant (TASK-0341.02.01.08 AC#1),
//! every worker must agree on the early-exit generation `k` BY
//! CONSTRUCTION — the convergence predicate is a GLOBAL reduction
//! (max-of-per-worker-partials) that must be all-reduced at a barrier and
//! broadcast, so every worker branches on the IDENTICAL global value. If
//! each worker instead tested only its LOCAL partial, worker A could
//! break at iteration 7 while worker B is still waiting at the next
//! barrier — a nondeterministic deadlock (and a bit-identity violation).
//! Until that collective-break machinery lands, the conservatively
//! CORRECT behaviour is to reject a multi-worker `break_cond` FAIL-LOUD
//! rather than silently drop the predicate (which would mis-lower a
//! convergence loop to a non-terminating full-cap loop —
//! `feedback-option-none-skip-arm-silent-drop`).
//!
//! This test is the falsifier that BITES if a future cycle naively lifts
//! the guard at `multi_worker_walker/event_walker.rs` without supplying
//! the collective all-reduce + broadcast: a plain pass-through would emit
//! a per-worker-LOCAL break, which this test forbids by asserting the
//! walker still returns `EmitError::UnsupportedFeature`. When the real
//! collective break lands, this test must be REPLACED by a positive pin
//! (all workers break on the same received global decision), NOT merely
//! deleted.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{IndexedRef, IrCmpOp, IrExpr};
use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, KernelId, SeqTag, WorkerId};

use backend_common::multi_worker_walker::{render_worker_events, WalkerCtx};

mod common;

type RendezvousIds = BTreeMap<(DataId, SeqTag), usize>;
type PairTiles = BTreeMap<(DataId, SeqTag), IterTile>;

/// Empty cross-worker maps — this fixture exercises only the
/// `Event::Loop` body-entry guard, no Push/Wait rendezvous.
fn empty_walker_maps() -> (RendezvousIds, PairTiles) {
    (BTreeMap::new(), BTreeMap::new())
}

/// A `for..until` early-exit loop: `for t : 0..8 until maxdiff[t] <= 0`,
/// with an empty body. The break predicate is a scalar `Compare`
/// (`DataRef <= IntLit`) — the exact shape `petri_to_events` clones onto
/// every per-worker `Event::Loop` when a `for..until` lives inside a
/// `partition=workers` scope.
fn until_loop(iv: IterVar, _data: DataId) -> Event {
    let cond = IrExpr::Compare(
        IrCmpOp::Le,
        Box::new(IrExpr::DataRef(IndexedRef {
            name: "maxdiff".to_string(),
            indices: vec![IrExpr::Ident("t".to_string())],
        })),
        Box::new(IrExpr::IntLit(0)),
    );
    Event::Loop {
        iter_var: iv,
        range: 0..8,
        body: vec![],
        block_tag: None,
        check_frame: None,
        break_cond: Some(cond),
    }
}

#[test]
fn multi_worker_walker_rejects_break_cond_some_fail_loud() {
    // Fixture: a single source loop `t : 0..8` over data `maxdiff` with a
    // unary `i64` kernel (enough that the loop header + a Fire body would
    // render without a ContractGap; the body is empty here so only the
    // break-guard arm is exercised).
    let iv = IterVar(3);
    let data = DataId(9);
    let kernel = KernelId(1);
    let (names, sidecar) = common::Tables::new()
        .with_iter_var(iv, "t")
        .with_data_name(data, "maxdiff")
        .with_kernel_i64(kernel, "step")
        .with_loop_bound(iv, 0, 8)
        .build();

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();
    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    let res = render_worker_events(&ctx, WorkerId(0), &[until_loop(iv, data)], &mut out, 0, "");

    // The guard MUST fire: a multi-worker `break_cond:Some` is rejected
    // fail-loud (NOT silently dropped, NOT emitted as a per-worker-local
    // break). See the file header for the collective-break invariant this
    // protects.
    let err = res.expect_err(
        "S7 INVARIANT (TASK-0341.02.01.08): the multi-worker walker must \
         REJECT a `break_cond:Some` loop fail-loud until the collective \
         all-reduce+broadcast break lands. It returned Ok — either the \
         guard was lifted without supplying the collective break (which \
         would emit an UNSOUND per-worker-LOCAL break), or the predicate \
         was silently dropped (mis-lowering the convergence loop to a \
         non-terminating full-cap loop). Emitted:\n\
         ----\n{out}\n----",
    );

    let msg = format!("{err}");
    // The message must name the multi-worker break limitation AND point
    // at the S7 task, so a maintainer who hits it knows the remediation
    // is the collective break, not a one-line guard lift.
    assert!(
        msg.contains("for..until") || msg.contains("break"),
        "the fail-loud message must name the `for..until` / break \
         limitation; got: {msg}"
    );
    assert!(
        msg.contains("TASK-0341.02.01.08") || msg.contains("collective"),
        "the fail-loud message must point at the S7 collective-break work \
         (TASK-0341.02.01.08) so the remediation is discoverable; got: {msg}"
    );
}

#[test]
fn multi_worker_walker_accepts_plain_loop_no_break() {
    // Sibling positive control: the SAME fixture with `break_cond: None`
    // (a plain `for`) must render WITHOUT error — so the test above is
    // pinning the `Some` arm specifically, not a blanket loop rejection.
    let iv = IterVar(3);
    let data = DataId(9);
    let kernel = KernelId(1);
    let (names, sidecar) = common::Tables::new()
        .with_iter_var(iv, "t")
        .with_data_name(data, "maxdiff")
        .with_kernel_i64(kernel, "step")
        .with_loop_bound(iv, 0, 8)
        .build();

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();
    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let plain = Event::Loop {
        iter_var: iv,
        range: 0..8,
        body: vec![],
        block_tag: None,
        check_frame: None,
        break_cond: None,
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[plain], &mut out, 0, "")
        .expect("a plain `for` loop (break_cond:None) must render without error");
    assert!(
        out.contains("for t in"),
        "the plain loop must emit a `for t in ..` header; got:\n{out}"
    );
    assert!(
        !out.contains("break"),
        "a plain `for` loop must NOT emit any break; got:\n{out}"
    );
}
