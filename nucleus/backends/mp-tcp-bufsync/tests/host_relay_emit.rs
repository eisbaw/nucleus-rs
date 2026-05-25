//! mp-tcp-bufsync host-relay emit pinning (TASK-0327 cycle 148).
//!
//! Why this file exists: cycle 148 lifted the prior
//! `data_conn_var` fail-loud rejection of non-host peers — when a
//! non-host worker's Push/Wait names another non-host worker, the
//! worker writes/reads its existing `data_host` connection and HOST
//! runs a SYNCHRONOUS RELAY PHASE that drains each (seq, dst) hop
//! from `data_<src>` and forwards it to `data_<dst>`.
//!
//! This test pins the relay-phase emit shape so a regression that
//! silently drops the relay (e.g. an over-zealous refactor of
//! `render_relay_phase` or a wrong `relay_phase_insertion_point`
//! heuristic for the 06/distributed2 schedule shape) fails LOUD
//! here, before the e2e cell runs.
//!
//! Scope: codegen text only. End-to-end build + run is exercised by
//! the e2e matrix's `06-separable-filter/distributed2 × mp-tcp-bufsync`
//! cell (cycle-148 promoted from [[skip]] to [[required]]).

use std::fs;
use std::path::{Path, PathBuf};

use nucleus_compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    apply_block_transforms, apply_halo_inference_partition_aware, apply_partition_blocks2d,
    apply_partition_rows, apply_partition_workers, apply_reuse_inference, build_acfg,
    build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
use pthreads_sync::NameTables;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above mp-tcp-bufsync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-host-relay-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Compile the in-tree 06-separable-filter / distributed2 schedule
/// through the mp-tcp-bufsync backend, returning `host.rs` and `w0.rs`
/// as Strings.
fn emit_06_distributed2(scratch: &Path) -> (String, String) {
    let root = repo_root();
    let algo_src = fs::read_to_string(
        root.join("nuc-nucleus/examples/06-separable-filter/prog.algo.nuc"),
    )
    .expect("read prog.algo.nuc");
    let sched_src = fs::read_to_string(
        root.join("nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc"),
    )
    .expect("read distributed2.sched.nuc");
    let kernels_src =
        fs::read_to_string(root.join("nuc-nucleus/examples/06-separable-filter/kernels.rs"))
            .expect("read kernels.rs");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&kernels_path, kernels_src).expect("write kernels stub");

    // Mirror the driver pass sequence (driver/src/main.rs ~lines 308-455):
    // partition-rows passes are load-bearing for 06/distributed2's
    // partition=rows on hy/vy — without them, transfer_inject sees
    // single-worker scope and the cross-tmp fan-out + the cycle-148
    // host-relay phase never get emitted.
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
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);

    let out_dir = scratch.join("gen");
    let _result = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-bufsync emit");
    let host_src = fs::read_to_string(out_dir.join("src/bin/host.rs")).expect("read host.rs");
    let w0_src = fs::read_to_string(out_dir.join("src/bin/w0.rs")).expect("read w0.rs");
    (host_src, w0_src)
}

/// Cycle-148 AC#1 / AC#3 (mp-tcp-bufsync slice): the host main MUST
/// include the synchronous relay phase, with exactly 12 hops
/// (N*(N-1) for N=4) under the `06/distributed2` schedule, srcs in
/// sorted-WorkerId order (data_w0, data_w1, data_w2, data_w3) — the
/// alignment relied on for seq-tag correctness in the deadlock-free
/// run (see `relay_phase_insertion_point` doc).
#[test]
fn host_emit_includes_task_0327_relay_phase_with_12_hops() {
    let scratch = scratch_dir("relay_phase_shape");
    let (host_src, _w0_src) = emit_06_distributed2(&scratch);

    // Header marker — the cycle-148 explainer block above the hops.
    assert!(
        host_src.contains("// TASK-0327 host-relay phase:"),
        "host.rs must carry the cycle-148 relay-phase header marker. \
         Got:\n{host_src}"
    );

    // Exactly 12 hops (4*3 = N*(N-1) for the 06/distributed2 4-worker
    // shape). Greppable witness: every hop line carries the marker
    // `relay \`tmp\` from`.
    let hop_count = host_src.matches("relay `tmp` from").count();
    assert_eq!(
        hop_count, 12,
        "host.rs must emit exactly 12 relay hops for 06/distributed2's \
         4-worker tmp fan-out (got {hop_count}). Got:\n{host_src}"
    );

    // Per-source presence: every non-host worker (w0, w1, w2, w3) must
    // be both a relay src (read_msg_expect on data_<src>) AND a relay
    // dst (write_msg to data_<dst>), with the seq cross-check active
    // (`read_msg_expect`, not `read_msg`).
    for w in ["w0", "w1", "w2", "w3"] {
        let src_reads = host_src
            .matches(&format!(
                "wire::read_msg_expect(&mut data_{w}, "
            ))
            .count();
        let dst_writes = host_src
            .matches(&format!(
                "wire::write_msg(&mut data_{w}, "
            ))
            .count();
        assert!(
            src_reads >= 3,
            "host.rs must include >= 3 relay reads from data_{w} (got \
             {src_reads}; one per cross-tmp push w_{w} makes). Got:\n{host_src}"
        );
        // dst_writes includes the in_arr scatter Push (1) + the 3
        // relay forwards to data_<w>. Total >= 4.
        assert!(
            dst_writes >= 4,
            "host.rs must include >= 4 writes to data_{w} (1 in_arr scatter \
             + 3 relay forwards from other workers). Got {dst_writes}.\nhost.rs:\n{host_src}"
        );
    }
}

