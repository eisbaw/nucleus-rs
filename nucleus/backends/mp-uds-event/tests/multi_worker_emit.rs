//! mp-uds-event multi-worker emit-string pins (TASK-0044.03.01 cycle 197).
//!
//! Structural-twin oracle vs mp-tcp-event: for any in-scope multi-
//! worker schedule, the mp-uds-event per-worker binary differs from
//! mp-tcp-event's ONLY in the transport-layer surface:
//!
//! - `UnixListener` / `UnixStream` vs `TcpListener` / `TcpStream`
//!   (handshake, reactor type, barrier-shim type).
//! - `mio::net::UnixStream` vs `mio::net::TcpStream`.
//! - `std::os::unix::net::UnixStream` vs `std::net::TcpStream`.
//! - Path-as-rendezvous handshake helpers (`check_uds_path_len`,
//!   path-based `connect_retry`) vs TCP's port-in-file handshake
//!   helpers (`read_rendezvous_port`, port-based `connect_retry`,
//!   tmp+rename publish block).
//! - 1-line banner provenance text (cycle-197 wording names
//!   mp-uds-event + UDS; mp-tcp-event names mp-tcp-event + TCP).
//! - Set-nodelay calls (TCP-specific; UDS doesn't have Nagle).
//!
//! Approach: assert POSITIVE NEEDLES BEFORE the canonicaliser runs
//! (UnixListener/UnixStream in mp-uds-event output; TcpListener/
//! TcpStream in mp-tcp-event output). Then strip the handshake/prelude
//! block from BOTH binaries and compare the remainder. This catches
//! silent-sibling regressions of the cycle-197 transport swap without
//! locking the per-byte text of the handshake (which legitimately
//! differs structurally between TCP and UDS).
//!
//! Per cycle-195b lesson #4 + cycle-196 lesson #2: the positive-needle
//! pre-checks defend against the regression direction the canonicaliser
//! would silently no-op on (if mp-uds-event regressed to TcpStream,
//! the strip wouldn't see the swap was real).

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above mp-uds-event crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/mp-uds-event-multi-worker-emit-scratch");
    let _ = std::fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Pre-canonicaliser positive needles: both sides must carry the
/// transport-specific calls THEY are responsible for, otherwise the
/// swap silently regressed.
fn assert_pre_canon_needles(label: &str, uds_src: &str, tcp_src: &str) {
    let uds_has_uds = uds_src.contains("UnixListener::bind")
        || uds_src.contains("StdUnixStream::connect")
        || uds_src.contains("mio::net::UnixStream::from_std")
        || uds_src.contains("Rc<RefCell<std::os::unix::net::UnixStream>>");
    let tcp_has_tcp = tcp_src.contains("TcpListener::bind")
        || tcp_src.contains("StdTcpStream::connect")
        || tcp_src.contains("mio::net::TcpStream::from_std")
        || tcp_src.contains("Rc<RefCell<std::net::TcpStream>>");
    assert!(
        uds_has_uds,
        "{label}: mp-uds-event bin carries NO UDS-specific call \
         (UnixListener / StdUnixStream / mio::net::UnixStream / \
         Rc<RefCell<std::os::unix::net::UnixStream>>) — cycle-197 \
         swap regressed (canonicaliser would silently no-op)"
    );
    assert!(
        tcp_has_tcp,
        "{label}: mp-tcp-event bin carries NO TCP-specific call — \
         oracle precondition violated (mp-tcp-event itself has \
         regressed; cannot prove the swap was real)"
    );
    // Conversely: mp-uds-event must NOT carry TCP calls (any sign of
    // such would mean the cycle-197 swap missed a call site).
    assert!(
        !uds_src.contains("TcpListener::bind"),
        "{label}: mp-uds-event bin contains TcpListener::bind — \
         cycle-197 swap missed at least one call site"
    );
    assert!(
        !uds_src.contains("StdTcpStream::connect"),
        "{label}: mp-uds-event bin contains StdTcpStream::connect — \
         cycle-197 swap missed at least one call site"
    );
    assert!(
        !uds_src.contains("mio::net::TcpStream::from_std"),
        "{label}: mp-uds-event bin contains mio::net::TcpStream::from_std \
         — cycle-197 swap missed at least one call site"
    );
}

