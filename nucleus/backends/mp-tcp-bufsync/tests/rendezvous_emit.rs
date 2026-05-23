//! mp-tcp-bufsync rendezvous-file emit pinning (TASK-0176).
//!
//! Why this file exists: TASK-0176 replaced the port-handshake
//! mechanism from
//!
//!     run.sh: PORT_w0=$(./__nuc_pick_port); export NUC_TCP_PORT_w0
//!     host:   read NUC_TCP_PORT_<nwn> env var; TcpListener::bind that port
//!     worker: read NUC_TCP_PORT_<wn>  env var; TcpStream::connect that port
//!
//! to
//!
//!     run.sh: NUC_RENDEZVOUS_DIR=$here/.nuc-rendezvous-$$ + mkdir + trap
//!     host:   TcpListener::bind("127.0.0.1:0") + atomic write of
//!             local_addr().port() to $NUC_RENDEZVOUS_DIR/<nwn>.port
//!             (tmp + rename)
//!     worker: poll $NUC_RENDEZVOUS_DIR/<wn>.port (600 x 10ms = 6s),
//!             parse port, connect_retry
//!
//! This closed the close-then-rebind TOCTOU window between the picker
//! binary exiting and the host re-binding. This test pins the new
//! emit-string shape and the ABSENCE of the old strings, so a
//! regression silently reintroducing either the picker binary or the
//! `NUC_TCP_PORT_*` env-var handshake fails LOUD here even if the
//! cross-backend pingpong test happens to stay green (it would; the
//! data motion is unchanged).
//!
//! Scope: codegen text only. End-to-end build + run is exercised by
//! `tests/pingpong.rs` and the e2e matrix; the stress concurrency
//! arm by `just port-stress-check` (AC#2).

use std::fs;
use std::path::{Path, PathBuf};

use compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
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
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-rendezvous-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// Smallest meaningful multi-worker fixture: host produces x, w0
// echoes it back. One Push (host->w0) + one Wait + one Push (w0->host)
// + one Wait. Triggers the multi-process emit path.
const ALGO_SRC: &str = "\
const N : usize = 4;
data x : i32[N];
data y : i32[N];
kernel produce : ()      -> i32[N] effectful;
kernel echo    : (i32[N]) -> i32[N] pure;
kernel sink    : (i32[N]) -> () effectful;
x <-- produce();
y <-- echo(x);
sink(y);
";

const SCHED_SRC: &str = "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0 };
    place produce on host;
    place echo on w0;
    place sink on host;
    transfer x : sync;
    transfer y : sync;
}
";

fn lower(scratch: &Path) -> (
    PathBuf,
    NameTables,
    compiler::sidecar::NameSidecar,
    std::collections::BTreeMap<compiler::event::WorkerId, Vec<compiler::event::Event>>,
) {
    let kernels_path = scratch.join("kernels.rs");
    fs::write(
        &kernels_path,
        "pub fn produce() -> Vec<i32> { vec![0; 4] }\n\
         pub fn echo(x: Vec<i32>) -> Vec<i32> { x }\n\
         pub fn sink(_y: Vec<i32>) {}\n",
    )
    .expect("write kernels stub");

    let algo_ast = parse_algo(ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);
    (kernels_path, names, sidecar, per_worker)
}

