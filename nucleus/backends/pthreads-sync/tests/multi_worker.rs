//! Multi-worker pthreads-sync codegen tests (TASK-0122 AC #1).
//!
//! Drives a synthetic two-worker pingpong scenario end-to-end:
//! parse + lower + link + ACFG + sync + transfer injection -> emit
//! to a tempdir -> `cargo build` the generated project -> run the
//! binary -> diff its output against an expected value.
//!
//! The driving algorithm here is *not* example 02 (that lives in
//! `nucleus/nucleus-compiler/tests/e2e_example_02.rs`); we use a
//! smaller,
//! purpose-built two-worker case that fits in a unit-test file so
//! the multi-worker rejection / codegen path can be exercised in
//! isolation from the example matrix.
//!
//! Why a real example would also work: we could just point this
//! test at example 02, but that would duplicate the e2e_example_02
//! coverage. Here we stress a smaller scenario whose kernels are
//! self-contained — `produce` returns a constant `Vec<i32>`,
//! `consume` writes a checksum into `NUC_OUTPUT_PATH`.
//!
//! Limitation: building a fresh Cargo project per test is slow
//! (~1.5 s). We don't gate this behind `#[ignore]` because the
//! multi-worker codegen is M1's load-bearing capability — the test
//! must run on every commit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nucleus_compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
use pthreads_sync::{emit, NameTables};

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above pthreads-sync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/pthreads-sync-multi-worker-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Tiny synthetic algorithm: host produces, w0 consumes, host
/// summarises. Three Push/Wait pairs — exactly the AC #1 shape.
///
/// Dataflow:
///   x <-- produce_x();       // on host
///   y <-- produce_y();       // on host
///   z <-- combine(x, y);     // on w0  -- consumes x, y; produces z
///   sink(z);                 // on host -- consumes z (effect)
///
/// All three data symbols (x, y, z) cross workers; AC #1 names
/// "three Push/Wait pairs".
const ALGO_SRC: &str = r#"
const N : usize = 16;

data x : i32[N];
data y : i32[N];
data z : i32[N];

kernel produce_x : () -> i32[N] effectful;
kernel produce_y : () -> i32[N] effectful;
kernel combine   : (i32[N], i32[N]) -> i32[N] pure;
kernel sink      : (i32[N]) -> () effectful;

x <-- produce_x();
y <-- produce_y();
z <-- combine(x, y);
sink(z);
"#;

const SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0 };

    place produce_x on host;
    place produce_y on host;
    place combine   on w0;
    place sink      on host;

    transfer x : sync;
    transfer y : sync;
    transfer z : sync;
}
"#;

/// Hand-written kernels for the synthetic test. `produce_x` returns
/// [0, 1, ..., 15]; `produce_y` returns [100, 100, ..., 100];
/// `combine` is element-wise wrapping add; `sink` writes the
/// expected sum (in this case 1320 = 0+1+...+15 + 16*100) to
/// `NUC_OUTPUT_PATH` as a single little-endian i32.
const KERNELS_SRC: &str = r#"
use std::env;
use std::fs;
use std::io::Write;

const N: usize = 16;

pub fn produce_x() -> Vec<i32> {
    (0..N as i32).collect()
}

pub fn produce_y() -> Vec<i32> {
    vec![100; N]
}

pub fn combine(x: Vec<i32>, y: Vec<i32>) -> Vec<i32> {
    assert_eq!(x.len(), N);
    assert_eq!(y.len(), N);
    let mut out = Vec::with_capacity(N);
    for i in 0..N {
        out.push(x[i].wrapping_add(y[i]));
    }
    out
}

pub fn sink(z: Vec<i32>) {
    assert_eq!(z.len(), N);
    let sum: i32 = z.iter().copied().sum();
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut f = fs::File::create(&path).expect("sink: cannot create output file");
    f.write_all(&sum.to_le_bytes()).expect("sink: write failed");
}
"#;

/// Expected sum: 0+1+...+15 + 16*100 = 120 + 1600 = 1720.
const EXPECTED_SUM: i32 = 120 + 1600;