/// Strip the handshake/reactor-setup prelude (legitimate structural
/// divergence between TCP and UDS — different rendezvous mechanism),
/// the role-banner provenance line, and every line that names a
/// transport-specific type or path-vs-port primitive.
///
/// The TCP and UDS prelude blocks have DIFFERENT shape (TCP: bind
/// listener at 127.0.0.1:0 + read local port + tmp+rename publish +
/// 2× accept + set_nodelay + set_nonblocking; UDS:
/// check_uds_path_len helper definition + 2× UnixListener::bind at
/// known path + 2× accept + set_nonblocking, no Nagle + no port).
/// The line counts differ, and even after dropping all transport-
/// specific lines a residual mid-prelude diff persists (helper
/// function bodies span multiple lines, only some of which contain
/// flag tokens). Rather than enumerate every helper-body line, we
/// take the WHOLE PRELUDE OUT: drop every line from the first
/// `let rendezvous_dir: PathBuf` to the line BEFORE the first
/// `let reactor = {` line (which begins the walker-driven body).
/// The walker output + check_frame substrate + flush_outbound is
/// what we want to pin; the handshake is a structural slack the
/// transport-layer swap is allowed to consume.
fn strip_transport_prelude(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    // First, drop the BANNER (Generated by ...) and the role-specific
    // use-statements (TcpListener / TcpStream / UnixListener /
    // UnixStream / std::io::Write / std::time::Duration / std::fs).
    // These appear BEFORE the rendezvous_dir block.
    let mut kept: Vec<&str> = Vec::new();
    let mut in_prelude = false;
    let mut prelude_done = false;
    for line in lines.iter() {
        let t = line.trim_start();
        // Banner provenance differs (drop both).
        if t.contains("Generated by the mp-tcp-event backend")
            || t.contains("Generated by the mp-uds-event backend")
        {
            continue;
        }
        // Transport-specific import lines (drop both).
        if t.starts_with("use std::net::TcpListener")
            || t.starts_with("use std::net::TcpStream")
            || t.starts_with("use std::os::unix::net::UnixListener")
            || t.starts_with("use std::os::unix::net::UnixStream")
            || t.starts_with("use std::io::Write")
            || t.starts_with("use std::time::Duration")
            || t.starts_with("use std::fs;")
            || t.starts_with("use std::path::PathBuf;")
        {
            continue;
        }
        // Detect prelude start: the rendezvous_dir line is the first
        // line of emit_handshake.
        if !in_prelude && !prelude_done && t.starts_with("let rendezvous_dir:") {
            in_prelude = true;
            continue;
        }
        // Detect prelude end: the `let reactor = {` line is the first
        // line of emit_reactor_and_chans.
        if in_prelude && t.starts_with("let reactor = {") {
            in_prelude = false;
            prelude_done = true;
            // KEEP this line — it's the start of the walker-driven
            // body and must be byte-identical across backends.
            kept.push(line);
            continue;
        }
        if in_prelude {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// Normalize transport-type names that legitimately differ between
/// the two backends (UDS UnixStream vs TCP TcpStream) inside the
/// SHARED walker output: reactor `peers` Vec element type + barrier-
/// shim `ctrl_<name>` field type. After stripping the prelude these
/// are the only structural-twin lines that carry a transport-type
/// token. Normalise both to a placeholder `__STREAM__`; any residual
/// divergence then reflects a real codegen drift, not the cycle-197
/// transport swap.
fn normalize_transport_types(src: &str) -> String {
    src.replace("mio::net::UnixStream", "mio::net::__STREAM__")
        .replace("mio::net::TcpStream", "mio::net::__STREAM__")
        .replace("std::os::unix::net::UnixStream", "std::__STREAM__")
        .replace("std::net::TcpStream", "std::__STREAM__")
}

fn assert_uds_bin_equiv_tcp(label: &str, uds_src: &str, tcp_src: &str) {
    assert_pre_canon_needles(label, uds_src, tcp_src);

    let uds_canon = normalize_transport_types(&strip_transport_prelude(uds_src));
    let tcp_canon = normalize_transport_types(&strip_transport_prelude(tcp_src));

    if uds_canon != tcp_canon {
        let ul: Vec<&str> = uds_canon.lines().collect();
        let tl: Vec<&str> = tcp_canon.lines().collect();
        let mut diff = String::new();
        for (i, (a, b)) in ul.iter().zip(tl.iter()).enumerate() {
            if a != b {
                diff.push_str(&format!(
                    "line {}:\n  uds(canon):    {a:?}\n  tcp(canon):    {b:?}\n",
                    i + 1
                ));
                if diff.len() > 4096 {
                    break;
                }
            }
        }
        if ul.len() != tl.len() {
            diff.push_str(&format!("\nlength: uds={} tcp={}", ul.len(), tl.len()));
        }
        panic!(
            "{label}: mp-uds-event bin (after transport-prelude strip) \
             differs from mp-tcp-event's. Either the cycle-197 swap missed \
             a call site OR a non-emit-layer drift slipped into the \
             multi_worker substrate.\n--- divergences:\n{diff}\n\
             --- uds (canon, {ulen} lines, head 4KB) ---\n{uhead}\n\
             --- tcp (canon, {tlen} lines, head 4KB) ---\n{thead}\n",
            ulen = ul.len(),
            tlen = tl.len(),
            uhead = &uds_canon[..uds_canon.len().min(4096)],
            thead = &tcp_canon[..tcp_canon.len().min(4096)],
        );
    }
}

/// 02-split-add/split — simplest in-tree multi-worker schedule.
/// 2 used workers (host + w0); minimal codegen surface; if any
/// transport swap site missed, this fails first.
#[test]
fn split_02_uds_equiv_tcp() {
    let scratch = scratch_dir("split_02");
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).unwrap();
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );
    let kernels = ex.join("kernels.rs");

    let uds_out = scratch.join("uds");
    let tcp_out = scratch.join("tcp");
    let uds = mp_uds_event::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &uds_out)
        .expect("uds emit");
    let tcp = mp_tcp_event::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &tcp_out)
        .expect("tcp emit");

    assert_eq!(
        uds.worker_bins.len(),
        tcp.worker_bins.len(),
        "per-worker bin counts must match across uds/tcp"
    );
    let mut uds_bins = uds.worker_bins.clone();
    uds_bins.sort();
    let mut tcp_bins = tcp.worker_bins.clone();
    tcp_bins.sort();
    for (u, t) in uds_bins.iter().zip(tcp_bins.iter()) {
        let u_name = u.file_name().unwrap().to_str().unwrap();
        let t_name = t.file_name().unwrap().to_str().unwrap();
        assert_eq!(u_name, t_name, "per-worker bin names must match");
        let u_src = std::fs::read_to_string(u).expect("read uds bin");
        let t_src = std::fs::read_to_string(t).expect("read tcp bin");
        assert_uds_bin_equiv_tcp(u_name, &u_src, &t_src);
    }
}