#[test]
fn mp_tcp_bufsync_emit_uses_rendezvous_file_handshake_not_env_port() {
    let scratch = scratch_dir("rendezvous_shape");
    let (kernels_path, names, sidecar, per_worker) = lower(&scratch);
    let out_dir = scratch.join("gen");
    let result = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-bufsync emit");

    let host_src = fs::read_to_string(out_dir.join("src/bin/host.rs")).expect("read host.rs");
    let w0_src = fs::read_to_string(out_dir.join("src/bin/w0.rs")).expect("read w0.rs");
    let run_sh = fs::read_to_string(out_dir.join("run.sh")).expect("read run.sh");
    let cargo_toml = fs::read_to_string(&result.cargo_toml).expect("read Cargo.toml");

    // ----- Host: binds 127.0.0.1:0 itself; writes the kernel-
    // assigned port atomically (tmp + rename) to the rendezvous file.
    assert!(
        host_src.contains("TcpListener::bind(\"127.0.0.1:0\")"),
        "host must bind 127.0.0.1:0 directly (kernel-assigned ephemeral). Got:\n{host_src}"
    );
    assert!(
        host_src.contains("local_addr()"),
        "host must read local_addr() to get the OS-assigned port. Got:\n{host_src}"
    );
    assert!(
        host_src.contains("\"NUC_RENDEZVOUS_DIR\""),
        "host must read NUC_RENDEZVOUS_DIR. Got:\n{host_src}"
    );
    assert!(
        host_src.contains("w0.port.tmp") && host_src.contains("rename"),
        "host must publish the port via tmp-file + rename (atomic). Got:\n{host_src}"
    );

    // ----- Worker: polls the rendezvous file, reads the port,
    // then connect_retry. No env-var port read.
    assert!(
        w0_src.contains("\"NUC_RENDEZVOUS_DIR\""),
        "worker must read NUC_RENDEZVOUS_DIR. Got:\n{w0_src}"
    );
    assert!(
        w0_src.contains("read_rendezvous_port"),
        "worker must use the read_rendezvous_port poll helper. Got:\n{w0_src}"
    );
    assert!(
        w0_src.contains("attempt > 600"),
        "worker poll must be bounded (600 attempts; symmetric with connect_retry). Got:\n{w0_src}"
    );
    assert!(
        w0_src.contains("did not appear within 6s"),
        "worker poll timeout must fail loud with a clear message. Got:\n{w0_src}"
    );
    assert!(
        w0_src.contains("connect_retry(port"),
        "worker must still use connect_retry to TCP-connect. Got:\n{w0_src}"
    );

    // ----- run.sh: sets up the rendezvous dir + cleans it up on exit.
    assert!(
        run_sh.contains("NUC_RENDEZVOUS_DIR=\"$here/.nuc-rendezvous-$$\""),
        "run.sh must declare NUC_RENDEZVOUS_DIR with a $$ suffix. Got:\n{run_sh}"
    );
    assert!(
        run_sh.contains("mkdir -p \"$NUC_RENDEZVOUS_DIR\""),
        "run.sh must mkdir the rendezvous dir. Got:\n{run_sh}"
    );
    assert!(
        run_sh.contains("trap 'rm -rf \"$NUC_RENDEZVOUS_DIR\"' EXIT"),
        "run.sh must trap EXIT to clean up the rendezvous dir. Got:\n{run_sh}"
    );
    assert!(
        run_sh.contains("export NUC_RENDEZVOUS_DIR"),
        "run.sh must export NUC_RENDEZVOUS_DIR to the workers. Got:\n{run_sh}"
    );

    // ----- ABSENCE checks: the old close-then-rebind picker mechanism
    // must NOT come back silently. These are the load-bearing
    // negative assertions of the TOCTOU fix.
    assert!(
        !run_sh.contains("pick_port"),
        "run.sh must NOT define a pick_port function (TASK-0176: \
         removed close-then-rebind picker). Got:\n{run_sh}"
    );
    assert!(
        !run_sh.contains("__nuc_pick_port"),
        "run.sh must NOT reference the __nuc_pick_port helper binary \
         (TASK-0176: deleted). Got:\n{run_sh}"
    );
    assert!(
        !run_sh.contains("NUC_TCP_PORT_"),
        "run.sh must NOT export NUC_TCP_PORT_<worker> (TASK-0176: \
         replaced by rendezvous file). Got:\n{run_sh}"
    );
    assert!(
        !host_src.contains("NUC_TCP_PORT_"),
        "host must NOT read any NUC_TCP_PORT_<worker> env var (TASK-0176). Got:\n{host_src}"
    );
    assert!(
        !w0_src.contains("NUC_TCP_PORT_"),
        "worker must NOT read any NUC_TCP_PORT_<worker> env var (TASK-0176). Got:\n{w0_src}"
    );
    assert!(
        !cargo_toml.contains("__nuc_pick_port"),
        "Cargo.toml must NOT declare a __nuc_pick_port [[bin]] target \
         (TASK-0176: helper deleted). Got:\n{cargo_toml}"
    );

    // Sanity: the worker_bins set is exactly {host, w0}, no picker.
    let bin_filenames: Vec<String> = result
        .worker_bins
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        bin_filenames.len(),
        2,
        "expected exactly 2 worker bins (host + w0); got {bin_filenames:?}"
    );
    assert!(
        !bin_filenames.iter().any(|n| n.contains("__nuc_pick_port")),
        "no worker_bin may be the picker (TASK-0176). Got {bin_filenames:?}"
    );
}
