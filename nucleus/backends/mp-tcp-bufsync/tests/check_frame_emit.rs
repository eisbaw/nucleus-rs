//! mp-tcp-bufsync emit-string assertion test for `check loop V :
//! latency_max=T` (TASK-0052.02 review-gate finding number 2).
//!
//! Why this file exists: the implementer's main commit `d2bbf76`
//! wired identical Instant-now-then-duration-check-then-panic codegen
//! into the mp-tcp-bufsync `Event::Loop` arm to mirror pthreads-sync
//! (`nucleus/compiler/tests/check_frame_codegen.rs`), but there was
//! no test pinning the mp-tcp emit string. AC number 1 of TASK-0052.02
//! lists BOTH backends as required and the cross-backend claim was
//! untested until this file landed (architect review finding number 2
//! on the cycle, fixed in-thread).
//!
//! This test does NOT run the generated binary; it only confirms the
//! `Event::Loop` codegen path emits the contracted shape. The
//! single-worker schedule below triggers mp-tcp's
//! single-process-fused-into-one-binary emit; the `worker_bins[0]`
//! path is the rendered source.

use std::fs;
use std::path::{Path, PathBuf};

use compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    build_acfg, build_sidecar, inject_check_frames, inject_syncs, inject_transfers, link,
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
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-check-frame-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn build_per_worker(
    algo_src: &str,
    sched_src: &str,
) -> (
    std::collections::BTreeMap<compiler::event::WorkerId, Vec<compiler::event::Event>>,
    NameTables,
    compiler::sidecar::NameSidecar,
) {
    let algo_ast = parse_algo(algo_src).expect("algo parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(sched_src).expect("sched parse");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    let per_worker = acfg_to_events(&acfg);
    let per_worker =
        inject_check_frames(per_worker, &linked.sched.checks, &acfg.name_iter_vars);
    let sidecar = build_sidecar(&linked, &acfg).expect("sidecar");
    let names = NameTables {
        data: acfg.name_data.iter().map(|(n, i)| (*i, n.clone())).collect(),
        kernel: acfg
            .name_kernels
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        worker: acfg
            .name_workers
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        iter_var: acfg
            .name_iter_vars
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
    };
    (per_worker, names, sidecar)
}

#[test]
fn mp_tcp_bufsync_emit_includes_panic_instrumentation_on_check_loop() {
    // TASK-0052.02 review-gate finding #2: the mp-tcp emit path
    // landed in commit d2bbf76 was untested. This test pins the
    // emit-string shape so a regression is caught.
    let algo = "\
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
    check loop n : latency_max = 1ms;
}
";
    let (per_worker, names, sidecar) = build_per_worker(algo, sched);

    let out = scratch_dir("mp_tcp_panic_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed for a valid schedule with a check loop");

    // mp-tcp emits per-worker bin files; for a single-worker schedule
    // there is one entry.
    assert!(
        !res.worker_bins.is_empty(),
        "expected at least one worker_bin; got none"
    );
    let bin_src = fs::read_to_string(&res.worker_bins[0]).expect("read worker bin");

    assert!(
        bin_src.contains("let _check_start = std::time::Instant::now();"),
        "worker bin must contain Instant::now() inside the loop body. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("let _check_elapsed = _check_start.elapsed().as_nanos();"),
        "worker bin must contain the elapsed-nanos read. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("if _check_elapsed > 1000000_u128"),
        "worker bin must contain `if _check_elapsed > 1000000_u128`. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("panic!(\"latency budget violated on `check loop n`"),
        "panic message must name the loop_var `n`. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("max 1000000 ns"),
        "panic message must contain `max 1000000 ns`. Got:\n{bin_src}"
    );
}

#[test]
fn mp_tcp_bufsync_emit_unchanged_without_check_loop_directive() {
    // Sister-test of the pthreads-sync variant: a schedule WITHOUT
    // `check loop V` must NOT emit Instant::now(). Determinism
    // contract: zero byte change when no check directive is present.
    let algo = "\
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
}
";
    let (per_worker, names, sidecar) = build_per_worker(algo, sched);

    let out = scratch_dir("mp_tcp_no_check");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed");
    assert!(!res.worker_bins.is_empty());
    let bin_src = fs::read_to_string(&res.worker_bins[0]).expect("read worker bin");

    assert!(
        !bin_src.contains("Instant::now"),
        "worker bin MUST NOT contain Instant::now() when no check loop directive. Got:\n{bin_src}"
    );
    assert!(
        !bin_src.contains("_check_start"),
        "worker bin MUST NOT contain _check_start. Got:\n{bin_src}"
    );
}
