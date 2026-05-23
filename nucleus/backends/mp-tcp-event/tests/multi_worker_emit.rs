//! Multi-worker emit-path smoke tests for the mp-tcp-event backend
//! (TASK-0042.05 / Stage 3 of TASK-0042.02).
//!
//! What these tests pin:
//!
//! 1. `emit()` on a 2-worker fixture (02-split-add/split) returns Ok
//!    (no ContractGap on the multi-worker arm).
//! 2. The emitted runtime substrate (`src/runtime.rs`) is byte-
//!    identical to `mp_tcp_event::RUNTIME_SRC` — same single-source-
//!    of-truth precedent as `mp_tcp_common::WIRE_RUNTIME_SRC`.
//! 3. The emitted per-worker binary contains the expected runtime
//!    references: `mod runtime;`, `runtime::Reactor::new`, at least
//!    one `chan_<id>: runtime::Chan<...>` declaration, and at least
//!    one `chan_<id>.push(...)` or `chan_<id>.wait()` (depending on
//!    role).
//! 4. The emitted Cargo.toml declares the `mio = "0.8"` dependency
//!    with the `os-poll` + `net` features (PRD §12 "one well-known
//!    crate" allowance).
//! 5. The emitted run.sh sets up `NUC_RENDEZVOUS_DIR` + EXIT trap
//!    (rendezvous-file handshake post-TASK-0176, NOT the deleted
//!    `__nuc_pick_port` helper).
//!
//! Scope: codegen text + path predicates only. End-to-end build+run
//! against reference.bin is exercised by the e2e matrix (cycle 79
//! verified bit-identical sha256s on 02-split-add/split,
//! 11-game-of-life/pipelined, 13-cnn-inference/batch_parallel).

use std::path::PathBuf;

use mp_tcp_event::{emit, EmitResult, NameTables, RUNTIME_SRC};
use nucleus_compiler::sidecar::NameSidecar;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("three ancestors above mp-tcp-event crate")
        .to_path_buf()
}

/// 02-split-add/split: a 2-worker (host + w0) sync schedule. The
/// minimum surface that exercises the multi-worker emit path.
#[test]
fn multi_worker_emit_for_02_split_succeeds() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");

    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );

    let scratch = root.join("nucleus/target/mp-tcp-event-test-scratch/multi_worker_02_split");
    let _ = std::fs::remove_dir_all(&scratch);
    let result = emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &ex.join("kernels.rs"),
        &scratch,
    )
    .expect("multi-worker emit must succeed on 02-split-add/split");

    // (1) EmitResult shape: two worker bins (host + w0) + runtime_rs Some.
    assert_eq!(
        result.worker_bins.len(),
        2,
        "02-split-add/split has 2 used workers (host + w0); emit must \
         produce 2 per-worker binaries"
    );
    assert!(
        result.runtime_rs.is_some(),
        "multi-worker emit must populate runtime_rs"
    );

    // (2) The emitted runtime.rs is byte-identical to RUNTIME_SRC.
    let emitted_runtime =
        std::fs::read_to_string(result.runtime_rs.as_ref().unwrap()).expect("read runtime.rs");
    assert_eq!(
        emitted_runtime, RUNTIME_SRC,
        "src/runtime.rs must be byte-identical to mp_tcp_event::RUNTIME_SRC \
         (single source of truth; same precedent as WIRE_RUNTIME_SRC)"
    );

    // (3) The emitted host.rs contains the runtime references we
    //     expect from a multi-worker mp-tcp-event project.
    let host_rs = std::fs::read_to_string(
        result
            .worker_bins
            .iter()
            .find(|p| p.file_name().and_then(|s| s.to_str()) == Some("host.rs"))
            .expect("host.rs in worker_bins"),
    )
    .expect("read host.rs");
    assert!(
        host_rs.contains("#[path = \"../runtime.rs\"]"),
        "host.rs must `#[path]`-include the sibling runtime.rs"
    );
    assert!(
        host_rs.contains("mod runtime;"),
        "host.rs must declare `mod runtime;`"
    );
    assert!(
        host_rs.contains("runtime::Reactor::new"),
        "host.rs must construct the Reactor"
    );
    assert!(
        host_rs.contains("runtime::Chan::new"),
        "host.rs must construct at least one Chan<T>"
    );
    // 02-split-add has 3 cross-worker transfers (a host->w0, b host->w0,
    // c w0->host); host pushes a and b, waits on c.
    assert!(
        host_rs.contains(".push("),
        "host.rs must contain at least one Chan::push (host pushes a and b)"
    );
    assert!(
        host_rs.contains(".wait()"),
        "host.rs must contain at least one Chan::wait (host waits on c)"
    );

    // (4) Cargo.toml declares mio = "0.8" with os-poll + net features.
    let cargo = std::fs::read_to_string(&result.cargo_toml).expect("read Cargo.toml");
    assert!(
        cargo.contains("mio = "),
        "Cargo.toml must declare mio as a dependency:\n{cargo}"
    );
    assert!(
        cargo.contains("\"os-poll\""),
        "Cargo.toml must enable mio's os-poll feature"
    );
    assert!(
        cargo.contains("\"net\""),
        "Cargo.toml must enable mio's net feature"
    );

    // (5) run.sh sets up NUC_RENDEZVOUS_DIR + EXIT trap (rendezvous-
    //     file handshake post-TASK-0176, NOT the deleted helper).
    let run_sh = std::fs::read_to_string(&result.run_sh).expect("read run.sh");
    assert!(
        run_sh.contains("NUC_RENDEZVOUS_DIR="),
        "run.sh must export NUC_RENDEZVOUS_DIR (rendezvous-file handshake)"
    );
    assert!(
        run_sh.contains("mkdir -p \"$NUC_RENDEZVOUS_DIR\""),
        "run.sh must create the rendezvous dir"
    );
    assert!(
        run_sh.contains("trap 'rm -rf \"$NUC_RENDEZVOUS_DIR\"' EXIT"),
        "run.sh must install the EXIT trap to clean up the rendezvous dir"
    );
    // ABSENCE of the deleted __nuc_pick_port helper (TASK-0176; do
    // not reintroduce its close-then-rebind TOCTOU shape).
    assert!(
        !run_sh.contains("__nuc_pick_port"),
        "run.sh must NOT reintroduce __nuc_pick_port (TASK-0176 deleted it; \
         close-then-rebind TOCTOU)"
    );
    assert!(
        !run_sh.contains("pick_port"),
        "run.sh must NOT contain any pick_port helper (TASK-0176)"
    );
    assert!(
        !run_sh.contains("NUC_TCP_PORT_"),
        "run.sh must NOT use the pre-TASK-0176 NUC_TCP_PORT_* env-var handshake"
    );
}

