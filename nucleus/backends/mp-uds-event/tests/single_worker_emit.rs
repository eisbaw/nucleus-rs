//! Single-worker emit pins for mp-uds-event (TASK-0044.03 cycle 194).
//!
//! Two invariants pinned here:
//!
//! - Empty event-list (smallest legal input): both mp-uds-event and
//!   mp-tcp-event must emit byte-identical Cargo.toml + binary +
//!   run.sh + kernels.rs (NOT wire.rs — cycle 197 AC#7), since
//!   mp-uds-event's single-process arm delegates to the SAME shared
//!   renderers mp-tcp-event's single-process arm uses
//!   (`pthreads_sync::render_single_worker_main_with_kernels_attr` +
//!   `backend_common::project_skeleton::multi_binary`). wire.rs
//!   CONTENT diverges by design from cycle 197 onwards: mp-uds-event
//!   emits the inlined UDS `WIRE_RUNTIME_SRC` (UnixStream API),
//!   mp-tcp-event emits `mp_tcp_common::WIRE_RUNTIME_SRC` (TcpStream
//!   API). Single-process bins don't `mod wire;` so the divergence
//!   is invisible to the BIN byte-equiv invariant.
//!
//! - Real non-trivial witness (01-elementwise-add / naive): the same
//!   byte-identical assertion on a real fixture proves the invariant
//!   survives a kernel-call site + sidecar consumption, not just an
//!   empty scaffold. Mirrors the cycle-191 (openmp-rs) and cycle-192
//!   (mp-tcp-poll) sibling tests. Drift detection: any
//!   mp-uds-event-specific wrapper around the delegated emitter would
//!   surface here as a diff.
//!
//! IMPORTANT byte-identity caveat: mp-tcp-event uses the OLDER
//! `wrap_single_worker` post-hoc `replacen` pattern; mp-uds-event
//! uses the newer typed-attribute parameter pattern (TASK-0177). They
//! produce byte-identical output today because both end up with the
//! same `#[path = "../kernels.rs"]\n#[allow(dead_code)]\nmod kernels;`
//! sequence (cycle-171 plan + TASK-0177 lift). If the mp-tcp-event
//! migration to the typed-parameter API ever lands and changes byte
//! output, these tests will surface it loud.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mp_uds_event::{emit, NameTables};
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

