//! mp-tcp-bufsync reuse circular-buffer codegen emit pinning
//! (TASK-0284 cycle 107).
//!
//! ## Why this file exists
//!
//! TASK-0270 cycle 104 wired reuse circular-buffer codegen into the
//! shared `backend_common::multi_worker_walker::render_worker_events_inner`,
//! which is consumed by pthreads-sync (multi-worker), pthreads-async,
//! and mp-tcp-event. mp-tcp-bufsync has its own per-event walker
//! (`Plan::render_events`) that delegates only the strip-mine
//! ABS-rebind to `render_block_tag_loop_header` — it did NOT consume
//! the new reuse codegen, leaving a silent-sibling defect: a future
//! multi-worker reuse schedule on mp-tcp-bufsync's capability surface
//! (sync notify + buffer=1) would emit byte-identical-to-spec output
//! but at the slow no-reuse rate, with no marker reporting the gap.
//!
//! TASK-0284 cycle 107 mirrored the cycle-104 4-block call sequence
//! (compute_block_tag_abs_exprs + render_reuse_buf_decls_pub +
//! render_reuse_marker_comment + render_reuse_per_iter_update_pub +
//! reuse-active-extended child ctx) into mp-tcp-bufsync's
//! `Plan::render_events` at BOTH arms. This test pins the codegen
//! shape on the worker emit (`src/bin/w0.rs`) for a synthetic 2-worker
//! reuse fixture.
//!
//! ## Fixture
//!
//! Smallest multi-worker reuse shape: 2-worker schedule (host + w0)
//! where the worker carries a `for n : reuse;` loop reading
//! `x[n-1]`, `x[n]`, `x[n+1]` (reuse window {-1, 0, +1}, length=3,
//! min_offset=-1). Host produces + sinks; worker computes via blur3.

use std::fs;
use std::path::{Path, PathBuf};

