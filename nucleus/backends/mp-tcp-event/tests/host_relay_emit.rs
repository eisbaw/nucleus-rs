//! mp-tcp-event host-relay emit pinning (TASK-0327 cycle 149).
//!
//! Why this file exists: cycle 149 lifted the prior `chan_pairs`
//! fail-loud rejection of worker-to-worker Push pairs — when a
//! non-host worker's `Push` names another non-host worker, both src
//! and dst use peer_idx=0 (host) for their reactor and HOST runs a
//! SYNCHRONOUS RELAY PHASE that drains each `inbound[seq]` and
//! re-pushes to `outbound[(seq, dst_peer_idx_at_host)]`.
//!
//! This file pins the relay-phase emit shape on the actual in-tree
//! 06-separable-filter / distributed2 reproducer so a regression that
//! silently drops the relay (e.g. an over-zealous refactor of
//! `render_relay_phase`, a wrong `relay_phase_insertion_point`
//! heuristic, or a wrong dst_peer index) fails LOUD here before the
//! e2e cell runs.
//!
//! Scope: codegen text only. End-to-end build + run is exercised by
//! the e2e matrix's `06-separable-filter/distributed2 × mp-tcp-event`
//! cell (cycle-149 promoted from [[skip]] to [[required]]).
//!
//! Mirror of `nucleus/backends/mp-tcp-bufsync/tests/host_relay_emit.rs`
//! (cycle 148).

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
        .expect("three ancestors above mp-tcp-event crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    // TASK-0426.01: per-call-unique scratch dir via the shared helper
    // (created once, never removed) — kills the remove/create race class.
    let target = repo_root().join("nucleus/target/mp-tcp-event-host-relay-scratch");
    test_common::unique_scratch_dir(&target, name)
}

/// Compile the in-tree 06-separable-filter / distributed2 schedule
/// through the mp-tcp-event backend, returning `host.rs` and `w0.rs`
/// as Strings. Same pass sequence as mp-tcp-bufsync's host-relay
/// test (driver/src/main.rs pipeline).
fn emit_06_distributed2(scratch: &Path) -> (String, String) {
    let root = repo_root();
    let algo_src =
        fs::read_to_string(root.join("nuc-nucleus/examples/06-separable-filter/prog.algo.nuc"))
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
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);

    let out_dir = scratch.join("gen");
    let _result = mp_tcp_event::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-event emit");
    let host_src = fs::read_to_string(out_dir.join("src/bin/host.rs")).expect("read host.rs");
    let w0_src = fs::read_to_string(out_dir.join("src/bin/w0.rs")).expect("read w0.rs");
    (host_src, w0_src)
}

/// Cycle-149 AC#1/AC#2 (mp-tcp-event slice): the host main MUST
/// include the synchronous relay phase, with exactly 12 hops
/// (N*(N-1) for N=4) under the `06/distributed2` schedule, srcs in
/// sorted-WorkerId order (one block per w0..w3) — the alignment
/// relied on by `Plan::render_relay_phase` documented order.
#[test]
fn host_emit_includes_task_0327_relay_phase_with_12_hops() {
    let scratch = scratch_dir("relay_phase_shape");
    let (host_src, _w0_src) = emit_06_distributed2(&scratch);

    // Header marker — the cycle-149 explainer block above the hops.
    assert!(
        host_src.contains("// TASK-0327 host-relay phase:"),
        "host.rs must carry the cycle-149 relay-phase header marker. \
         Got:\n{host_src}"
    );

    // Exactly 12 hops (4*3 = N*(N-1) for the 06/distributed2 4-worker
    // shape). Greppable witness: every hop line carries the marker
    // `relay \`tmp\` from`. Same predicate shape as cycle-148
    // bufsync test for consistency.
    let hop_count = host_src.matches("relay `tmp` from").count();
    assert_eq!(
        hop_count, 12,
        "host.rs must emit exactly 12 relay hops for 06/distributed2's \
         4-worker tmp fan-out (got {hop_count}). Got:\n{host_src}"
    );

    // Every hop must go through `__relay.relay_one(...)` — the
    // host-relay primitive defined in runtime.rs cycle 149. A
    // regression that emitted bare `chan_<rid>.push/wait` on host
    // (which has no chan instance for w2w pairs) would silently
    // panic at runtime.
    let relay_one_count = host_src.matches("__relay.relay_one(").count();
    assert_eq!(
        relay_one_count, 12,
        "host.rs must call __relay.relay_one(...) exactly 12 times \
         (1 per hop); got {relay_one_count}. Got:\n{host_src}"
    );

    // dst_peer index range check: every hop's dst_peer must be in
    // [0, 3] (host has 4 non-host peers w0..w3 at indices 0..3).
    // Negative pin: regression that mapped dst_peer to WorkerId.0
    // (would give 1..4) would fail here.
    for bad in ["4usize", "5usize", "99usize"] {
        // Substring narrow to the relay-call form so a hop_count or
        // similar literal elsewhere doesn't false-positive.
        assert!(
            !host_src.contains(&format!(", {bad}, ")) || !host_src.contains("relay_one("),
            "host.rs must not pass dst_peer={bad} to relay_one (host \
             only has 4 peers w0..w3 = indices 0..3). Got:\n{host_src}"
        );
    }
}

