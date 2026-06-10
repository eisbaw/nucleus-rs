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
use nucleus_compiler::sidecar::{NameSidecar, XferFacts};

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

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &root.join("nucleus/target/mp-tcp-event-test-scratch"),
        "multi_worker_02_split",
    );
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
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-event-test-scratch"),
        "host_excluding_barrier",
    );

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

/// 15-transpose/distributed-rows — the host-EXCLUDING-barrier emit
/// oracle on a REAL `[[required]]` cell (TASK-0044.08, cycle 232).
///
/// Why this exists alongside `host_excluding_barrier_is_typed_contract_gap`
/// above: that test hand-builds a SYNTHETIC per-worker EventList to drive
/// the `Plan::build` rejection branch. This test instead lowers the REAL
/// 15-transpose/distributed-rows schedule (a promoted `[[required]]`
/// cell on all 7 tier-1 backends), so the ACFG that reaches `emit()` is
/// the production one, and it exercises BOTH halves of the mediation
/// contract on a single fixture:
///   (a) the UNMEDIATED ACFG carries a genuinely host-EXCLUDING barrier
///       (participants `{w0,w1,w2,w3}`, host absent; its SyncTag bid is
///       DERIVED at runtime, not pinned — TASK-0044.11), so `Plan::build`
///       REJECTS it with the `ContractGap("... exclude the host
///       worker ...")` — proving the mediation pass is load-bearing,
///       not cosmetic; and
///   (b) after `apply_host_mediation_inject` (the CTRL arm) the emit
///       SUCCEEDS and host's bin carries the barrier-shim for the
///       formerly-host-excluding barrier — host became a participant.
///
/// PROVENANCE (TASK-0044.08): sibling of TASK-0044.01.03 (openmp-rs) +
/// TASK-0044.02.02.01 (mp-tcp-poll), both closed cycle 231. Those two
/// retargeted from the (empirically-false-premise) 03-reduction/
/// distributed to 15-transpose/distributed-rows after a scan found it
/// is the ONLY schedule that is BOTH genuinely host-excluding AND a
/// promoted `[[required]]` e2e cell on these backends. mp-tcp-event +
/// mp-uds-event are the last two backends of this family.
///
/// Pass sequence MIRRORS the driver's mp-tcp-event gate EXACTLY: the
/// IR-stage passes (parse..inject_transfers, driver/src/main.rs
/// ~460-487) PLUS `apply_host_mediation_inject` (gate ~525-563,
/// mp-tcp-event in the set) PLUS `apply_host_data_relay_inject` (gate
/// ~592-614, mp-tcp-event in the set). This is the DIFFERENCE from the
/// mp-tcp-poll sibling oracle, which calls ONLY mediation (poll is NOT
/// in the data-relay gate). On 15-transpose/distributed-rows the
/// data-relay pass is a no-op (no in-`Repeat`-body non-host↔non-host
/// Push pair) but we apply it to keep the inline ACFG byte-identical
/// to what the driver feeds `mp_tcp_event::emit` in production — a
/// divergent inline sequence would make this oracle a different ACFG
/// than production (silent false confidence).
///
/// No cross-backend byte-equivalence arm: mp-tcp-event's true
/// structural twin is mp-uds-event, but a `mp-uds-event` dev-dep here
/// would be CIRCULAR (mp-uds-event already dev-deps mp-tcp-event for
/// the reverse oracle, `separable_filter_06_distributed2_uds_equiv_tcp`).
/// mp-tcp-bufsync is the sync sibling, NOT a structural twin (its
/// buffered-sync emit differs from the async reactor shape). So the
/// bite here is the pre-mediation `expect_err`, then post-mediation
/// `Ok`, then an anchored host-bin barrier-shim marker — NOT a generic
/// `contains("barrier_cross")` (host already crosses the host-INCLUDED
/// barriers, so a bare substring would pass even if mediation were a
/// no-op — the vacuity trap the architect flagged on the cycle-231
/// sibling tasks).
#[test]
fn transpose_15_distributed_rows_event_host_excluding_barrier_mediated() {
    use nucleus_compiler::{
        acfg_to_events,
        algo::{lower_algo, parse_algo},
        apply_block_transforms, apply_halo_inference_partition_aware, apply_host_data_relay_inject,
        apply_host_mediation_inject, apply_partition_blocks2d, apply_partition_rows,
        apply_partition_workers, apply_reuse_inference, build_acfg, build_sidecar, inject_syncs,
        inject_transfers, link,
        sched::{lower_sched, parse_sched},
        NameTables,
    };

    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/15-transpose");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/distributed-rows.sched.nuc")).unwrap();
    let kernels = ex.join("kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    // The `unmediated` / `mediated` subdirs below stay under this now-
    // unique parent, so they need no per-subdir uniqueness of their own.
    let scratch = test_common::unique_scratch_dir(
        &root.join("nucleus/target/mp-tcp-event-test-scratch"),
        "transpose_15_distributed_rows",
    );

    let algo_ast = parse_algo(&algo_src).expect("algo parse");
    let sched_ast = parse_sched(&sched_src).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block transforms");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows");
    let acfg = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d");
    let (acfg, _advisory) =
        apply_halo_inference_partition_aware(&linked, acfg).expect("halo inference");
    let acfg = apply_reuse_inference(&linked, acfg).expect("reuse inference");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    // Project the UNMEDIATED ACFG once; it drives BOTH the host election
    // (the `used` set the election rule consumes) AND the host-excluding
    // barrier bid derivation, so the anchors below are DERIVED rather than
    // hardcoded (TASK-0044.11; was WorkerId(1..4) literals + bid=2 —
    // architect P3-1/P3-2 on TASK-0044.08).
    let unmediated_pw = acfg_to_events(&acfg);
    let used: std::collections::BTreeSet<_> = unmediated_pw
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    // Host election: shared helper the backend + driver use (memory
    // feedback-driver-must-mirror-backend-election-exactly). Done up-front
    // so the pre-mediation participant-set assertion can reason about
    // which WorkerId is host.
    let host = backend_common::elect_host_from_name_workers(&acfg.name_workers, &used)
        .expect("host election must succeed on 15-transpose/distributed-rows");
    // The host-EXCLUDING barrier's bid is the SyncTag whose participant
    // set lacks host; `worker_program.rs::emit_barrier_shims` emits
    // `Bar{bid}` / `let bar_{bid}` / `barrier_cross(_, bid)` with
    // bid == SyncTag.0. Derive it from the UNMEDIATED projection
    // (post-mediation the set includes host, so no barrier is
    // host-excluding — it must be read before mediation).
    let host_excluding_bid = test_common::host_excluding_barrier_bid(&unmediated_pw, host).expect(
        "15-transpose/distributed-rows must carry a host-excluding barrier \
         (its bid drives the post-mediation Bar{bid} anchor)",
    );

    // ---- Half 1: the UNMEDIATED ACFG is REJECTED. ----
    // 15-transpose/distributed-rows places `xpose on {w0,w1,w2,w3}`,
    // producing a host-EXCLUDING inner compute barrier. mp-tcp-event's
    // `Plan::build` (multi_worker/mod.rs:273-284) cannot lower a
    // barrier that excludes the hub of its one-CTRL-stream-per-(host,
    // worker) star, so the unmediated emit MUST fail. If it succeeded,
    // the mediation pass would be a no-op here and this oracle would
    // not exercise mediation (the same vacuity that sank the original
    // 03-reduction premise — see provenance above).
    let unmediated_sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar (unmediated)");
    let unmediated_names = NameTables::from_acfg(&acfg);
    let unmediated_emit = emit(
        &unmediated_pw,
        &unmediated_names,
        &unmediated_sidecar,
        &kernels,
        &scratch.join("unmediated"),
    );
    let err = unmediated_emit.expect_err(
        "15-transpose/distributed-rows: UNMEDIATED mp-tcp-event emit MUST fail \
         (host-excluding barrier rejected by Plan::build); if it succeeds, the \
         mediation pass is a no-op and this oracle does not exercise mediation",
    );
    let err_text = format!("{err:?}");
    assert!(
        err_text.contains("exclude the host worker"),
        "15-transpose/distributed-rows: UNMEDIATED emit must fail with the \
         host-excluding-barrier ContractGap; got: {err_text}"
    );
    // Anchor to the SPECIFIC host-excluding barrier by PARTICIPANT SET,
    // not hardcoded WorkerId literals (TASK-0044.11, mirroring the
    // cycle-233 bufsync sibling): host must be ABSENT from the rejected
    // barrier's participants, and every other used (compute) worker must
    // be present. A regression that mediated host into the participant
    // set upstream would put host in the rendered set and trip the first
    // assert; a renumber of the compute WorkerIds is tracked
    // automatically because the anchors are derived, not literal.
    assert!(
        !err_text.contains(&format!("{host:?}")),
        "15-transpose/distributed-rows: the rejected host-excluding barrier's \
         participant set must NOT contain host ({host:?}); the whole point is \
         host is excluded. ContractGap: {err_text}"
    );
    for w in &used {
        if *w == host {
            continue;
        }
        assert!(
            err_text.contains(&format!("{w:?}")),
            "15-transpose/distributed-rows: the rejected host-excluding barrier's \
             participant set must contain compute worker {w:?} (the compute \
             workers are the rejected participants). ContractGap: {err_text}"
        );
    }

    // ---- Mediate: SAME host election the backend uses (elected above
    // from the unmediated projection, mirroring the driver's
    // host-mediation gate — memory
    // feedback-driver-must-mirror-backend-election-exactly: a compiler
    // pass mediating against a backend-elected host MUST use the
    // identical election rule). ----
    let acfg = apply_host_mediation_inject(acfg, host);
    // Data-relay arm: the mp-tcp-event driver gate applies this in
    // ADDITION to mediation (driver/src/main.rs ~592-614); the poll
    // gate does NOT. A no-op on this schedule (no in-Repeat-body w↔w
    // Push pair) but applied for production-ACFG fidelity.
    let acfg = apply_host_data_relay_inject(acfg, host);

    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);

    // ---- Half 2: after mediation the emit SUCCEEDS. ----
    let result = emit(
        &per_worker,
        &names,
        &sidecar,
        &kernels,
        &scratch.join("mediated"),
    )
    .expect("post-mediation mp-tcp-event emit must succeed");

    let host_name = names.worker.get(&host).expect("host name").clone();
    let host_bin = result
        .worker_bins
        .iter()
        .find(|p| {
            p.file_name().and_then(|s| s.to_str()) == Some(format!("{host_name}.rs").as_str())
        })
        .expect("host bin must be present in the mediated emit");
    let host_src = std::fs::read_to_string(host_bin).expect("read host bin");

    // BITING anchor: pre-mediation host did NOT participate in the
    // host-excluding barrier (bid `host_excluding_bid`, DERIVED above from
    // the unmediated participant set), so its bin carried NO `Bar{bid}`
    // shim. After mediation host IS a participant, so its bin MUST now
    // declare `struct Bar{bid}` + `let bar_{bid} = Bar{bid}` (the
    // per-barrier shim, see worker_program.rs::emit_barrier_shims). This
    // is host-specific and barrier-specific: it cannot be satisfied by
    // the host-INCLUDED barriers' shims (different bid), so it BITES where
    // a bare `contains("barrier_cross")` would pass vacuously. The bid is
    // DERIVED (TASK-0044.11) so a sync-tag renumber re-targets the anchor
    // instead of silent-passing on a host-included barrier that happened
    // to inherit the old literal bid.
    let bar_struct = format!("struct Bar{host_excluding_bid} {{");
    let bar_let = format!("let bar_{host_excluding_bid} = Bar{host_excluding_bid} {{");
    assert!(
        host_src.contains(&bar_struct) && host_src.contains(&bar_let),
        "15-transpose/distributed-rows: after host-mediation the host bin \
         ({host_name}.rs) MUST declare the barrier shim for the formerly \
         host-EXCLUDING barrier (bid {host_excluding_bid}: `{bar_struct}` / \
         `{bar_let}`); its absence means mediation did not add host to that \
         barrier. host bin:\n{host_src}"
    );
    // The mediated shim must cross host with EVERY compute worker (host is
    // now the hub of the formerly host-excluding barrier). Derive both the
    // peer NAME and the bid rather than hardcoding `ctrl_w0 .. 2`.
    for w in &used {
        if *w == host {
            continue;
        }
        let wn = names.worker.get(w).expect("compute worker name");
        let cross =
            format!("wire::barrier_cross(&mut *self.ctrl_{wn}.borrow_mut(), {host_excluding_bid})");
        assert!(
            host_src.contains(&cross),
            "15-transpose/distributed-rows: host's Bar{host_excluding_bid} shim \
             must cross compute worker {wn} (bid {host_excluding_bid}) — the \
             mediated, formerly host-excluding barrier. Expected `{cross}`. \
             host bin:\n{host_src}"
        );
    }
}