/// Cycle-148 placement constraint: the relay phase must sit BETWEEN
/// the two barriers in host's event list (after barrier 0 = pass-1
/// boundary; before barrier 1 = pass-2 boundary). If we splice
/// before barrier 0 OR after barrier 1 the run deadlocks (workers
/// are gated on the missing relays). Pin this by witnessing the
/// substring order in host.rs.
#[test]
fn host_relay_phase_sits_between_the_two_barrier_rounds() {
    let scratch = scratch_dir("relay_phase_position");
    let (host_src, _w0_src) = emit_06_distributed2(&scratch);

    // First occurrence of `barrier_cross(... , 0)` — the pass-1
    // boundary on host (first round, host crosses with each worker).
    // Greppable witness: `&mut ctrl_w0,` (trailing comma) — chosen
    // over a bare `ctrl_w0` substring per cycle-148 architect P2.4
    // fold-back: a future refactor renaming host's per-worker ctrl
    // var (e.g. `ctrl_w0` → `ctrl_to_w0`) would silently re-match a
    // bare-name needle through substring overlap and the test would
    // green-pass via the wrong path.
    let pos_b0 = host_src
        .find("wire::barrier_cross(&mut ctrl_w0, 0);")
        .expect("host.rs must cross barrier 0 with ctrl_w0");
    let pos_b1 = host_src
        .find("wire::barrier_cross(&mut ctrl_w0, 1);")
        .expect("host.rs must cross barrier 1 with ctrl_w0");
    let pos_relay = host_src
        .find("// TASK-0327 host-relay phase:")
        .expect("host.rs must contain the relay phase marker");

    assert!(
        pos_b0 < pos_relay,
        "host's barrier-0 round must precede the relay phase \
         (pos_b0={pos_b0}, pos_relay={pos_relay}). Splicing before \
         barrier 0 would deadlock workers gated on barrier-0 + relay."
    );
    assert!(
        pos_relay < pos_b1,
        "the relay phase must precede host's barrier-1 round \
         (pos_relay={pos_relay}, pos_b1={pos_b1}). Splicing after \
         barrier 1 would deadlock host on barrier-1 with workers gated \
         on pass-2 (which needs the relay)."
    );
}

/// Cycle-148: non-host worker emits MUST route w2w Push/Wait through
/// `data_host` (the existing host connection), NOT through a non-
/// existent worker-to-worker socket. Pin this by witnessing that
/// w0.rs does NOT contain any `data_w1`/`data_w2`/`data_w3`-named
/// sockets (those names belong to HOST's connections).
#[test]
fn non_host_worker_routes_w2w_through_data_host_only() {
    let scratch = scratch_dir("worker_uses_data_host");
    let (_host_src, w0_src) = emit_06_distributed2(&scratch);

    // w0 only has connections named `data_host` + `ctrl_host`. Any
    // sibling `data_w*` / `ctrl_w*` would be a regression (worker
    // trying to talk to a peer socket that doesn't exist on its end).
    for forbidden in ["data_w1", "data_w2", "data_w3", "ctrl_w1", "ctrl_w2", "ctrl_w3"] {
        assert!(
            !w0_src.contains(forbidden),
            "w0.rs must NOT reference `{forbidden}` — non-host workers \
             only own `data_host`/`ctrl_host`. A reference would mean a \
             regression of the cycle-148 host-relay routing decision. \
             Got:\n{w0_src}"
        );
    }
    // Positive shape: w0 sends its w2w `tmp` Pushes (3 of them, one
    // per dst worker) through `data_host`. Witness with the 3 send
    // comments the cycle-147 emit produces.
    let tmp_sends = w0_src
        .matches("wire::write_msg(&mut data_host, ")
        .filter(|_| true)
        .count();
    // 3 tmp pushes + 1 out push = 4 minimum writes to data_host on w0.
    assert!(
        tmp_sends >= 4,
        "w0.rs must include >= 4 writes to data_host (3 tmp w2w pushes \
         + 1 out gather push). Got {tmp_sends}.\nw0.rs:\n{w0_src}"
    );
}