#[test]
fn two_worker_pingpong_compiles_and_runs() {
    let scratch = scratch_dir("two_worker_pingpong");

    // Write the algo / sched / kernels.rs to the scratch dir so the
    // backend has real files to point at.
    let algo_path = scratch.join("prog.algo.nuc");
    let sched_path = scratch.join("prog.sched.nuc");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&algo_path, ALGO_SRC).unwrap();
    fs::write(&sched_path, SCHED_SRC).unwrap();
    fs::write(&kernels_path, KERNELS_SRC).unwrap();

    // Drive the pipeline.
    let algo_ast = parse_algo(ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    let out_dir = scratch.join("gen");
    // TASK-0124 contract path: project to per-worker EventList +
    // build sidecar + reverse name tables, exactly as the driver.
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    // TASK-0238 (cycle 25): the 5-field NameTables literal collapsed
    // to the centralized constructor.
    let names = NameTables::from_acfg(&acfg);
    let result =
        emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir).expect("emit succeeded");

    // Verify the generated main.rs structurally mentions the
    // expected primitives.
    //
    // TASK-0218: the bare-Operation pingpong fixture
    // (produce_x/y → combine → sink, no Repeat) now has ZERO
    // sync_inject barriers — every cross-worker Sequence-rule barrier
    // is between a Push and its matching Wait and is elided. So
    // `Barrier::new` is no longer expected in the emitted code for
    // THIS fixture. The pthreads-sync Barrier emission path is
    // exercised by the partial_nonuniform_barrier_multi_worker test
    // below (whose fixture KEEPS barriers — see comments there) and
    // by real e2e cells (02-split-add__split__pthreads-sync via its
    // Repeat-entry sync, etc.).
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();
    for needle in &[
        "thread::spawn",
        "Slot<Vec<i32>>",
        ".wait()",
        ".push(",
        "kernels::produce_x",
        "kernels::combine",
        "kernels::sink",
    ] {
        assert!(
            main_rs.contains(needle),
            "main.rs missing expected snippet `{needle}`:\n{main_rs}",
        );
    }

    // Build the generated project.
    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build on generated project");
    assert!(
        build.status.success(),
        "generated project failed to build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    // Run the binary; sink will write the i32 sum to out_path.
    let out_bin = out_dir.join("output.bin");
    let exe = out_dir.join("target/release/nuc-generated");
    assert!(exe.exists(), "expected binary at {}", exe.display());
    let run = Command::new(&exe)
        .env("NUC_OUTPUT_PATH", &out_bin)
        .output()
        .expect("run generated binary");
    assert!(
        run.status.success(),
        "generated binary returned non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let bytes = fs::read(&out_bin).expect("read output.bin");
    assert_eq!(
        bytes.len(),
        4,
        "expected 4-byte i32 sum, got {}",
        bytes.len()
    );
    let got = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    assert_eq!(got, EXPECTED_SUM, "pingpong sum mismatch");
}

// --------------------------------------------------------------------
// TASK-0172 AC#3 / AC#7: a GENUINE partial / non-uniform-barrier
// multi-worker schedule lowers correctly.
//
// Three workers (host, w0, w1). The sync-injection Sequence rule puts
// a barrier between a cross-worker writer and the next reader. Under
// the pre-TASK-0218 over-syncing rule there were THREE barriers; with
// the TASK-0218 elision (between a Push and its matching Wait) only
// the boundaries WITHOUT a shared dataflow symbol survive:
//
//   produce_a/b       on host
//   pa <-- inc_a(a)   on w0   -- prev=produce_b writes `b`;
//                                inc_a reads `a` (no overlap)
//                                => Sync {host,w0} survives
//   pb <-- inc_b(b)   on w1   -- prev=inc_a writes `pa`;
//                                inc_b reads `b` (no overlap)
//                                => Sync {w0,w1} survives  (HOST-EXCLUDING)
//   sink2(pa, pb)     on host -- prev=inc_b writes `pb`;
//                                sink2 reads `pa, pb` (pb overlaps)
//                                => future Push/Wait for `pb` covers
//                                   the rendezvous; barrier ELIDED.
//
// Two barriers with TWO DIFFERENT participant sets — {host,w0},
// {w0,w1} — a non-uniform / partial barrier set INCLUDING the
// critical {w0,w1} HOST-EXCLUDING barrier (the case the pre-TASK-0172
// pre-order-index heuristic mis-aligned).
//
// Before TASK-0172 the backend recovered barrier id by per-worker
// pre-order Sync index: e.g. w0 saw [{host,w0},{w0,w1}] (idx 0,1)
// while w1 saw [{w0,w1}] (idx 0) — w0#0 = {host,w0} but w1#0 =
// {w0,w1}, a participant-set disagreement at the same index ->
// EmitError::ContractGap (the schedule was rejected, not a wrong
// binary). With the contract-carried SyncTag every participant of one
// barrier carries the SAME tag, so each barrier resolves
// independently and the schedule lowers to a correct barrier graph.
//
// NOTE this is a pthreads-sync-only test: the {w0,w1} barrier needs a
// w0<->w1 rendezvous, which pthreads-sync (shared-memory threads,
// std::sync::Barrier) does fine, but mp-tcp-bufsync deliberately
// cannot (one-stream-per-(host,worker); a host-excluding barrier is
// the SEPARATE genuine TASK-0175 transport limitation, retained as a
// typed ContractGap there — unrelated to partial-barrier identity).
const PARTIAL_ALGO_SRC: &str = r#"
const N : usize = 8;

data a  : i32[N];
data b  : i32[N];
data pa : i32[N];
data pb : i32[N];

kernel produce_a : () -> i32[N] effectful;
kernel produce_b : () -> i32[N] effectful;
kernel inc_a     : (i32[N]) -> i32[N] pure;
kernel inc_b     : (i32[N]) -> i32[N] pure;
kernel sink2     : (i32[N], i32[N]) -> () effectful;

a  <-- produce_a();
b  <-- produce_b();
pa <-- inc_a(a);
pb <-- inc_b(b);
sink2(pa, pb);
"#;

const PARTIAL_SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0, w1 };

    place produce_a on host;
    place produce_b on host;
    place inc_a     on w0;
    place inc_b     on w1;
    place sink2     on host;

    transfer a  : sync;
    transfer b  : sync;
    transfer pa : sync;
    transfer pb : sync;
}
"#;