/// 06-separable-filter/distributed2 — exercises the host-relay phase
/// (host + 4 workers, w2w pushes routed through host). Largest
/// emit-string surface; if any transport-type swap or relay-emission
/// regression slipped through, this fails more loudly than the
/// simpler 02/split case.
///
/// Pass sequence MIRRORS mp-tcp-poll's `tests/multi_worker_emit.rs::
/// separable_filter_06_distributed2_poll_equiv_bufsync` — the
/// 06/distributed2 schedule needs partition_workers, partition_rows,
/// and halo_inference for the transfer_inject pass to see the
/// cross-worker fan-out shape. The mp-uds-event driver gate widening
/// (cycle 197 step 1: apply_host_data_relay_inject and
/// apply_safe_push_reorder now include mp-uds-event) makes the same
/// passes run on the mp-uds-event input as on the mp-tcp-event input,
/// so the structural-twin invariant holds.
#[test]
fn separable_filter_06_distributed2_uds_equiv_tcp() {
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

    let scratch = scratch_dir("separable_filter_06_distributed2");
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/06-separable-filter");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = std::fs::read_to_string(ex.join("schedules/distributed2.sched.nuc")).unwrap();
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
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    // The mp-uds-event driver wires apply_host_mediation_inject +
    // apply_host_data_relay_inject (cycle 197 step 1); this oracle
    // test must mirror that pipeline order or the structural-twin
    // invariant would not hold (the inputs to emit() would differ).
    let preview = acfg_to_events(&acfg);
    let used: std::collections::BTreeSet<_> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    let host = backend_common::elect_host_from_name_workers(&acfg.name_workers, &used)
        .expect("host election must succeed on 06/distributed2");
    let acfg = apply_host_mediation_inject(acfg, host);
    let acfg = apply_host_data_relay_inject(acfg, host);
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);

    let uds_out = scratch.join("uds");
    let tcp_out = scratch.join("tcp");
    let uds =
        mp_uds_event::emit(&per_worker, &names, &sidecar, &kernels, &uds_out).expect("uds emit");
    let tcp =
        mp_tcp_event::emit(&per_worker, &names, &sidecar, &kernels, &tcp_out).expect("tcp emit");

    let mut uds_bins = uds.worker_bins.clone();
    uds_bins.sort();
    let mut tcp_bins = tcp.worker_bins.clone();
    tcp_bins.sort();
    for (u, t) in uds_bins.iter().zip(tcp_bins.iter()) {
        let u_name = u.file_name().unwrap().to_str().unwrap();
        let u_src = std::fs::read_to_string(u).expect("read uds bin");
        let t_src = std::fs::read_to_string(t).expect("read tcp bin");
        assert_uds_bin_equiv_tcp(u_name, &u_src, &t_src);
    }
}