// --------------------------------------------------------------------
// TASK-0255 — negative-path coverage for the OTHER 4 typed-ContractGap
// branches landed in cycle 79 (host-excluding-barrier above was the
// only one with a test). Forward-carried from architect F10 of
// TASK-0042.05.
//
// Pattern is the same as host_excluding_barrier_is_typed_contract_gap:
// hand-build per_worker / NameTables / NameSidecar fixtures and assert
// the matching EmitError::ContractGap message contains a stable
// substring (+ the TASK-NNNN forward-link where the production message
// carries one — Branches C and D).
//
// Branch A (used_workers.len() < 2) is NOT exercisable from emit()
// because lib.rs:290 routes single-worker input to the single-worker
// arm BEFORE Plan::build is called. Its test lives as a
// `#[cfg(test)] mod tests` inside src/multi_worker.rs so it can call
// the pub(crate) Plan::build directly.
// --------------------------------------------------------------------

/// Branch B (search in `multi_worker.rs` for the `"Wait but no
/// matching Push"` ContractGap inside `Plan::build`; symbolic anchor
/// since cycle 130 shortened the local layout — TASK-0300 hoist) —
/// `pair_tiles` is populated from BOTH Push and Wait (see
/// `collect_xfer_pairs` / its shared wrapper `collect_pair_tiles`), but
/// `chan_pairs` is populated ONLY from Push. So a worker carrying a
/// `Wait` with no peer's `Push` producing the same `(DataId, SeqTag)`
/// triggers the defensive "Wait but no matching Push — malformed
/// projection" check.
///
/// This branch is normally unreachable from valid projections (the
/// `transfer_inject` pass guarantees matched pairs); the test pins it
/// so a regression in `transfer_inject` or `collect_xfer_pairs` does
/// not silently route a malformed projection to a deadlocked binary.
#[test]
fn wait_without_matching_push_is_typed_contract_gap() {
    use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, WorkerId};
    use std::collections::BTreeMap;

    use nucleus_compiler::event::{SyncKind, SyncTag};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let data = DataId(0);
    let seq = SeqTag(0);

    // w1 carries a Wait expecting data from host, but host has NO
    // Push event producing this (data, seq). `pair_tiles` will pick
    // up the Wait (via collect_xfer_pairs, which scans BOTH Push and
    // Wait) but `chan_pairs` (which only scans Push) will not —
    // triggering Branch B.
    //
    // The Branch-B check fires BEFORE the barrier topology check and
    // the chan_pairs topology check, so we don't have to be careful
    // about the barrier participants here — but we DO need both
    // workers non-empty so `used_workers.len() >= 2` (else Branch A
    // would fire first). Give host a host-only Sync as a non-empty
    // marker; the Branch-A check is the only earlier gate and we're
    // past it.
    let host_marker = Event::Sync {
        participants: [w_host].into_iter().collect(),
        kind: SyncKind::Barrier,
        sync: SyncTag(42),
    };
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(w_host, vec![host_marker]);
    per_worker.insert(
        w1,
        vec![Event::Wait {
            src: w_host,
            data,
            tile: IterTile::empty(),
            seq,
        }],
    );

    let mut names = NameTables::default();
    names.worker.insert(w_host, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    let sidecar = NameSidecar::default();

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-event-test-scratch"),
        "wait_without_matching_push",
    );

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    match r {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("Wait") && msg.contains("no matching Push"),
                "ContractGap must name the Wait-without-Push rejection: {msg}"
            );
            assert!(
                msg.contains("malformed projection"),
                "ContractGap must include the 'malformed projection' diagnosis: {msg}"
            );
        }
        Ok(_) => panic!("expected ContractGap on Wait without matching Push"),
    }
}