const PARTIAL_KERNELS_SRC: &str = r#"
use std::env;
use std::fs;
use std::io::Write;

const N: usize = 8;

pub fn produce_a() -> Vec<i32> {
    (0..N as i32).collect()           // [0,1,...,7], sum 28
}
pub fn produce_b() -> Vec<i32> {
    vec![10; N]                       // [10;8], sum 80
}
pub fn inc_a(a: Vec<i32>) -> Vec<i32> {
    a.into_iter().map(|x| x + 1).collect()   // sum 28 + 8 = 36
}
pub fn inc_b(b: Vec<i32>) -> Vec<i32> {
    b.into_iter().map(|x| x + 1).collect()   // sum 80 + 8 = 88
}
pub fn sink2(pa: Vec<i32>, pb: Vec<i32>) {
    let sum: i32 = pa.iter().chain(pb.iter()).copied().sum();
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut f = fs::File::create(&path).expect("sink2: cannot create output file");
    f.write_all(&sum.to_le_bytes()).expect("sink2: write failed");
}
"#;

/// 36 (pa) + 88 (pb) = 124.
const PARTIAL_EXPECTED_SUM: i32 = 124;

#[test]
fn partial_nonuniform_barrier_multi_worker_lowers_correctly() {
    use nucleus_compiler::event::{Event, SyncTag, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    let scratch = scratch_dir("partial_nonuniform_barrier");
    let algo_path = scratch.join("prog.algo.nuc");
    let sched_path = scratch.join("prog.sched.nuc");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&algo_path, PARTIAL_ALGO_SRC).unwrap();
    fs::write(&sched_path, PARTIAL_SCHED_SRC).unwrap();
    fs::write(&kernels_path, PARTIAL_KERNELS_SRC).unwrap();

    let algo_ast = parse_algo(PARTIAL_ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(PARTIAL_SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    // TASK-0238 (cycle 25): the 5-field NameTables literal collapsed
    // to the centralized constructor.
    let names = NameTables::from_acfg(&acfg);

    // --- Contract-level checks (TASK-0172 AC#1/#6): the SyncTag is a
    //     genuine cross-worker join key. ---
    //
    // Reverse the worker name table so we can refer to host/w0/w1.
    let wid = |nm: &str| -> WorkerId {
        *acfg
            .name_workers
            .iter()
            .find(|(n, _)| n.as_str() == nm)
            .map(|(_, i)| i)
            .unwrap_or_else(|| panic!("worker `{nm}` not in name table"))
    };
    let (host, w0, w1) = (wid("host"), wid("w0"), wid("w1"));

    // For each SyncTag, the union of participant sets recorded by
    // every worker that carries it. If the tag is a real join key,
    // every participant records the SAME set, so the union == that
    // set, and the tag appears in exactly its participants' lists.
    let mut by_tag: BTreeMap<SyncTag, BTreeSet<WorkerId>> = BTreeMap::new();
    let mut carriers: BTreeMap<SyncTag, BTreeSet<WorkerId>> = BTreeMap::new();
    for (w, evs) in &per_worker {
        for e in evs {
            if let Event::Sync {
                participants, sync, ..
            } = e
            {
                by_tag
                    .entry(*sync)
                    .or_default()
                    .extend(participants.iter().copied());
                carriers.entry(*sync).or_default().insert(*w);
            }
        }
    }

    // The two barriers, with the two different participant sets
    // this schedule's sync-injection rules produce (TASK-0218: the
    // third {host,w1} barrier is now elided as redundant with the
    // Push/Wait pair for `pb`; see the module comment block above).
    // If the rules drift, these lookups fail loudly (the test would
    // otherwise stop exercising the partial-barrier path).
    let set_hw0: BTreeSet<WorkerId> = [host, w0].into_iter().collect();
    let set_w0w1: BTreeSet<WorkerId> = [w0, w1].into_iter().collect();
    let set_hw1: BTreeSet<WorkerId> = [host, w1].into_iter().collect();

    let only_tag = |want: &BTreeSet<WorkerId>| -> SyncTag {
        let hits: Vec<SyncTag> = by_tag
            .iter()
            .filter(|(_, p)| *p == want)
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one barrier with participants {want:?}; \
             by_tag={by_tag:?}"
        );
        hits[0]
    };
    let tag_hw0 = only_tag(&set_hw0);
    let tag_w0w1 = only_tag(&set_w0w1);

    // TASK-0218: the {host,w1} barrier is now elided (Push/Wait for
    // `pb` covers the rendezvous between inc_b and sink2). Assert
    // its ABSENCE explicitly so a regression that re-introduces it
    // (or another barrier-emitting bug) shows up here.
    assert!(
        !by_tag.values().any(|p| p == &set_hw1),
        "TASK-0218: {{host,w1}} barrier must be elided (Push/Wait for `pb` \
         covers inc_b->sink2 rendezvous); by_tag={by_tag:?}"
    );

    // Two genuinely distinct barriers => two distinct SyncTags.
    let tags: BTreeSet<SyncTag> = [tag_hw0, tag_w0w1].into_iter().collect();
    assert_eq!(
        tags.len(),
        2,
        "two distinct barriers must carry two distinct SyncTags; \
         got hw0={tag_hw0:?} w0w1={tag_w0w1:?}"
    );
    // And it is genuinely non-uniform (participant sets differ;
    // includes the critical host-excluding {w0,w1} case).
    assert_ne!(
        set_hw0, set_w0w1,
        "this test must exercise a non-uniform barrier set"
    );

    // The SyncTag is a genuine cross-worker join key: it is carried by
    // EXACTLY the workers in the barrier, and every carrier agrees on
    // the participant set. This is precisely the property the old
    // pre-order-index heuristic could NOT provide for a partial
    // barrier (and rejected with EmitError::ContractGap).
    for (tag, parts) in &by_tag {
        assert_eq!(
            carriers.get(tag),
            Some(parts),
            "SyncTag {tag:?}: carriers must equal participants \
             (a genuine cross-worker join key); carriers={carriers:?}"
        );
    }

    // --- AC#3/#7: lowering must SUCCEED (no ContractGap) on the
    //     MULTI-worker path, and the barrier wiring must be correct. ---
    let out_dir = scratch.join("gen");
    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("partial-barrier multi-worker emit must succeed (no ContractGap)");

    let main_rs = fs::read_to_string(&result.main_rs).unwrap();
    // The bar name is the SyncTag value. Both surviving barriers are
    // 2-party here. TASK-0218: the {host,w1} barrier is elided.
    let (b_hw0, b_w0w1) = (tag_hw0.0, tag_w0w1.0);
    for (b, label) in [(b_hw0, "{host,w0}"), (b_w0w1, "{w0,w1}")] {
        assert!(
            main_rs.contains(&format!(
                "let bar_{b}: Arc<Barrier> = Arc::new(Barrier::new(2))"
            )),
            "expected a 2-party Barrier for the {label} barrier (tag {b}):\n{main_rs}"
        );
    }
    // Per-worker wiring. Spawned workers clone-capture bars as
    // `w0_bar_<tag>` / `w1_bar_<tag>`; the host uses bare `bar_<tag>`.
    // w0 participates in {host,w0} and {w0,w1}.
    assert!(
        main_rs.contains(&format!("w0_bar_{b_hw0}.wait()")),
        "w0 must barrier on {{host,w0}}:\n{main_rs}"
    );
    assert!(
        main_rs.contains(&format!("w0_bar_{b_w0w1}.wait()")),
        "w0 must barrier on {{w0,w1}}:\n{main_rs}"
    );
    // w1 participates in {w0,w1} only (TASK-0218: the {host,w1}
    // barrier is elided; w1 must NOT carry it).
    assert!(
        main_rs.contains(&format!("w1_bar_{b_w0w1}.wait()")),
        "w1 must barrier on {{w0,w1}}:\n{main_rs}"
    );
    assert!(
        !main_rs.contains(&format!("w1_bar_{b_hw0}.wait()")),
        "w1 must NOT barrier on {{host,w0}}:\n{main_rs}"
    );
    // The host participates in {host,w0} only (TASK-0218: the
    // {host,w1} barrier is elided). Assert it does NOT wait the
    // w0<->w1 barrier — a tighter check than counts alone.
    assert!(
        !main_rs.contains(&format!("    bar_{b_w0w1}.wait()")),
        "host must NOT barrier on {{w0,w1}}:\n{main_rs}"
    );

    // --- End-to-end: it must also build and produce the right value. ---
    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build on generated partial-barrier project");
    assert!(
        build.status.success(),
        "generated partial-barrier project failed to build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let out_bin = out_dir.join("output.bin");
    let exe = out_dir.join("target/release/nuc-generated");
    let run = Command::new(&exe)
        .env("NUC_OUTPUT_PATH", &out_bin)
        .output()
        .expect("run generated partial-barrier binary");
    assert!(
        run.status.success(),
        "generated partial-barrier binary returned non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let bytes = fs::read(&out_bin).expect("read output.bin");
    assert_eq!(bytes.len(), 4, "expected 4-byte i32, got {}", bytes.len());
    let got = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    assert_eq!(
        got, PARTIAL_EXPECTED_SUM,
        "partial-barrier program produced wrong result"
    );
}

// --------------------------------------------------------------------
// TASK-0052.05: check_frame codegen on the multi-worker path.
//
// Combines `partition=workers` + `check loop V : latency_max=T,
// on_violation=panic`. With partition=workers, each compute worker
// projects its OWN Event::Loop over the partitioned slice; each of
// those loops carries the same `check_frame`. The multi-worker
// renderer wraps EACH worker's loop in `let _check_start =
// Instant::now()` + body + `let _check_elapsed = ...as_nanos()` +
// panic dispatch — so a violation by ANY worker thread panics that
// thread, and the host's `handle.join().expect(..)` propagates the
// panic to the main thread (exit 101).
//
// Why this matters: the single-worker codegen and the multi-worker
// codegen live in DIFFERENT files (lib.rs vs multi_worker.rs); only
// the multi-worker path drives this combination of features.
// Pinning the emit-string shape catches drift between the two
// renderers (it's the same shape — same `Instant::now()` line,
// same elapsed read, same `panic!` format string).
// --------------------------------------------------------------------

const CHECK_ALGO_SRC: &str = r#"
const N : usize = 4;
data x : i32[N];
data y : i32[N];

kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel slow_inc    : (i32)    -> i32   pure;

x <-- load_input();
for n : 0 .. N {
    y[n] <-- slow_inc(x[n]);
}
save_output(y);
"#;

const CHECK_SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0, w1 };

    place load_input  on host;
    place save_output on host;
    place slow_inc    on { w0, w1 };

    loop n : partition=workers;

    transfer x : sync;
    transfer y : sync;

    // 1ns is unachievable: the kernel sleeps 1ms (see kernels), so
    // every iteration violates the budget. AC#3: both worker threads
    // panic independently with loop_var + measured + threshold in
    // the message.
    check loop n : latency_max = 1ns;
}
"#;

const CHECK_KERNELS_SRC: &str = r#"
use std::env;
use std::fs;
use std::io::Write;

const N: usize = 4;

pub fn load_input() -> Vec<i32> {
    (0..N as i32).collect()
}
pub fn slow_inc(x: i32) -> i32 {
    // Deliberately slow so the 1ns latency budget can never hold.
    std::thread::sleep(std::time::Duration::from_millis(1));
    x + 1
}
pub fn save_output(y: Vec<i32>) {
    let sum: i32 = y.iter().copied().sum();
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut f = fs::File::create(&path).expect("save_output: cannot create output file");
    f.write_all(&sum.to_le_bytes()).expect("save_output: write failed");
}
"#;

#[test]
fn multi_worker_check_loop_panics_per_thread_with_loop_var_and_numbers() {
    use nucleus_compiler::{apply_block_transforms, apply_partition_workers, inject_check_frames};
    let scratch = scratch_dir("multi_worker_check_loop_panic");
    let algo_path = scratch.join("prog.algo.nuc");
    let sched_path = scratch.join("prog.sched.nuc");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&algo_path, CHECK_ALGO_SRC).unwrap();
    fs::write(&sched_path, CHECK_SCHED_SRC).unwrap();
    fs::write(&kernels_path, CHECK_KERNELS_SRC).unwrap();

    let algo_ast = parse_algo(CHECK_ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(CHECK_SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block-transform");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition-workers");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    let per_worker = acfg_to_events(&acfg);
    let per_worker = inject_check_frames(per_worker, &linked.sched.checks, &acfg.name_iter_vars);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    // TASK-0238 (cycle 25): the 5-field NameTables literal collapsed
    // to the centralized constructor.
    let names = NameTables::from_acfg(&acfg);

    let out_dir = scratch.join("gen");
    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("multi-worker emit with check_frame must succeed (TASK-0052.05)");

    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // AC#1: the Event::Loop arm in multi_worker.rs wraps the body in
    // Instant::now() + elapsed compare + panic dispatch. The renderer
    // emits ONE `_check_start = Instant::now()` per projected Event::
    // Loop body — partition=workers projects the same source loop
    // onto each of {w0, w1}, so the rendered main.rs contains
    // exactly TWO occurrences (one per spawned worker; host is not
    // a participant of partition=workers).
    let start_count = main_rs
        .matches("let _check_start = std::time::Instant::now();")
        .count();
    assert_eq!(
        start_count, 2,
        "expected 2 `Instant::now()` instrumentation points (one per \
         partitioned worker w0/w1); got {start_count}.\nmain.rs:\n{main_rs}"
    );
    let elapsed_count = main_rs
        .matches("let _check_elapsed = _check_start.elapsed().as_nanos();")
        .count();
    assert_eq!(
        elapsed_count, 2,
        "expected 2 elapsed-nanos reads. Got {elapsed_count}.\nmain.rs:\n{main_rs}"
    );
    // The panic message embeds loop_var + threshold literal (matches
    // the single-worker emit shape — TASK-0052.02 byte-shape pin).
    let panic_count = main_rs
        .matches("panic!(\"latency budget violated on `check loop n`")
        .count();
    assert_eq!(
        panic_count, 2,
        "expected 2 panic-message sites (per-worker). Got {panic_count}.\nmain.rs:\n{main_rs}"
    );
    assert!(
        main_rs.contains("if _check_elapsed > 1_u128"),
        "panic guard must compare against the 1ns threshold literal.\nmain.rs:\n{main_rs}"
    );

    // AC#3: cargo-build + run; at least one worker thread must panic
    // and `handle.join().expect(..)` must propagate it (exit 101 with
    // a panic message naming the loop_var + measured + threshold).
    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build on generated multi-worker check_loop project");
    assert!(
        build.status.success(),
        "generated multi-worker check_loop project failed to build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let exe = out_dir.join("target/release/nuc-generated");
    assert!(exe.exists(), "expected binary at {}", exe.display());
    let out_bin = out_dir.join("output.bin");
    let run = Command::new(&exe)
        .env("NUC_OUTPUT_PATH", &out_bin)
        .output()
        .expect("run generated multi-worker check_loop binary");

    let stderr = String::from_utf8_lossy(&run.stderr).to_string();
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    // The generated Cargo.toml sets `[profile.release] panic = "abort"`
    // (see `Plan::emit` Cargo.toml template / nucleus default). Under
    // panic=abort a thread panic immediately aborts the WHOLE process
    // with SIGABRT — Rust does not unwind. So the binary terminates
    // either with:
    //   * exit code 101 (panic + unwind to main + main's panic on
    //     join().expect — only if the binary is built with panic=
    //     "unwind"), OR
    //   * signal-terminated by SIGABRT (exit code None) — the panic=
    //     "abort" path; the same signal the OS uses for `abort()`.
    // BOTH are valid evidence that a worker thread panicked; what
    // matters for the AC#3 assertion is the panic MESSAGE on stderr
    // (loop_var + measured + threshold) and that the process did NOT
    // exit cleanly. A clean exit (status == Some(0)) would mean the
    // panic dispatch silently no-op'd, which is the bug TASK-0052.05
    // exists to prevent.
    let exited_cleanly = matches!(run.status.code(), Some(0));
    assert!(
        !exited_cleanly,
        "multi-worker check_loop with tight threshold must NOT exit \
         cleanly — the worker threads' panic dispatch was silently \
         dropped. exit={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        run.status.code(),
    );
    // The worker-thread panic message must embed the loop_var + the
    // threshold literal. Measured ns is dynamic (whatever 1ms-of-
    // sleep elapsed; always >> 1ns).
    assert!(
        stderr.contains("latency budget violated on `check loop n`"),
        "stderr must contain the worker-thread panic naming the \
         loop_var.\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("max 1 ns"),
        "stderr must contain the threshold literal (`max 1 ns`).\nstderr:\n{stderr}"
    );
    // The measured ns must be > 1 (the kernel sleeps 1ms). Extract
    // from the message form "iteration took {N} ns, max 1 ns".
    let took = stderr
        .split("iteration took ")
        .nth(1)
        .and_then(|s| s.split(" ns").next())
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or_else(|| panic!("could not parse measured ns from:\n{stderr}"));
    assert!(
        took > 1,
        "measured ns ({took}) must exceed the 1ns threshold for the \
         panic to have fired."
    );
}

// --------------------------------------------------------------------
// TASK-0236 (cycle 23, 2026-05-22): multi-worker check_frame emit-string
// pinning for Log + Count on_violation kinds. These tests close the
// review-gate B.1 gap surfaced in cycle 22: the single-worker
// emit-string tests at check_frame_codegen.rs pin the SHARED helpers
// end-to-end via the single-worker call paths, but multi_worker.rs's
// own 4 call sites (Plan::emit's static + guard, render_worker_events
// Log + Count branches) were structurally byte-transparent (by
// shared-helper construction) without a direct pinning test.
//
// These two tests are EMIT-ONLY (no cargo build/run): they call emit()
// + read main.rs + assert the multi-worker shape literally. Fast,
// drift-detection focused. The slower build+run coverage already
// exists for Panic at multi_worker_check_loop_panics_per_thread_with_loop_var_and_numbers
// above; Log + Count don't need a runtime witness here (check_frame_codegen.rs
// already builds + runs them on the single-worker code path).
// --------------------------------------------------------------------

const CHECK_LOG_SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0, w1 };

    place load_input  on host;
    place save_output on host;
    place slow_inc    on { w0, w1 };

    loop n : partition=workers;

    transfer x : sync;
    transfer y : sync;

    check loop n : latency_max = 5ms, on_violation = log;
}
"#;