/// Cycle-149 placement constraint: the relay phase must sit BETWEEN
/// the two barrier rounds in host's event list (after barrier 0 =
/// pass-1 boundary; before barrier 1 = pass-2 boundary). Mirror of
/// cycle-148 bufsync test; the heuristic
/// (`relay_phase_insertion_point` = LAST top-level Sync) is shared
/// verbatim in spirit (different file, same shape).
#[test]
fn host_relay_phase_sits_between_the_two_barrier_rounds() {
    let scratch = scratch_dir("relay_phase_position");
    let (host_src, _w0_src) = emit_06_distributed2(&scratch);

    // Barriers in mp-tcp-event lower to `bar_<bid>.wait()` shims (NOT
    // direct `wire::barrier_cross` calls — see emit_barrier_shims).
    // Greppable witnesses on the host's main():
    let pos_b0 = host_src
        .find("bar_0.wait();")
        .expect("host.rs must cross barrier 0 via bar_0.wait()");
    let pos_b1 = host_src
        .find("bar_1.wait();")
        .expect("host.rs must cross barrier 1 via bar_1.wait()");
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
         barrier 1 would deadlock host on barrier-1 with workers \
         gated on pass-2 (which needs the relay)."
    );
}

/// Cycle-149: non-host worker emits MUST route w2w `Push`/`Wait`
/// through the SINGLE `data_host` reactor peer (peer_idx=0), NOT
/// through any non-existent worker-to-worker socket. Pin this by
/// witnessing that w0.rs's Chan::new calls all carry peer_idx=0 (the
/// host-relay route).
#[test]
fn non_host_worker_routes_w2w_through_host_peer_only() {
    let scratch = scratch_dir("worker_uses_host_peer");
    let (_host_src, w0_src) = emit_06_distributed2(&scratch);

    // Every Chan::new on w0 must use peer_idx=0 (host). A regression
    // that left a non-zero peer_idx on a w2w pair would either fail
    // codegen (peer_index_for None) or silently mis-route. Greppable
    // witness: the third positional arg of Chan::new is the peer_idx;
    // the rendered form is `<seq>u64, <peer>usize, <cap>usize`.
    // Negative pin: any `, [1-9]\d*usize,` after `Chan::new(`'s seq
    // arg in w0.rs would mean a non-host peer index.
    //
    // Easier check: count Chan::new sites + count `0usize,` in the
    // chan-construction region. Total Chan::new on w0 (consumer of
    // its hblur push to host + producer of vblur into out + 3
    // cross-tmp Pushes + 3 cross-tmp Waits + ...). Use the simpler
    // pin: NO Chan::new on w0 should pass a peer arg other than 0.
    //
    // Scan: for each `Chan::new(` site, parse the next ` Nusize,`
    // pattern after the `u64, ` seq separator and assert it is 0.
    let mut cursor = 0usize;
    let mut chan_new_count = 0usize;
    while let Some(rel) = w0_src[cursor..].find("Chan::new(") {
        let pos = cursor + rel;
        // Locate the seq's `u64,` then the next ` Nusize,`.
        let after_call = &w0_src[pos + "Chan::new(".len()..];
        let u64_pos = after_call
            .find("u64,")
            .unwrap_or_else(|| panic!("Chan::new at offset {pos} missing seq u64 arg"));
        let after_seq = &after_call[u64_pos + "u64,".len()..];
        // Skip leading whitespace then parse digits up to `usize,`.
        let after_seq_trim = after_seq.trim_start();
        let usize_pos = after_seq_trim
            .find("usize,")
            .unwrap_or_else(|| panic!("Chan::new at offset {pos} missing peer usize arg"));
        let peer_str = &after_seq_trim[..usize_pos];
        let peer: usize = peer_str
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("Chan::new at offset {pos} peer `{peer_str}`: {e}"));
        assert_eq!(
            peer, 0,
            "w0.rs Chan::new at offset {pos} carries peer_idx={peer}; \
             cycle-149 host-relay requires peer_idx=0 (host) for every \
             chan a non-host worker owns. A non-zero peer_idx would \
             mean the host-relay routing was bypassed.\nw0.rs:\n{w0_src}"
        );
        chan_new_count += 1;
        cursor = pos + "Chan::new(".len();
    }
    // 06/distributed2 on w0 has the chans touching: 1 in_arr Wait
    // from host + 3 cross-tmp Pushes to peers + 3 cross-tmp Waits
    // from peers + 1 out Push to host = 8 chans. (The walker only
    // builds Chan::new for chans the worker actually touches via
    // worker_chans.)
    assert_eq!(
        chan_new_count, 8,
        "w0.rs must declare exactly 8 Chan::new instances for \
         06/distributed2 (1 in_arr + 6 cross-tmp + 1 out); got \
         {chan_new_count}. Drift here means worker_chans or the chan_ids \
         set has shifted shape — re-derive the expected count.\nw0.rs:\n{w0_src}"
    );
}