/// Branch C (multi_worker.rs:174-186) — every cross-worker
/// `(DataId, SeqTag)` pair must have an `xfer_facts[seq]` entry in the
/// sidecar (TASK-0233 buffer contract, unified into `XferFacts` by
/// TASK-0455.08). Missing entry = ContractGap forward-linking TASK-0233.
///
/// Mirrors `pthreads-async`'s `build_fails_on_missing_sidecar_buffer_entry`
/// (in `pthreads-async/src/multi_worker.rs`) — same shape, same TASK-0233
/// forward-link, but exercises the mp-tcp-event Plan::build instead of the
/// pthreads-async one (the two share the contract but each owns its own check).
#[test]
fn missing_sidecar_buffer_for_seq_is_typed_contract_gap() {
    use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncKind, SyncTag, WorkerId};
    use std::collections::BTreeMap;

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let data = DataId(0);
    let seq = SeqTag(0);

    // host pushes; w1 waits. Both worker events form a VALID
    // Push/Wait pair (chan_pairs lookup will succeed). The ONLY
    // missing piece is the sidecar's xfer_facts entry
    // for `seq` — Plan::build must catch this and forward-link
    // TASK-0233 rather than default-size and silently produce a
    // runtime mismatch.
    //
    // Both workers also carry an inclusive barrier so the
    // host-excluding-barrier check passes (every barrier must include
    // host).
    let parts: std::collections::BTreeSet<WorkerId> = [w_host, w1].into_iter().collect();
    let barrier = Event::Sync {
        participants: parts,
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };

    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(
        w_host,
        vec![
            Event::Push {
                dst: w1,
                data,
                tile: IterTile::empty(),
                seq,
            },
            barrier.clone(),
        ],
    );
    per_worker.insert(
        w1,
        vec![
            Event::Wait {
                src: w_host,
                data,
                tile: IterTile::empty(),
                seq,
            },
            barrier,
        ],
    );

    let mut names = NameTables::default();
    names.worker.insert(w_host, "host".to_string());
    names.worker.insert(w1, "w1".to_string());
    // Deliberately empty sidecar — no xfer_facts[seq].
    let sidecar = NameSidecar::default();

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-event-test-scratch"),
        "missing_sidecar_buffer",
    );

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    match r {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("xfer_facts"),
                "ContractGap must name the missing sidecar field: {msg}"
            );
            assert!(
                msg.contains("TASK-0233"),
                "ContractGap must forward-link TASK-0233 (the sidecar-buffer contract): {msg}"
            );
        }
        Ok(_) => panic!("expected ContractGap on missing xfer_facts entry"),
    }
}