/// 15-transpose/distributed-rows — the host-EXCLUDING-barrier emit
/// oracle on a REAL `[[required]]` cell (TASK-0044.08, cycle 232).
///
/// Why this exists alongside `separable_filter_06_distributed2_uds_equiv_tcp`
/// above: 06/distributed2's barriers all INCLUDE host (every barrier-
/// participant union there is driven by a host-touching transfer
/// boundary), so `apply_host_mediation_inject` is a complete NO-OP on
/// that cell — it never exercises the host-EXCLUDING path. This test
/// closes that hole by retargeting to 15-transpose/distributed-rows,
/// whose `xpose on {w0,w1,w2,w3}` placement produces a genuinely
/// host-EXCLUDING inner barrier (participants `{w0,w1,w2,w3}`, host
/// absent; its SyncTag bid is DERIVED at runtime, not pinned — see
/// TASK-0044.11), the precise shape mediation exists to handle for the
/// one-CTRL-stream-per-(host,worker) UDS-star topology.
///
/// Strengthened over the 06 sibling: this cell asserts (a) the
/// UNMEDIATED mp-uds-event emit FAILS (host-excluding barrier rejected
/// by `Plan::build`, multi_worker/mod.rs:286-297) — proving mediation
/// is load-bearing, not cosmetic — THEN (b) after mediation the
/// per-worker bins are the structural twin of mp-tcp-event's (the same
/// transport-canonicalised equivalence the 02/06 oracles assert). If
/// mediation were a no-op here (as on 06), half (a) would FAIL the
/// `expect_err` (the unmediated emit would succeed), so this test
/// cannot pass vacuously.
///
/// PROVENANCE (TASK-0044.08): sibling of TASK-0044.01.03 (openmp-rs) +
/// TASK-0044.02.02.01 (mp-tcp-poll), both closed cycle 231; this task
/// closes the last two backends (mp-tcp-event + mp-uds-event) of the
/// family. 15-transpose/distributed-rows is the ONLY schedule that is
/// BOTH genuinely host-excluding AND a promoted `[[required]]` e2e
/// cell on these backends (a scan of every multi-worker schedule at
/// cycle 231 established this; the originally-filed 03-reduction
/// premise was empirically false — its barriers all include host).
///
/// Pass sequence MIRRORS the driver's mp-uds-event gate EXACTLY: the
/// IR-stage passes (parse..inject_transfers) PLUS `apply_host_mediation_inject`
/// PLUS `apply_host_data_relay_inject` (the mp-uds-event driver gate
/// applies BOTH — driver/src/main.rs ~525-563 mediation gate AND
/// ~592-614 data-relay gate, both list mp-uds-event). Identical to the
/// 06 sibling's pipeline; the only difference is the schedule. On
/// 15-transpose/distributed-rows data-relay is a no-op (no in-`Repeat`-
/// body non-host↔non-host Push pair) but applied for production-ACFG
/// fidelity.
#[test]
fn transpose_15_distributed_rows_uds_equiv_tcp() {
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
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    // ---- Half (a): the UNMEDIATED mp-uds-event emit is REJECTED. ----
    // The host-EXCLUDING inner compute barrier ({w0,w1,w2,w3}, host
    // absent) cannot be lowered by the UDS-star topology
    // (multi_worker/mod.rs:286-297), so the unmediated emit MUST fail.
    // This is what distinguishes this cell from 06/distributed2 (where
    // mediation is a complete no-op): if mediation were a no-op here,
    // this `expect_err` would fail.
    let unmediated_pw = acfg_to_events(&acfg);
    let unmediated_sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar (unmediated)");
    let unmediated_names = NameTables::from_acfg(&acfg);
    let unmediated_emit = mp_uds_event::emit(
        &unmediated_pw,
        &unmediated_names,
        &unmediated_sidecar,
        &kernels,
        &scratch.join("unmediated"),
    );
    let err = unmediated_emit.expect_err(
        "15-transpose/distributed-rows: UNMEDIATED mp-uds-event emit MUST fail \
         (host-excluding barrier rejected by Plan::build); if it succeeds, the \
         mediation pass is a no-op and this oracle does not exercise mediation",
    );
    let err_text = format!("{err:?}");
    assert!(
        err_text.contains("exclude the host worker"),
        "15-transpose/distributed-rows: UNMEDIATED emit must fail with the \
         host-excluding-barrier ContractGap; got: {err_text}"
    );

    // ---- Mediate: same passes the mp-uds-event driver gate runs. ----
    // The mp-uds-event driver wires apply_host_mediation_inject +
    // apply_host_data_relay_inject (driver/src/main.rs ~525-563 +
    // ~592-614); this oracle mirrors that pipeline order or the
    // structural-twin invariant would not hold (the inputs to emit()
    // would differ from production). Host election via the shared
    // helper, mirroring the driver (memory
    // feedback-driver-must-mirror-backend-election-exactly).
    let preview = acfg_to_events(&acfg);
    let used: std::collections::BTreeSet<_> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    let host = backend_common::elect_host_from_name_workers(&acfg.name_workers, &used)
        .expect("host election must succeed on 15-transpose/distributed-rows");
    // The host-EXCLUDING barrier's bid is the SyncTag whose participant
    // set lacks host; the worker shim emitter renders `Bar{bid}` /
    // `let bar_{bid}` with bid == SyncTag.0. Derive it from the UNMEDIATED
    // projection (post-mediation the set includes host) so the
    // post-mediation anchor stays correct under a sync-tag renumber
    // (TASK-0044.11; was hardcoded bid=2).
    let host_excluding_bid = test_common::host_excluding_barrier_bid(&unmediated_pw, host).expect(
        "15-transpose/distributed-rows must carry a host-excluding barrier \
         (its bid drives the post-mediation Bar{bid} anchor)",
    );
    let acfg = apply_host_mediation_inject(acfg, host);
    let acfg = apply_host_data_relay_inject(acfg, host);
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);

    // ---- Half (b): post-mediation mp-uds-event == mp-tcp-event twin. ----
    let uds_out = scratch.join("uds");
    let tcp_out = scratch.join("tcp");
    let uds =
        mp_uds_event::emit(&per_worker, &names, &sidecar, &kernels, &uds_out).expect("uds emit");
    let tcp =
        mp_tcp_event::emit(&per_worker, &names, &sidecar, &kernels, &tcp_out).expect("tcp emit");

    // Anchor: host (the mediated barrier's new hub) MUST now carry the
    // formerly host-excluding barrier shim `Bar{bid}` — host did NOT
    // participate in that barrier (bid DERIVED above, TASK-0044.11)
    // pre-mediation, so this BITES where a bare barrier-substring would
    // pass vacuously (host already crosses the host-INCLUDED barriers).
    // Checked on the mp-uds-event bin.
    let host_name = names.worker.get(&host).expect("host name").clone();
    let uds_host_bin = uds
        .worker_bins
        .iter()
        .find(|p| p.file_name().unwrap().to_str().unwrap() == format!("{host_name}.rs"))
        .expect("host bin must be present in the mediated uds emit");
    let uds_host_src = std::fs::read_to_string(uds_host_bin).expect("read uds host bin");
    let bar_struct = format!("struct Bar{host_excluding_bid} {{");
    let bar_let = format!("let bar_{host_excluding_bid} = Bar{host_excluding_bid} {{");
    assert!(
        uds_host_src.contains(&bar_struct) && uds_host_src.contains(&bar_let),
        "15-transpose/distributed-rows: after host-mediation the mp-uds-event \
         host bin ({host_name}.rs) MUST declare the barrier shim for the \
         formerly host-EXCLUDING barrier (bid {host_excluding_bid}: \
         `{bar_struct}` / `{bar_let}`). host bin:\n{uds_host_src}"
    );

    let mut uds_bins = uds.worker_bins.clone();
    uds_bins.sort();
    let mut tcp_bins = tcp.worker_bins.clone();
    tcp_bins.sort();
    assert_eq!(
        uds_bins.len(),
        tcp_bins.len(),
        "per-worker bin counts must match across uds/tcp"
    );
    for (u, t) in uds_bins.iter().zip(tcp_bins.iter()) {
        let u_name = u.file_name().unwrap().to_str().unwrap();
        let u_src = std::fs::read_to_string(u).expect("read uds bin");
        let t_src = std::fs::read_to_string(t).expect("read tcp bin");
        assert_uds_bin_equiv_tcp(u_name, &u_src, &t_src);
    }
}

