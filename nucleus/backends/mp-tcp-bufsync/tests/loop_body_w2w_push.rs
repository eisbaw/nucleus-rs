//! mp-tcp-bufsync defensive ContractGap test for the
//! Loop-body-w2w-Push host-relay hazard (TASK-0330 AC#1 + AC#2).
//!
//! Mirror of `nucleus/backends/mp-tcp-event/tests/loop_body_w2w_push.rs`
//! per the cycle-148/149 paired-lift discipline
//! ([[feedback-silent-sibling-defect]]): the same defensive guard
//! exists in both backends' `collect_w2w_pushes`; both must reject the
//! same synthetic fixture with the same shape of typed error.
//!
//! Why this matters: `render_relay_phase` emits the relay block FLAT
//! outside any loop, so a w2w Push nested inside an `Event::Loop` body
//! would either over-count (host reads once per (seq, dst) but the
//! worker pushes N times around the loop) or mis-order (the flat read
//! order would not align with the loop's nested iteration order). The
//! cycle-148 architect P3.2 disclosure noted this as a dormant
//! limitation; TASK-0330 converts it to an active fail-loud guard per
//! [[feedback-panic-not-diagnostic-recurring]].

use std::path::PathBuf;

use mp_tcp_bufsync::{emit, NameTables};
use nucleus_compiler::sidecar::NameSidecar;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("three ancestors above mp-tcp-bufsync crate")
        .to_path_buf()
}

/// TASK-0330 AC#1 + AC#2 positive case: a synthetic schedule whose
/// non-host worker has a worker-to-worker Push nested inside an
/// `Event::Loop` body must fail-loud with `EmitError::ContractGap`.
///
/// 3-worker fixture (host + w1 + w2). w1 pushes to w2 (the w2w shape)
/// INSIDE a single-iteration `Event::Loop` body. The Loop wrapping is
/// the trigger — the same Push at TOP LEVEL would pass through the
/// cycle-148 host-relay path without firing this guard.
#[test]
fn loop_body_w2w_push_is_typed_contract_gap() {
    use nucleus_compiler::event::{
        DataId, Event, IterTile, IterVar, SeqTag, SyncKind, SyncTag, WorkerId,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let w2 = WorkerId(2);
    let data = DataId(0);
    let seq = SeqTag(0);

    let parts_all: BTreeSet<WorkerId> = [w_host, w1, w2].into_iter().collect();
    let barrier_1 = Event::Sync {
        participants: parts_all.clone(),
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };
    let barrier_2 = Event::Sync {
        participants: parts_all,
        kind: SyncKind::Barrier,
        sync: SyncTag(1),
    };

    // The hazardous shape: a single-iteration loop wrapping the w2w
    // Push. The loop range is 0..1 so it is structurally a Loop but
    // semantically equivalent to a single Push; the guard fires on
    // the STRUCTURAL nesting, not the iteration count.
    let nested_push = Event::Loop {
        iter_var: IterVar(0),
        range: 0..1,
        body: vec![Event::Push {
            dst: w2,
            data,
            tile: IterTile::empty(),
            seq,
        }],
        block_tag: None,
        check_frame: None,
    };

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(w_host, vec![barrier_1.clone(), barrier_2.clone()]);
    per_worker.insert(w1, vec![nested_push, barrier_1.clone(), barrier_2.clone()]);
    per_worker.insert(
        w2,
        vec![
            barrier_1,
            Event::Wait {
                src: w1,
                data,
                tile: IterTile::empty(),
                seq,
            },
            barrier_2,
        ],
    );

    let mut names = NameTables::default();
    names.worker.insert(w_host, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    names.worker.insert(w2, "w2".to_string());
    names.data.insert(data, "tmp".to_string());
    let mut sidecar = NameSidecar::default();
    sidecar.transfer_buffer_for_seq.insert(seq, 1);
    sidecar.data_types.insert(
        data,
        nucleus_compiler::algo::ResolvedType {
            scalar: nucleus_compiler::algo::ScalarType::I32,
            dims: vec![],
        },
    );

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-bufsync-test-scratch"),
        "loop_body_w2w_push_hazard",
    );

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    match r {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("mp-tcp-bufsync"),
                "ContractGap must name the backend prefix mp-tcp-bufsync: {msg}"
            );
            assert!(
                msg.contains("TASK-0330"),
                "ContractGap must forward-link TASK-0330 (this task): {msg}"
            );
            assert!(
                msg.contains("TASK-0327"),
                "ContractGap must forward-link TASK-0327 (the host-relay \
                 design this guard pins) per task AC#1: {msg}"
            );
            assert!(
                msg.contains("Loop"),
                "ContractGap must name Event::Loop as the structural \
                 trigger: {msg}"
            );
            assert!(
                msg.contains("FLAT outside any loop"),
                "ContractGap must pin the exact 'FLAT outside any loop' \
                 mechanism narrative (cycle-153 architect P3 nit): {msg}"
            );
        }
        Ok(_) => panic!(
            "expected ContractGap on a w2w Push nested inside an \
             Event::Loop body; TASK-0330 defensive guard was not \
             triggered. Emit returned Ok — the host-relay would emit \
             flat relay code that under-drains a nested Push."
        ),
    }
}