/// TASK-0327 (cycle 149) — worker-to-worker `Push`/`Wait` is now
/// lowered via HOST-RELAY: HOST's `main()` contains an inline relay
/// phase that calls `Reactor::relay_one(seq, dst_peer, cap)` per hop;
/// src worker pushes via peer_idx=0 (host); dst worker waits via its
/// `chan_<rid>.wait()` (reads `inbound[seq]` populated by the host's
/// forwarded frame). What used to be Branch D's ContractGap rejection
/// (cycle 79 — `worker-to-worker_push_is_typed_contract_gap`, deleted
/// cycle 149) now succeeds; this test pins the positive emit surface
/// so a regression that silently dropped the relay block, or routed
/// via the wrong dst_peer index, fails here before reaching the e2e
/// matrix.
///
/// Mirrors mp-tcp-bufsync's cycle-148 host-relay positive fixtures
/// in `nucleus/backends/mp-tcp-bufsync/tests/host_relay_emit.rs`.
#[test]
fn worker_to_worker_push_emits_host_relay() {
    use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncKind, SyncTag, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    let w_host = WorkerId(0);
    let w1 = WorkerId(1);
    let w2 = WorkerId(2);
    let data = DataId(0);
    let seq = SeqTag(0);

    // The previously-OFFENDING Push is w1 -> w2 (neither endpoint is
    // host). Host is in used_workers (smallest WorkerId + named
    // "host"), the lone barrier participates ALL three workers (so
    // the host-excluding-barrier check stays inert). Two top-level
    // Syncs on host so the relay-phase insertion point splices the
    // relay between them (matches the 06/distributed2 reproducer
    // shape: pass-1 barrier -> RELAY -> pass-2 barrier).
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
    // host: two top-level barriers, no Push/Wait. The cycle-149 relay
    // splice inserts the relay block right BEFORE the LAST top-level
    // Sync (= barrier_2 here), giving the layout
    //   [pre: barrier_1] [relay block] [post: barrier_2]
    // on host. matches the cycle-148 mp-tcp-bufsync test fixture
    // shape.
    per_worker.insert(w_host, vec![barrier_1.clone(), barrier_2.clone()]);
    // w1: Pushes to w2 (the worker-to-worker shape lifted in cycle
    // 149) and crosses both barriers.
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
    // w2: Waits from w1 and crosses both barriers.
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
    // Cycle 149: relay block emits `// relay \`<data_name>\` from ...`
    // and `Plan::render_relay_phase` propagates EmitError::ContractGap
    // when the DataId has no name (mp-tcp-bufsync cycle-148 P2.2
    // discipline). Without a data name, the test would fail with
    // `data id DataId(0) has no name in NameTables` instead of
    // exercising the positive relay surface.
    names.data.insert(data, "tmp".to_string());
    let mut sidecar = NameSidecar::default();
    // Branch C must NOT trip: provide the sidecar entry.
    sidecar.xfer_facts.insert(
        seq,
        XferFacts {
            buffer: 1,
            ..Default::default()
        },
    );
    // Chan::new (emit_reactor_and_chans) needs the ResolvedType to
    // pick the right encode/decode fn pair. i32 scalar is enough for
    // a synthetic 1-byte-payload-equivalent test fixture (the relay
    // itself is bytes-verbatim; the type only affects the encode/
    // decode wiring on src and dst, which we assert exists below).
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
        &repo_root().join("nucleus/target/mp-tcp-event-test-scratch"),
        "worker_to_worker_push",
    );

    let result = emit(&per_worker, &names, &sidecar, &kernels, &scratch)
        .expect("cycle-149 host-relay lifts the cycle-79 w2w ContractGap; emit must Ok");

    // ---- host main.rs contains the relay block. ----
    let host_bin = result
        .worker_bins
        .iter()
        .find(|p| p.file_name().and_then(|s| s.to_str()) == Some("host.rs"))
        .expect("host.rs must be emitted (host is in used_workers)");
    let host_src = std::fs::read_to_string(host_bin).expect("read host.rs");
    assert!(
        host_src.contains("TASK-0327 host-relay phase"),
        "host main.rs must contain the TASK-0327 host-relay banner: {host_src}"
    );
    // dst_peer index for w2 in host's non_host_workers list = position
    // of w2 in [w1, w2] = 1. seq=0, cap=1 (from sidecar).
    assert!(
        host_src.contains("__relay.relay_one(0u64, 1usize, 1usize)"),
        "host main.rs must call relay_one(seq=0, dst_peer=1, cap=1) for the \
         w1->w2 hop: {host_src}"
    );
    assert!(
        host_src.contains("// relay `tmp` from w1 to w2"),
        "host main.rs must carry the disambiguating relay comment naming \
         the data + src + dst: {host_src}"
    );
    // Splice between barriers: relay banner appears AFTER the first
    // barrier crossing and BEFORE the second.
    let banner_pos = host_src
        .find("TASK-0327 host-relay phase")
        .expect("banner present");
    let bar0_pos = host_src
        .find("bar_0.wait()")
        .expect("first barrier wait present");
    let bar1_pos = host_src
        .find("bar_1.wait()")
        .expect("second barrier wait present");
    assert!(
        bar0_pos < banner_pos && banner_pos < bar1_pos,
        "relay banner must splice BETWEEN bar_0.wait() (pre) and \
         bar_1.wait() (post); got bar0@{bar0_pos}, banner@{banner_pos}, \
         bar1@{bar1_pos}"
    );

    // ---- src worker (w1) pushes via peer_idx=0 (host). ----
    let w1_bin = result
        .worker_bins
        .iter()
        .find(|p| p.file_name().and_then(|s| s.to_str()) == Some("w1.rs"))
        .expect("w1.rs must be emitted");
    let w1_src = std::fs::read_to_string(w1_bin).expect("read w1.rs");
    // Chan::new(reactor, seq=0, peer_idx=0, cap=1, encode, decode).
    assert!(
        w1_src.contains("runtime::Chan::new(") && w1_src.contains("0u64, 0usize, 1usize,"),
        "w1.rs must build chan with peer_idx=0 (host-relay route): {w1_src}"
    );
    assert!(
        w1_src.contains("chan_0.push("),
        "w1.rs must call chan_0.push for the w1->w2 hop: {w1_src}"
    );

    // ---- dst worker (w2) waits via chan_0.wait(). ----
    let w2_bin = result
        .worker_bins
        .iter()
        .find(|p| p.file_name().and_then(|s| s.to_str()) == Some("w2.rs"))
        .expect("w2.rs must be emitted");
    let w2_src = std::fs::read_to_string(w2_bin).expect("read w2.rs");
    assert!(
        w2_src.contains("chan_0.wait()"),
        "w2.rs must call chan_0.wait() for the w1->w2 hop: {w2_src}"
    );
}