const CHECK_COUNT_SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0, w1 };

    place load_input  on host;
    place save_output on host;
    place slow_inc    on { w0, w1 };

    loop n : partition=workers;

    transfer x : sync;
    transfer y : sync;

    check loop n : latency_max = 5ms, on_violation = count;
}
"#;

/// Build the (per_worker, names, sidecar, kernels_path, out_dir)
/// tuple for the multi-worker check_loop schedule variant.
///
/// TASK-0237 (cycle 24): the lower-link-inject pipeline now lives in
/// the shared `test_common::lower_for_test` helper. This function is
/// thin glue that handles the scratch dir + file writes + the local
/// NameTables construction.
fn lower_multi_worker_check_schedule(
    sched_src: &str,
    scratch_name: &str,
) -> (
    std::collections::BTreeMap<nucleus_compiler::WorkerId, Vec<nucleus_compiler::event::Event>>,
    NameTables,
    nucleus_compiler::NameSidecar,
    PathBuf,
    PathBuf,
) {
    let scratch = scratch_dir(scratch_name);
    let algo_path = scratch.join("prog.algo.nuc");
    let sched_path = scratch.join("prog.sched.nuc");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&algo_path, CHECK_ALGO_SRC).unwrap();
    fs::write(&sched_path, sched_src).unwrap();
    fs::write(&kernels_path, CHECK_KERNELS_SRC).unwrap();

    let r = test_common::lower_for_test(
        CHECK_ALGO_SRC,
        sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: true,
            apply_partition_workers: true,
            inject_check_frames: true,
        },
    );
    (
        r.per_worker,
        r.names,
        r.sidecar,
        kernels_path,
        scratch.join("gen"),
    )
}