#[test]
fn single_worker_empty_eventlist_emits_byte_identical_to_mp_tcp_event() {
    let per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = NameSidecar::default();

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .expect("workspace target/");
    // TASK-0426.01: per-call-unique stem (created once by the helper,
    // never removed). The `uds`/`tcp` subdirs ride under this unique stem.
    let stem = test_common::unique_scratch_dir(
        &target.join("mp-uds-event-test-scratch"),
        "single_worker_empty",
    );
    let uds_out = stem.join("uds");
    let tcp_out = stem.join("tcp");

    let kernels = stem.join("empty_kernels.rs");
    std::fs::write(
        &kernels,
        "// Empty kernels.rs for the empty-eventlist test.\n",
    )
    .expect("empty kernels.rs");

    let uds_res =
        emit(&per_worker, &names, &sidecar, &kernels, &uds_out).expect("mp-uds-event emit");
    let tcp_res = mp_tcp_event::emit(&per_worker, &names, &sidecar, &kernels, &tcp_out)
        .expect("mp-tcp-event emit");

    // ---- The invariant: byte-identical binary + Cargo.toml + run.sh
    //                     + wire.rs + kernels.rs ----
    assert_eq!(
        uds_res.worker_bins.len(),
        1,
        "single-worker mp-uds-event must emit exactly one binary, got {}",
        uds_res.worker_bins.len()
    );
    assert_eq!(
        tcp_res.worker_bins.len(),
        1,
        "single-worker mp-tcp-event must emit exactly one binary, got {}",
        tcp_res.worker_bins.len()
    );

    let uds_bin = std::fs::read_to_string(&uds_res.worker_bins[0]).expect("uds binary");
    let tcp_bin = std::fs::read_to_string(&tcp_res.worker_bins[0]).expect("tcp binary");
    assert_eq!(
        uds_bin, tcp_bin,
        "mp-uds-event single-process binary must be byte-identical to \
         mp-tcp-event's (the cross-backend differential invariant):\n\
         uds:\n{uds_bin}\n--- tcp:\n{tcp_bin}"
    );

    let uds_cargo = std::fs::read_to_string(&uds_res.cargo_toml).expect("uds Cargo.toml");
    let tcp_cargo = std::fs::read_to_string(&tcp_res.cargo_toml).expect("tcp Cargo.toml");
    assert_eq!(
        uds_cargo, tcp_cargo,
        "mp-uds-event Cargo.toml must be byte-identical to mp-tcp-event's \
         (shared backend_common::project_skeleton::multi_binary::render_cargo_toml)"
    );

    let uds_run = std::fs::read_to_string(&uds_res.run_sh).expect("uds run.sh");
    let tcp_run = std::fs::read_to_string(&tcp_res.run_sh).expect("tcp run.sh");
    assert_eq!(
        uds_run, tcp_run,
        "mp-uds-event single-process run.sh must be byte-identical to \
         mp-tcp-event's (shared render_run_sh_single)"
    );

    // Cycle 197 (TASK-0044.03.01 AC#7): wire.rs CONTENT now diverges
    // across backends — mp-tcp-event emits `mp_tcp_common::WIRE_RUNTIME_SRC`
    // (TCP-specific TcpStream API), mp-uds-event emits the inlined
    // UDS-specific `WIRE_RUNTIME_SRC` (UnixStream API). Single-process
    // bin doesn't `mod wire;` so the file divergence is invisible to
    // the single-worker BIN byte-equiv invariant above. We pin the
    // divergence via load-bearing positive needles + the inverse
    // positive needle on TCP. The negative-needle check ("UDS wire.rs
    // contains NO `TcpStream`") would false-positive on the explanatory
    // file-header docstring (which legitimately names TcpStream when
    // explaining why we inlined), so we anchor on the function-
    // signature substring `&mut TcpStream` instead — that token can
    // only appear in actual API surface, not in prose.
    let uds_wire = std::fs::read_to_string(&uds_res.wire_rs).expect("uds wire.rs");
    let tcp_wire = std::fs::read_to_string(&tcp_res.wire_rs).expect("tcp wire.rs");
    assert!(
        uds_wire.contains("&mut UnixStream"),
        "mp-uds-event wire.rs must carry `&mut UnixStream` function \
         signatures (UDS-specific). Sign of regression to pre-cycle-197 \
         shared mp_tcp_common::WIRE_RUNTIME_SRC."
    );
    assert!(
        tcp_wire.contains("&mut TcpStream"),
        "mp-tcp-event wire.rs must carry `&mut TcpStream` function \
         signatures (TCP-specific). Oracle precondition violated."
    );
    assert!(
        !uds_wire.contains("&mut TcpStream"),
        "mp-uds-event wire.rs must NOT carry `&mut TcpStream` function \
         signatures (cycle-197 UDS swap regressed)"
    );

    let uds_kernels = std::fs::read_to_string(&uds_res.kernels_rs).expect("uds kernels.rs");
    let tcp_kernels = std::fs::read_to_string(&tcp_res.kernels_rs).expect("tcp kernels.rs");
    assert_eq!(
        uds_kernels, tcp_kernels,
        "mp-uds-event kernels.rs must be a verbatim copy (same input file)"
    );

    // Single-worker emit MUST produce runtime_rs: None (no mio reactor
    // needed for single-process).
    assert!(
        uds_res.runtime_rs.is_none(),
        "single-worker mp-uds-event emit must NOT produce a runtime.rs (no mio reactor for single-process), got: {:?}",
        uds_res.runtime_rs
    );
}

/// Non-trivial witness — proves the actual codegen (kernel call, loop
/// header, sidecar-driven pre-init) is byte-identical to mp-tcp-event,
/// not just the empty scaffold. Mirrors cycle 191 + cycle 192 sibling
/// tests.
#[test]
fn single_worker_real_example_emits_byte_identical_to_mp_tcp_event() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("01 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("01 sched");

    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: false,
        },
    );
    let kernels = ex.join("kernels.rs");

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &root.join("nucleus/target/mp-uds-event-test-scratch"),
        "single_worker_01_naive",
    );
    let uds_out = scratch.join("uds");
    let tcp_out = scratch.join("tcp");

    let uds_res = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &uds_out)
        .expect("mp-uds-event emit (single-worker real example)");
    let tcp_res = mp_tcp_event::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &tcp_out)
        .expect("mp-tcp-event emit (same input)");

    let uds_bin = std::fs::read_to_string(&uds_res.worker_bins[0]).expect("uds binary");
    let tcp_bin = std::fs::read_to_string(&tcp_res.worker_bins[0]).expect("tcp binary");
    assert_eq!(
        uds_bin, tcp_bin,
        "mp-uds-event single-process binary on 01-elementwise-add/naive MUST \
         be byte-identical to mp-tcp-event's. A diff means the delegation to \
         render_single_worker_main_with_kernels_attr was bypassed or wrapped:\n\
         === uds ===\n{uds_bin}\n=== tcp ===\n{tcp_bin}"
    );

    // Non-trivial witness check: the emitted binary MUST contain the
    // kernel call (so we are not vacuously identical).
    assert!(
        uds_bin.contains("kernels::add"),
        "01-elementwise-add witness must emit kernels::add — absence of it \
         means the test passed vacuously:\n{uds_bin}"
    );
}

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above mp-uds-event crate")
        .to_path_buf()
}
