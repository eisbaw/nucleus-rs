//! mp-tcp-bufsync multi-worker emit-string pins (TASK-0044.10 cycle 233).
//!
//! Host-EXCLUDING-barrier oracle — completes the 7-backend host-
//! excluding-barrier fast-emit-oracle family (openmp-rs, mp-tcp-poll
//! cycle 231; mp-tcp-event, mp-uds-event cycle 232; pthreads-async
//! TASK-0044.09; pthreads-sync covered pre-existing by its partial-
//! barrier fixture; mp-tcp-bufsync = this file).
//!
//! The hole: no pre-existing mp-tcp-bufsync test (check_frame_emit,
//! host_relay_emit, loop_body_w2w_push, pingpong, rendezvous_emit,
//! reuse_codegen_emit, wait_before_push) exercises a host-EXCLUDING
//! barrier or calls `apply_host_mediation_inject`. So a barrier-
//! participant off-by-one on the host-excluding shape — or a regression
//! that drops the mediation pass — would slip the fast bufsync emit
//! oracle and only bite the slow e2e gate.
//!
//! mp-tcp-bufsync's one-CTRL-stream-per-(host,worker) star topology
//! CANNOT lower a barrier that excludes the hub: `Plan::build` carries a
//! defensive `ContractGap` on any host-excluding barrier
//! (`backend_common::tcp_plan::Plan::build` "exclude the host worker"). The driver mediates by
//! running `apply_host_mediation_inject` (driver/src/main.rs host-
//! mediation gate — bufsync IS listed) which turns each host-excluding
//! barrier into a star-shaped N+1-party rendezvous through host. bufsync
//! is NOT in the `apply_host_data_relay_inject` gate (that DATA-arm
//! relay is mp-tcp-event / mp-uds-event only), so this oracle is
//! MEDIATION-ONLY — it mirrors the mp-tcp-poll cycle-231 template
//! `mp-tcp-poll/tests/multi_worker_emit.rs::
//! transpose_15_distributed_rows_poll_equiv_bufsync` (apply mediation,
//! NOT data-relay), and asserts the bufsync emit directly (pre/post-
//! mediation) the way the mp-tcp-event sibling oracle does (no cross-
//! backend equiv arm — that comparison is the poll test's job).

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above mp-tcp-bufsync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    // TASK-0426: per-call-unique subdir so concurrent test threads never
    // share a path (same shared-parent remove/create-vs-write race the
    // check_frame_emit.rs sibling hit; fixed proactively here).
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-multi-worker-emit-scratch");
    let _ = std::fs::create_dir_all(&target);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = target.join(format!("{name}-{}-{}", std::process::id(), nonce));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// 15-transpose/distributed-rows — the host-EXCLUDING-barrier coverage