use nucleus_compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    apply_reuse_inference, build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
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
    // TASK-0426: per-call-unique subdir so concurrent test threads never
    // share a path (same shared-parent remove/create-vs-write race the
    // check_frame_emit.rs sibling hit; fixed proactively here).
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-reuse-codegen-scratch");
    let _ = fs::create_dir_all(&target);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = target.join(format!("{name}-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// Smallest multi-worker reuse fixture: host produces x, w0 computes
// y[n] = blur3(x[n-1], x[n], x[n+1]) with `for n : reuse;`, host
// sinks y. Triggers the multi-process emit path AND exercises
// mp-tcp-bufsync's Plan::render_events Event::Loop regular arm with
// active reuse_widths.
const ALGO_SRC: &str = "\
const N : usize = 16;
data x : i32[N];
data y : i32[N];
kernel produce : ()              -> i32[N] effectful;
kernel blur3   : (i32, i32, i32) -> i32    pure;
kernel sink    : (i32[N])        -> ()     effectful;
x <-- produce();
for n : 1 .. N-1 {
    y[n] <-- blur3(x[n-1], x[n], x[n+1]);
}
sink(y);
";

const SCHED_SRC: &str = "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0 };
    place produce on host;
    place blur3   on w0;
    place sink    on host;
    transfer x : sync;
    transfer y : sync;
    loop n : reuse;
}
";

fn lower(
    scratch: &Path,
) -> (
    PathBuf,
    NameTables,
    nucleus_compiler::sidecar::NameSidecar,
    std::collections::BTreeMap<
        nucleus_compiler::event::WorkerId,
        Vec<nucleus_compiler::event::Event>,
    >,
) {
    let kernels_path = scratch.join("kernels.rs");
    fs::write(
        &kernels_path,
        "pub fn produce() -> Vec<i32> { vec![0; 16] }\n\
         pub fn blur3(a: i32, b: i32, c: i32) -> i32 { (a + b + c) / 3 }\n\
         pub fn sink(_y: Vec<i32>) {}\n",
    )
    .expect("write kernels stub");

    let algo_ast = parse_algo(ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    // Apply reuse_inference so sidecar.reuse_widths is populated.
    // Without this, the codegen path is a no-op (reuse_widths empty)
    // and the test would not exercise the TASK-0284 wiring.
    let acfg = apply_reuse_inference(&linked, acfg).expect("apply_reuse_inference");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);
    (kernels_path, names, sidecar, per_worker)
}

#[test]
fn mp_tcp_bufsync_worker_emit_contains_reuse_buffer_codegen() {
    let scratch = scratch_dir("reuse_worker_shape");
    let (kernels_path, names, sidecar, per_worker) = lower(&scratch);
    let out_dir = scratch.join("gen");
    mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-bufsync emit");

    let w0_src = fs::read_to_string(out_dir.join("src/bin/w0.rs")).expect("read w0.rs");
    let host_src = fs::read_to_string(out_dir.join("src/bin/host.rs")).expect("read host.rs");

    // PRESENCE: the worker bin MUST contain the reuse buffer
    // declaration. Pre-TASK-0284 mp-tcp-bufsync's Plan::render_events
    // skipped the call entirely on both arms; the emit had no
    // `__reuse_buf` substring at all. Buffer name carries `_g0` suffix
    // uniformly post-TASK-0282 (single outer-axes pattern in this 1D
    // fixture — the empty outer tuple — so only g0 is emitted).
    assert!(
        w0_src.contains("let mut __reuse_buf_x_a0_g0: Vec<i32>"),
        "TASK-0284: mp-tcp-bufsync's w0 emit MUST contain the reuse \
         buffer declaration for x on axis 0 (the schedule carries \
         `loop n : reuse;` and the worker body reads x[n-1], x[n], \
         x[n+1]). If absent, mp-tcp-bufsync's Plan::render_events \
         regressed to the pre-cycle-107 silent-sibling state. Got:\n{w0_src}"
    );

    // PRESENCE: the per-iter update / rewrites use `rem_euclid(3_i64)`
    // (the slot-wrap math). Same canary as the shared-walker tests.
    assert!(
        w0_src.contains("rem_euclid(3_i64)"),
        "TASK-0284: mp-tcp-bufsync's w0 emit MUST contain the \
         `rem_euclid(3_i64)` circular-buffer slot expression. Got:\n{w0_src}"
    );

    // PRESENCE: the cross-backend marker substring is preserved as the
    // first-layer regression canary.
    assert!(
        w0_src.contains("reuse_widths_pending"),
        "TASK-0284: mp-tcp-bufsync's w0 emit MUST contain the \
         `reuse_widths_pending` marker (preserved across all backends \
         that consume the shared reuse-codegen helpers). Got:\n{w0_src}"
    );

    // PRESENCE: the blur3 call inside the body MUST read at least one
    // arg via the buffer. All three reads x[n±1] share the empty
    // outer-axes set, so post-TASK-0282 all three rewrite to the same
    // `_g0` buffer (the empty outer tuple is a single unique pattern,
    // hence a single group). Asserts the rewrite path is reached
    // inside Fire args.
    assert!(
        w0_src.contains("kernels::blur3(__reuse_buf_x_a0_g0["),
        "TASK-0284: mp-tcp-bufsync's blur3 call MUST contain reuse-buffer \
         reads (the rewrite path in `try_rewrite_reuse_arg` consults \
         ctx.reuse_active which is populated by the cycle-107 wiring). \
         Got:\n{w0_src}"
    );

    // ABSENCE: the host emit MUST NOT contain reuse codegen — host has
    // no Fire on `n`-loop-bound DataRefs. The host Push/Wait for x and
    // y crosses no reuse-tagged loop on the host side.
    assert!(
        !host_src.contains("__reuse_buf_"),
        "TASK-0284: host emit MUST NOT contain reuse buffer codegen — \
         host's responsibilities (produce, sink, Push x, Wait y) do \
         not include the reuse loop. If `__reuse_buf_` appears, the \
         walker is over-eagerly emitting on host events. Got:\n{host_src}"
    );
}