#[test]
fn multi_worker_check_loop_log_emit_pins_per_thread_eprintln_template() {
    // Mirrors single-worker check_frame_codegen::log_on_violation_codegen
    // shape but exercises the MULTI-WORKER code path (render_worker_events
    // Log branch via the shared `emit_log_branch` helper, TASK-0222).
    let (per_worker, names, sidecar, kernels_path, out_dir) =
        lower_multi_worker_check_schedule(CHECK_LOG_SCHED_SRC, "multi_worker_check_loop_log_emit");
    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("multi-worker emit with on_violation=log must succeed");
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // partition=workers projects the source loop onto BOTH w0 and w1;
    // each rendered Event::Loop carries the same check_frame. So the
    // multi-worker main.rs contains EXACTLY 2 eprintln template sites
    // (one per spawned worker; host is not a participant).
    let eprintln_count = main_rs
        .matches(
            "eprintln!(\"warning: check loop `n` violated latency_max=5000000 ns: iteration took {} ns\", _check_elapsed);",
        )
        .count();
    assert_eq!(
        eprintln_count, 2,
        "expected 2 Log eprintln sites in multi-worker main.rs (per-worker); \
         got {eprintln_count}. Shared template via emit_log_branch (TASK-0222) \
         must produce the SAME template across both workers.\nmain.rs:\n{main_rs}"
    );
    // No Panic site should appear (verifies Log dispatch beats Panic
    // in the multi-worker render_worker_events arm).
    assert!(
        !main_rs.contains("panic!(\"latency budget violated"),
        "multi-worker Log emit must NOT include the Panic template:\n{main_rs}"
    );
}