/// arm. Its `xpose on {w0,w1,w2,w3}` placement (inner `j` loop
/// partitioned across the four compute workers) produces a genuinely
/// host-EXCLUDING inner barrier (participants `{w0,w1,w2,w3}`), which is
/// precisely the shape `apply_host_mediation_inject` exists to mediate
/// for the one-CTRL-stream-per-(host,worker) TCP star topology.
///
/// Why 15-transpose/distributed-rows and not 03-reduction/distributed:
/// 03-reduction/distributed's barriers BOTH already include host (host
/// owns load_input / combine), so `apply_host_mediation_inject` is a
/// complete no-op there and the test would be VACUOUS (the unmediated
/// emit would succeed and never exercise mediation). 15-transpose/
/// distributed-rows is the schedule that is BOTH genuinely host-
/// excluding AND a promoted `[[required]]` e2e cell on mp-tcp-bufsync
/// (see `nuc-nucleus/e2e-matrix.toml`). On this cell the UNMEDIATED
/// bufsync emit does not merely "lack" a host barrier — it FAILS to emit
/// at all (`Plan::build` returns a `ContractGap` on the host-excluding
/// barrier), which proves mediation is load-bearing rather than cosmetic.
///
/// Pass sequence MIRRORS the driver's mp-tcp-bufsync gate: the IR-stage
/// passes (parse..inject_transfers) PLUS an inline
/// `apply_host_mediation_inject`. The driver applies that CTRL-arm pass
/// for the {mp-tcp-bufsync, mp-tcp-event, mp-tcp-poll, mp-uds-event}
/// backends (driver/src/main.rs ~525) but does NOT apply
/// `apply_host_data_relay_inject` for bufsync (that DATA-arm relay is
/// mp-tcp-event / mp-uds-event only, driver/src/main.rs ~592), so this
/// test calls ONLY `apply_host_mediation_inject`. `inject_check_frames`
/// is omitted: 15-transpose/distributed-rows has no `check loop`
/// directive, so it would be a no-op.
///
/// Host election uses the shared helper
/// `backend_common::elect_host_from_name_workers`, mirroring the
/// driver's host-mediation gate exactly (TASK-0336 / memory
/// feedback-driver-must-mirror-backend-election-exactly).
#[test]
fn transpose_15_distributed_rows_host_excluding_barrier_bufsync() {
    use nucleus_compiler::{
        acfg_to_events,
        algo::{lower_algo, parse_algo},
        apply_block_transforms, apply_halo_inference_partition_aware, apply_host_mediation_inject,
        apply_partition_blocks2d, apply_partition_rows, apply_partition_workers,
        apply_reuse_inference, build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
        sched::{lower_sched, parse_sched},
        NameTables,
    };

    let scratch = scratch_dir("transpose_15_distributed_rows");
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/15-transpose");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/distributed-rows.sched.nuc")).unwrap();
    let kernels = ex.join("kernels.rs");

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

    // Host election: same shared helper the bufsync backend + the driver
    // use, derived from the UNMEDIATED projection (the `used` set the
    // election rule consumes). Done up-front so the participant-set
    // assertion below can reason about which WorkerId is host.
    let preview = acfg_to_events(&acfg);
    let used: std::collections::BTreeSet<_> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    let host = backend_common::elect_host_from_name_workers(&acfg.name_workers, &used)
        .expect("host election must succeed on 15-transpose/distributed-rows");

    // Pre-mediation (half 1) — the UNMEDIATED ACFG carries a host-
    // EXCLUDING barrier, so mp-tcp-bufsync's `Plan::build` REJECTS it
    // (its one-CTRL-stream-per-(host,worker) star cannot lower a barrier
    // that excludes the hub). This proves mediation is load-bearing: if
    // mediation were a no-op here (as on 03-reduction/distributed), this
    // emit would succeed and the oracle would be vacuous.
    let unmediated_pw = acfg_to_events(&acfg);
    let unmediated_sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar (unmediated)");
    let unmediated_names = NameTables::from_acfg(&acfg);
    let unmediated_emit = mp_tcp_bufsync::emit(
        &unmediated_pw,
        &unmediated_names,
        &unmediated_sidecar,
        &kernels,
        &scratch.join("unmediated"),
    );
    let err = unmediated_emit.expect_err(
        "15-transpose/distributed-rows: UNMEDIATED mp-tcp-bufsync emit MUST fail \
         (host-excluding barrier rejected by Plan::build); if it succeeds, the \
         mediation pass is a no-op and this oracle does not exercise mediation",
    );
    let err_text = format!("{err:?}");
    assert!(
        err_text.contains("exclude the host worker"),
        "15-transpose/distributed-rows: UNMEDIATED emit must fail with the \
         host-excluding-barrier ContractGap; got: {err_text}"
    );
    // The rejected barrier's participants are the 4 COMPUTE workers, with
    // host ABSENT. The ContractGap message renders `participants {parts:?}`
    // (a set of `WorkerId(n)`); assert host's WorkerId is NOT in it, and
    // every one of the four compute workers IS. Anchoring on the
    // participant SET — not a hardcoded barrier index (`#N`) — keeps the
    // assertion invariant under a future sync-tag renumber (forward-
    // carried lesson from TASK-0044.11 / cycle-232 reactor bid-hardcode).
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
             participant set must contain compute worker {w:?} (the four compute \
             workers {{w0,w1,w2,w3}} are the rejected participants). ContractGap: {err_text}"
        );
    }

    // Mediate (same host elected above), then re-project — the mediation
    // may add a Sync event to host's list, so the post-mediation
    // projection is the authoritative per_worker.
    let acfg = apply_host_mediation_inject(acfg, host);
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar (mediated)");
    let names = NameTables::from_acfg(&acfg);

    // Post-mediation (half 2) — the emit now SUCCEEDS, and host's bin
    // carries a `barrier_cross` for the formerly host-excluding barrier
    // (host became a participant of the mediated star rendezvous).
    let bufsync = mp_tcp_bufsync::emit(
        &per_worker,
        &names,
        &sidecar,
        &kernels,
        &scratch.join("gen"),
    )
    .expect("mediated mp-tcp-bufsync emit must succeed");

    let host_name = names.worker.get(&host).expect("host name").clone();
    let host_bin = bufsync
        .worker_bins
        .iter()
        .find(|p| p.file_name().unwrap().to_str().unwrap() == format!("{host_name}.rs"))
        .expect("host bin must be present in the mediated emit");
    let host_src = std::fs::read_to_string(host_bin).expect("read host bin");
    assert!(
        host_src.contains("wire::barrier_cross(&mut "),
        "15-transpose/distributed-rows: after host-mediation the host bin \
         ({host_name}.rs) MUST carry a `wire::barrier_cross` call — host now \
         participates in the mediated (formerly host-excluding) barrier. \
         host bin:\n{host_src}"
    );
}
