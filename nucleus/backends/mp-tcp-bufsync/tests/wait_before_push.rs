//! mp-tcp-bufsync defensive ContractGap test for the wait-before-push
//! host-relay hazard (TASK-0332 cycle 151 AC#2).
//!
//! Mirror of `nucleus/backends/mp-tcp-event/tests/multi_worker_emit.rs::
//! wait_before_push_w2w_is_typed_contract_gap` per the cycle-148/149
//! paired-lift discipline ([[feedback-silent-sibling-defect]] 10th
//! firing): the same detector function `detect_wait_before_push_hazard`
//! exists in both backends; both must reject the same synthetic
//! fixture with the same shape of typed error.

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

/// TASK-0332 cycle 151 AC#2 (mp-tcp-bufsync sibling): defensive
/// ContractGap for the wait-before-push host-relay deadlock.
/// Cycle-148's synchronous host-relay (mp-tcp-bufsync slice of the
/// cycle-148/149 paired lift) would deadlock at runtime on this shape
/// — cycle 151 converts it to a codegen-time fail-loud rejection.
///
/// 3-worker fixture (host + w1 + w2): w1 and w2 both have w2w Push +
/// w2w Wait, and BOTH workers' first top-level w2w event is a Wait.
#[test]
fn wait_before_push_w2w_is_typed_contract_gap() {
    use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncKind, SyncTag, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let w2 = WorkerId(2);
    let data_a = DataId(0); // w1 -> w2
    let data_b = DataId(1); // w2 -> w1
    let seq_a = SeqTag(0);
    let seq_b = SeqTag(1);

    let parts_all: BTreeSet<WorkerId> = [w_host, w1, w2].into_iter().collect();
    let barrier = Event::Sync {
        participants: parts_all,
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(w_host, vec![barrier.clone()]);
    per_worker.insert(
        w1,
        vec![
            Event::Wait {
                src: w2,
                data: data_b,
                tile: IterTile::empty(),
                seq: seq_b,
            },
            Event::Push {
                dst: w2,
                data: data_a,
                tile: IterTile::empty(),
                seq: seq_a,
            },
            barrier.clone(),
        ],
    );
    per_worker.insert(
        w2,
        vec![
            Event::Wait {
                src: w1,
                data: data_a,
                tile: IterTile::empty(),
                seq: seq_a,
            },
            Event::Push {
                dst: w1,
                data: data_b,
                tile: IterTile::empty(),
                seq: seq_b,
            },
            barrier,
        ],
    );

    let mut names = NameTables::default();
    names.worker.insert(w_host, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    names.worker.insert(w2, "w2".to_string());
    names.data.insert(data_a, "a".to_string());
    names.data.insert(data_b, "b".to_string());
    let mut sidecar = NameSidecar::default();
    sidecar.transfer_buffer_for_seq.insert(seq_a, 1);
    sidecar.transfer_buffer_for_seq.insert(seq_b, 1);
    sidecar.data_types.insert(
        data_a,
        nucleus_compiler::algo::ResolvedType {
            scalar: nucleus_compiler::algo::ScalarType::I32,
            dims: vec![],
        },
    );
    sidecar.data_types.insert(
        data_b,
        nucleus_compiler::algo::ResolvedType {
            scalar: nucleus_compiler::algo::ScalarType::I32,
            dims: vec![],
        },
    );

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    let scratch =
        repo_root().join("nucleus/target/mp-tcp-bufsync-test-scratch/wait_before_push_hazard");
    let _ = std::fs::remove_dir_all(&scratch);

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    match r {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("mp-tcp-bufsync"),
                "ContractGap must name the backend prefix mp-tcp-bufsync: {msg}"
            );
            assert!(
                msg.contains("Wait") && msg.contains("Push"),
                "ContractGap must name both Wait and Push as the hazard \
                 components: {msg}"
            );
            assert!(
                msg.contains("TASK-0332"),
                "ContractGap must forward-link TASK-0332: {msg}"
            );
            assert!(
                msg.contains("host-relay") || msg.contains("circular"),
                "ContractGap must explain the deadlock mechanism: {msg}"
            );
        }
        Ok(_) => panic!(
            "expected ContractGap on wait-before-push hazard; cycle-151 \
             AC#2 defensive check was not triggered. Emit returned Ok — \
             the host-relay would silently deadlock at runtime."
        ),
    }
}

/// TASK-0332 cycle 151 AC#2 negative-path sanity (mp-tcp-bufsync
/// sibling): the pure-consumer shape (a worker with w2w Waits but NO
/// w2w Pushes) is SAFE under host-relay and must NOT trigger the new
/// defensive check. Mirror of the equivalent mp-tcp-event test.
#[test]
fn pure_consumer_wait_only_does_not_trigger_wait_before_push_check() {
    use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncKind, SyncTag, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1); // pure producer
    let w2 = WorkerId(2); // pure consumer (wait-only)
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

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(w_host, vec![barrier_1.clone(), barrier_2.clone()]);
    per_worker.insert(
        w1,
        vec![
            Event::Push {
                dst: w2,
                data,
                tile: IterTile::empty(),
                seq,
            },
            barrier_1.clone(),
            barrier_2.clone(),
        ],
    );
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
    names.data.insert(data, "data".to_string());
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
    let scratch =
        repo_root().join("nucleus/target/mp-tcp-bufsync-test-scratch/pure_consumer_wait_only");
    let _ = std::fs::remove_dir_all(&scratch);

    emit(&per_worker, &names, &sidecar, &kernels, &scratch).expect(
        "pure-consumer wait-only worker (no w2w Push) must NOT trigger the \
         cycle-151 AC#2 wait-before-push check — host's relay does not wait \
         FOR a wait-only worker because it's not a src in relay_schedule",
    );
}