/// TASK-0332 cycle 151 AC#2: defensive ContractGap for the
/// wait-before-push host-relay deadlock. Synthetic 3-worker fixture
/// (host + w1 + w2) where w1 AND w2 both have w2w Push + w2w Wait,
/// and BOTH workers' first top-level w2w event is a Wait. Cycle-149's
/// host-relay would deadlock on this shape at runtime (cycle-150
/// empirical reproducer in-tree: 05-stencil/distributed-2d ×
/// mp-tcp-event). This test pins the codegen-time fail-loud
/// rejection so a regression that re-introduced silent runtime
/// deadlock fails here.
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
    // host: just a barrier (so it's in used_workers + elected host).
    per_worker.insert(w_host, vec![barrier.clone()]);
    // w1: Wait FROM w2 BEFORE Push TO w2 (wait-before-push hazard).
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
    // w2: symmetric — Wait FROM w1 BEFORE Push TO w1.
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
    sidecar.xfer_facts.insert(
        seq_a,
        XferFacts {
            buffer: 1,
            ..Default::default()
        },
    );
    sidecar.xfer_facts.insert(
        seq_b,
        XferFacts {
            buffer: 1,
            ..Default::default()
        },
    );
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
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/mp-tcp-event-test-scratch"),
        "wait_before_push_hazard",
    );

    let r = emit(&per_worker, &names, &sidecar, &kernels, &scratch);
    match r {
        Err(e) => {
            let msg = format!("{e}");
            // Cycle-151 architect P3 fold-back: parenthesize the
            // operator-precedence-fragile assertion (Rust parses as
            // `A || (B && C)`; correct but a future refactor that
            // deletes the dashed phrase clause would silently change
            // the assertion's bind).
            assert!(
                msg.contains("wait-before-push") || (msg.contains("Wait") && msg.contains("Push")),
                "ContractGap must name the wait-before-push hazard: {msg}"
            );
            // Cycle-151 architect P3 fold-back: pin the backend-prefix
            // (the whole point of the per-backend duplication is the
            // per-backend message prefix). Symmetric with the
            // mp-tcp-bufsync sibling test.
            assert!(
                msg.contains("mp-tcp-event"),
                "ContractGap must name the backend prefix mp-tcp-event: {msg}"
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

/// TASK-0332 cycle 151 AC#2 — negative-path sanity: the pure-consumer
/// shape (a worker with w2w Waits but NO w2w Pushes) is SAFE under
/// host-relay and must NOT trigger the new defensive check. This is
/// the same shape exercised by `worker_to_worker_push_emits_host_relay`
/// above (where w2 is the wait-only consumer); pinning it as a
/// dedicated negative-path test so a future tightening of the
/// detector that drops the `has_w2w_push` precondition fails here.
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
    // w2: pure consumer — Wait first, no Push, then barriers.
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
    sidecar.xfer_facts.insert(
        seq,
        XferFacts {
            buffer: 1,
            ..Default::default()
        },
    );
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
        &repo_root().join("nucleus/target/mp-tcp-event-test-scratch"),
        "pure_consumer_wait_only",
    );

    emit(&per_worker, &names, &sidecar, &kernels, &scratch).expect(
        "pure-consumer wait-only worker (no w2w Push) must NOT trigger the \
         cycle-151 AC#2 wait-before-push check — host's relay does not wait \
         FOR a wait-only worker because it's not a src in relay_schedule",
    );
}