/// `EmitResult` shape pin — compile-time only. If the struct changes,
/// this `let _r = EmitResult { ... }` won't compile and the driver
/// dispatch arm has to be updated in lockstep.
#[test]
fn emit_result_shape_is_six_fields() {
    let _r = EmitResult {
        project_dir: PathBuf::new(),
        cargo_toml: PathBuf::new(),
        worker_bins: Vec::new(),
        kernels_rs: PathBuf::new(),
        wire_rs: PathBuf::new(),
        runtime_rs: None,
        run_sh: PathBuf::new(),
    };
}

/// The runtime substrate is non-empty and contains the load-bearing
/// type names. Pinning these here means a regression that strips out
/// `Reactor` or `Chan<T>` fails the test before reaching e2e.
#[test]
fn runtime_src_contains_load_bearing_types() {
    assert!(
        RUNTIME_SRC.contains("pub struct Reactor"),
        "RUNTIME_SRC must declare pub struct Reactor"
    );
    assert!(
        RUNTIME_SRC.contains("pub struct Chan<T>"),
        "RUNTIME_SRC must declare pub struct Chan<T>"
    );
    assert!(
        RUNTIME_SRC.contains("mio::Poll"),
        "RUNTIME_SRC must reference mio::Poll (the reactor's substrate)"
    );
    assert!(
        RUNTIME_SRC.contains("HEADER_LEN"),
        "RUNTIME_SRC must declare/use HEADER_LEN (wire-protocol invariant)"
    );
}

/// Plan::build must reject schedules with host-excluding barriers
/// with a typed ContractGap forward-linking TASK-0175. Mirrors the
/// mp-tcp-bufsync test (same transport limit on the star topology).
///
/// Synthesised in-test (no in-tree mp-tcp-event fixture has a host-
/// excluding barrier today; the schedules that DO have one are
/// blocked by capability/projection at upstream layers) — drop the
/// per-worker EventList directly.
#[test]
fn host_excluding_barrier_is_typed_contract_gap() {
    use nucleus_compiler::event::{Event, SyncKind, SyncTag, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    let w0 = WorkerId(0); // host (named "host", below)
    let w1 = WorkerId(1);
    let w2 = WorkerId(2);
    let tag = SyncTag(0);

    // Barrier participants {w1, w2} — excludes w0 (the elected host).
    let parts: BTreeSet<WorkerId> = [w1, w2].into_iter().collect();
    let sync = Event::Sync {
        participants: parts,
        kind: SyncKind::Barrier,
        sync: tag,
    };

    // Give w0 a NON-empty event list so it's in used_workers — the
    // host election + host-excluding-barrier check both need w0 in
    // used_workers. A standalone barrier on w0 (no participants
    // exclude it) is enough.
    let host_only_marker = Event::Sync {
        participants: [w0].into_iter().collect(),
        kind: SyncKind::Barrier,
        sync: SyncTag(99),
    };
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(w0, vec![host_only_marker]);
    per_worker.insert(w1, vec![sync.clone()]);
    per_worker.insert(w2, vec![sync.clone()]);

    // Force host election to w0: insert a "host" name for it.
    let mut names = NameTables::default();
    names.worker.insert(w0, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    names.worker.insert(w2, "w2".to_string());
    let sidecar = NameSidecar::default();

    // Reach into the emit path via a temp dir. The kernels.rs path
    // must exist on disk; use the 02-split-add fixture's.
    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    let scratch =
        repo_root().join("nucleus/target/mp-tcp-event-test-scratch/host_excluding_barrier");
    let _ = std::fs::remove_dir_all(&scratch);

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    match r {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("exclude") && msg.contains("host"),
                "ContractGap message must name the host-excluding barrier rejection: {msg}"
            );
            assert!(
                msg.contains("TASK-0175"),
                "ContractGap message must forward-link TASK-0175: {msg}"
            );
        }
        Ok(_) => panic!("expected ContractGap on host-excluding barrier"),
    }
}