#[test]
fn multi_worker_check_loop_count_emit_pins_static_guard_and_fetch_add_templates() {
    // Mirrors single-worker check_frame_codegen::count_on_violation_codegen
    // shape but exercises the MULTI-WORKER path's THREE Count templates
    // (TASK-0052.05): file-scope static + per-loop guard local in fn main
    // + per-worker fetch_add branch. All three are emitted via the shared
    // helpers (TASK-0222).
    let (per_worker, names, sidecar, kernels_path, out_dir) = lower_multi_worker_check_schedule(
        CHECK_COUNT_SCHED_SRC,
        "multi_worker_check_loop_count_emit",
    );
    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("multi-worker emit with on_violation=count must succeed");
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // (a) ONE file-scope static (deduped by sanitized ident; both
    // workers share it under partition=workers — TASK-0052.05).
    let static_count = main_rs
        .matches(
            "static NUC_CHECK_COUNT_n: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);",
        )
        .count();
    assert_eq!(
        static_count, 1,
        "expected exactly 1 shared static AtomicU64 (deduped by ident across workers); \
         got {static_count}.\nmain.rs:\n{main_rs}"
    );

    // (b) ONE Drop guard local in fn main (host thread owns the
    // summary printing — Drop runs after all handle.join() returns).
    let guard_count = main_rs
        .matches("let _nuc_check_reporter_n = NucCheckCountReporter {")
        .count();
    assert_eq!(
        guard_count, 1,
        "expected exactly 1 NucCheckCountReporter guard local in fn main \
         (host thread owns the Drop summary); got {guard_count}.\nmain.rs:\n{main_rs}"
    );

    // (c) EXACTLY 2 fetch_add sites (one per spawned worker; host is
    // not a participant of partition=workers).
    let fetch_add_count = main_rs
        .matches("NUC_CHECK_COUNT_n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);")
        .count();
    assert_eq!(
        fetch_add_count, 2,
        "expected 2 fetch_add sites (per-worker); got {fetch_add_count}.\n\
         main.rs:\n{main_rs}"
    );

    // (d) The reporter struct definition appears ONCE at file scope.
    assert!(
        main_rs.contains("struct NucCheckCountReporter {"),
        "NucCheckCountReporter struct definition missing:\n{main_rs}"
    );
    // No Panic / Log dispatch leaked in (Count is exclusive).
    assert!(
        !main_rs.contains("panic!(\"latency budget violated"),
        "Count emit must NOT include Panic template:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("eprintln!(\"warning: check loop"),
        "Count emit must NOT include Log template:\n{main_rs}"
    );
}