/// TASK-0330 AC#2 negative-path sanity: a host-bound `Push` (dst ==
/// host) inside an `Event::Loop` body must NOT trigger the new guard.
/// The guard fires only on the w2w shape (dst != host); host-bound
/// pushes are handled by the worker's own emit and never reach
/// `collect_w2w_pushes` as a relay-hop candidate.
///
/// Note: w1's Push at top level (dst=w2) is what populates relay_schedule
/// and exercises the cycle-148 lifted host-relay path. Inside the loop
/// body, w1 also pushes to host (dst=w_host) — that must NOT trigger
/// the guard because dst == host (the relay does not relay host-bound
/// pushes).
#[test]
fn host_bound_push_inside_loop_does_not_trigger_guard() {
    use nucleus_compiler::event::{
        DataId, Event, IterTile, IterVar, SeqTag, SyncKind, SyncTag, WorkerId,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let w2 = WorkerId(2);
    let data_w2w = DataId(0); // w1 -> w2 (relay candidate)
    let data_to_host = DataId(1); // w1 -> host inside loop body
    let seq_w2w = SeqTag(0);
    let seq_to_host = SeqTag(1);

    let parts_all: BTreeSet<WorkerId> = [w_host, w1, w2].into_iter().collect();
    let barrier_1 = Event::Sync {
        participants: parts_all.clone(),
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };
    let barrier_2 = Event::Sync {
        participants: parts_all,
        kind: SyncKind::Barrier,
        sync: SyncTag(1),
    };

    // Loop body contains a host-bound Push (dst == host) — must NOT
    // trigger the TASK-0330 guard. The guard's predicate is dst !=
    // host, so this body is ignored by collect_w2w_pushes entirely.
    let loop_with_host_push = Event::Loop {
        iter_var: IterVar(0),
        range: 0..1,
        body: vec![Event::Push {
            dst: w_host,
            data: data_to_host,
            tile: IterTile::empty(),
            seq: seq_to_host,
        }],
        block_tag: None,
        check_frame: None,
    };

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(
        w_host,
        vec![
            barrier_1.clone(),
            Event::Wait {
                src: w1,
                data: data_to_host,
                tile: IterTile::empty(),
                seq: seq_to_host,
            },
            barrier_2.clone(),
        ],
    );
    per_worker.insert(
        w1,
        vec![
            // Top-level w2w push (the cycle-148 lifted shape — exercises
            // the host-relay code path).
            Event::Push {
                dst: w2,
                data: data_w2w,
                tile: IterTile::empty(),
                seq: seq_w2w,
            },
            barrier_1.clone(),
            // Loop-body push to host — dst == host, must not trigger
            // the TASK-0330 guard.
            loop_with_host_push,
            barrier_2.clone(),
        ],
    );
    per_worker.insert(
        w2,
        vec![
            Event::Wait {
                src: w1,
                data: data_w2w,
                tile: IterTile::empty(),
                seq: seq_w2w,
            },
            barrier_1,
            barrier_2,
        ],
    );

    let mut names = NameTables::default();
    names.worker.insert(w_host, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    names.worker.insert(w2, "w2".to_string());
    names.data.insert(data_w2w, "tmp".to_string());
    names.data.insert(data_to_host, "to_host".to_string());
    // Worker emit recurses into Event::Loop and renders `for <name> in
    // range { ... }`; the iter_var must be named or the unrelated
    // "iter var ... has no name in NameTables" rejection fires before
    // the TASK-0330 guard's negative-path is exercised.
    names
        .iter_var
        .insert(nucleus_compiler::event::IterVar(0), "k".to_string());
    let mut sidecar = NameSidecar::default();
    sidecar.transfer_buffer_for_seq.insert(seq_w2w, 1);
    sidecar.transfer_buffer_for_seq.insert(seq_to_host, 1);
    sidecar.data_types.insert(
        data_w2w,
        nucleus_compiler::algo::ResolvedType {
            scalar: nucleus_compiler::algo::ScalarType::I32,
            dims: vec![],
        },
    );
    sidecar.data_types.insert(
        data_to_host,
        nucleus_compiler::algo::ResolvedType {
            scalar: nucleus_compiler::algo::ScalarType::I32,
            dims: vec![],
        },
    );

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-bufsync-test-scratch"),
        "loop_body_host_push_negative",
    );

    emit(&per_worker, &names, &sidecar, &kernels, &scratch).expect(
        "host-bound Push (dst == host) inside an Event::Loop body must NOT \
         trigger the TASK-0330 guard — the guard's predicate is dst != \
         host. collect_w2w_pushes ignores host-bound pushes entirely.",
    );
}

/// TASK-0330 cycle-153 architect P3.1 fold-back: multi-iteration loop
/// positive case. The cycle-153 base fixture uses `range: 0..1` (a
/// single-iteration loop) so the guard's "would over-count, host reads
/// once per (seq, dst) but worker pushes N times around the loop"
/// narrative was structurally true but quantitatively under-exercised
/// (N=1). This case pins the same guard on a `range: 0..3` loop where
/// N=3 makes the over-count narrative concrete.
#[test]
fn multi_iter_loop_body_w2w_push_is_typed_contract_gap() {
    use nucleus_compiler::event::{
        DataId, Event, IterTile, IterVar, SeqTag, SyncKind, SyncTag, WorkerId,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let w2 = WorkerId(2);
    let data = DataId(0);
    let seq = SeqTag(0);

    let parts_all: BTreeSet<WorkerId> = [w_host, w1, w2].into_iter().collect();
    let barrier_1 = Event::Sync {
        participants: parts_all.clone(),
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };
    let barrier_2 = Event::Sync {
        participants: parts_all,
        kind: SyncKind::Barrier,
        sync: SyncTag(1),
    };

    let nested_push = Event::Loop {
        iter_var: IterVar(0),
        range: 0..3,
        body: vec![Event::Push {
            dst: w2,
            data,
            tile: IterTile::empty(),
            seq,
        }],
        block_tag: None,
        check_frame: None,
    };

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(w_host, vec![barrier_1.clone(), barrier_2.clone()]);
    per_worker.insert(w1, vec![nested_push, barrier_1.clone(), barrier_2.clone()]);
    per_worker.insert(
        w2,
        vec![
            barrier_1,
            Event::Wait {
                src: w1,
                data,
                tile: IterTile::empty(),
                seq,
            },
            barrier_2,
        ],
    );

    let mut names = NameTables::default();
    names.worker.insert(w_host, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    names.worker.insert(w2, "w2".to_string());
    names.data.insert(data, "tmp".to_string());
    let mut sidecar = NameSidecar::default();
    sidecar.transfer_buffer_for_seq.insert(seq, 1);
    sidecar.data_types.insert(
        data,
        nucleus_compiler::algo::ResolvedType {
            scalar: nucleus_compiler::algo::ScalarType::I32,
            dims: vec![],
        },
    );

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-bufsync-test-scratch"),
        "multi_iter_loop_body_hazard",
    );

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    let e = r.expect_err(
        "expected ContractGap on multi-iteration Loop-body w2w Push (range 0..3); \
         TASK-0330 fires on the STRUCTURAL nesting regardless of iteration count",
    );
    let msg = format!("{e}");
    assert!(
        msg.contains("TASK-0330") && msg.contains("Loop"),
        "ContractGap must forward-link TASK-0330 + name the Loop trigger: {msg}"
    );
}