/// AC#10 const-in-IndexExpr regression pin: any future regression in
/// `pthreads_sync::render_const_expr_pub` (the shared renderer the
/// multi_worker walker consumes) that fails to resolve the `ITERS`
/// const to its literal value would be MISSED by the structural-
/// twin oracle (both mp-uds-event AND mp-tcp-event would drift in
/// lockstep). This test pins the EMITTED LITERAL on mp-uds-event
/// directly, so the regression surfaces here even if mp-tcp-event
/// silently drifts too.
///
/// Forward-carried from TASK-0044.01.02 cycle 193 (cycle-36 pin
/// pattern). The bite point is the moment multi_worker_walker is
/// consumed — HERE not at the single-worker arm.
#[test]
fn const_in_indexexpr_multi_worker_emit_pin() {
    let r = test_common::lower_for_test(
        test_common::CONST_IN_INDEXEXPR_ALGO_SRC,
        test_common::CONST_IN_INDEXEXPR_SCHED_SRC,
        &test_common::LowerForTestOpts::default(),
    );

    let scratch = scratch_dir("const_in_indexexpr_mp_uds_event");
    let kernels = scratch.join("kernels.rs");
    std::fs::write(&kernels, "// stub for emit-string test\n").unwrap();

    let result = mp_uds_event::emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &kernels,
        &scratch.join("gen"),
    )
    .expect("mp-uds-event emit must succeed on const-in-IndexExpr fixture");

    // Multi-worker: read every worker_bin and concat for the search
    // (the walker may have placed the IndexExpr on any of them).
    let mut combined = String::new();
    for bin in &result.worker_bins {
        let src = std::fs::read_to_string(bin).expect("read worker bin");
        combined.push_str(&src);
        combined.push('\n');
    }

    let iters_val = test_common::CONST_IN_INDEXEXPR_ITERS_VALUE;
    let resolved_row = format!("({iters_val}) * 4");
    let bare_ident = test_common::CONST_IN_INDEXEXPR_ITERS_IDENT;

    // (1) The resolved literal must appear at the IndexExpr site.
    assert!(
        combined.contains(&resolved_row),
        "mp-uds-event multi-worker bins must contain the resolved \
         `ITERS={iters_val}` literal at the IndexExpr row-stride site \
         (`{resolved_row}`); cycle-35 render_const_expr fix not \
         reaching this code path via backend_common::multi_worker_walker. \
         Combined bin sources:\n{combined}"
    );

    // (2) The bare const ident `ITERS` does NOT appear anywhere in
    // the emitted multi-worker bins. If it does, the generated Rust
    // would fail to compile (no `ITERS` const is emitted).
    assert!(
        !combined.contains(bare_ident),
        "mp-uds-event multi-worker bins must NOT contain the bare const \
         ident `{bare_ident}` — its presence means render_int_expr \
         failed to resolve to the sidecar const value, and the generated \
         Rust would fail to compile. Combined:\n{combined}"
    );
}