// --------------------------------------------------------------------
// TASK-0245 (cycle 36): regression-pin for the cycle-35
// `render_int_expr` const-in-IndexExpr fix.
//
// Cycle 35 (commit 894f63f) fixed `pthreads_sync::render_int_expr` to
// resolve declared consts (e.g. `ITERS`) when they appear inside an
// `IndexExpr`. Examples 01..09/13 only used iter-vars (NOT consts)
// inside IndexExprs, so the bug was inert in the existing matrix;
// example 11's `grid[(t + ITERS) % (ITERS + 1)][i]` was the witness.
//
// This test pins the structural contract on the PTHREADS-SYNC backend
// — the home of the private `render_int_expr` — using the shared
// fixture from `test_common`. A sibling test for mp-tcp-bufsync lives
// in `mp-tcp-bufsync/tests/pingpong.rs`; the sibling for pthreads-
// async lives in `pthreads-async/tests/skeleton.rs`. All three sister
// tests consume the same `CONST_IN_INDEXEXPR_*` constants in
// `test_common`, so the fixture is single-sourced.
//
// What it pins:
//   1. The IndexExpr arithmetic site contains the resolved const
//      LITERAL (`8`) at the position `ITERS` occupied in the source.
//   2. The bare const ident (`ITERS`) does NOT appear anywhere in the
//      emitted `main.rs` — Rust does not have `ITERS` in scope; an
//      unresolved bare ident is the bug.
//
// What it does NOT do: it does NOT cargo-build the generated project.
// The slow build/run coverage already exists end-to-end at the e2e
// gate (`just e2e`) on example 11. This test's role is FAST
// drift-detection: a future cycle that copies a private parallel
// IndexExpr renderer into a backend without the `sidecar.consts`
// lookup fails this test immediately, before the e2e tally moves.
#[test]
fn const_in_indexexpr_pthreads_sync_resolves_to_literal_value() {
    let scratch = scratch_dir("const_in_indexexpr_pthreads_sync");
    let kernels_path = scratch.join("kernels.rs");
    // The kernels.rs file is not consumed by emit's RENDERER (it's
    // verbatim-copied into the generated project), but `emit()`
    // reads it from disk and fails fast if missing. A minimal stub
    // is enough — this test never builds the generated project.
    fs::write(&kernels_path, "// stub for emit-string test\n").unwrap();

    let r = test_common::lower_for_test(
        test_common::CONST_IN_INDEXEXPR_ALGO_SRC,
        test_common::CONST_IN_INDEXEXPR_SCHED_SRC,
        &test_common::LowerForTestOpts::default(),
    );

    let out_dir = scratch.join("gen");
    let result = emit(&r.per_worker, &r.names, &r.sidecar, &kernels_path, &out_dir)
        .expect("pthreads-sync emit must succeed on const-in-IndexExpr fixture");
    let main_rs = fs::read_to_string(&result.main_rs).expect("read main.rs");

    // (1) The resolved literal `8` appears at the IndexExpr site. The
    // fixture writes `y[ITERS][i]` (LHS on w0) and reads `y[ITERS][0]`
    // (RHS on host). With y typed `i32[ITERS+1][N]` (`ITERS+1=9` rows,
    // `N=4` cols), the flat-index spelling is `({ITERS}) * 4 + ({i})`
    // — substring `(8) * 4` is the load-bearing fingerprint of a
    // resolved const at the row-stride position. If `ITERS` rendered
    // as a bare ident, the substring would be `(ITERS) * 4` and the
    // assertion would fail.
    let iters_val = test_common::CONST_IN_INDEXEXPR_ITERS_VALUE;
    let resolved_row = format!("({iters_val}) * 4");
    assert!(
        main_rs.contains(&resolved_row),
        "pthreads-sync main.rs must contain the resolved `ITERS=8` literal at \
         the IndexExpr row-stride site (`{resolved_row}`); cycle-35 fix not \
         reaching this code path. main.rs:\n{main_rs}"
    );

    // (2) The bare const ident `ITERS` does NOT appear anywhere in
    // the emitted main.rs. Rust has no `ITERS` in scope; this is the
    // primary regression-pin — the bug cycle-35 fixed was precisely
    // the bare ident leaking through.
    let bare_ident = test_common::CONST_IN_INDEXEXPR_ITERS_IDENT;
    assert!(
        !main_rs.contains(bare_ident),
        "pthreads-sync main.rs must NOT contain the bare const ident \
         `{bare_ident}` — render_int_expr must resolve it via sidecar.consts \
         (cycle-35 fix; TASK-0245 audit). main.rs:\n{main_rs}"
    );
}
