//! Multi-worker pthreads-sync codegen tests (TASK-0122 AC #1).
//!
//! Drives a synthetic two-worker pingpong scenario end-to-end:
//! parse + lower + link + ACFG + sync + transfer injection -> emit
//! to a tempdir -> `cargo build` the generated project -> run the
//! binary -> diff its output against an expected value.
//!
//! The driving algorithm here is *not* example 02 (that lives in
//! `nucleus/compiler/tests/e2e_example_02.rs`); we use a smaller,
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

use compiler::{
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
    let acfg = inject_transfers(&linked, acfg);

    let out_dir = scratch.join("gen");
    // TASK-0124 contract path: project to per-worker EventList +
    // build sidecar + reverse name tables, exactly as the driver.
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
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
    let result =
        emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir).expect("emit succeeded");

    // Verify the generated main.rs structurally mentions the
    // expected primitives.
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();
    for needle in &[
        "thread::spawn",
        "Slot<Vec<i32>>",
        "Barrier::new",
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
// a barrier between a cross-worker writer and the next reader:
//
//   produce_a/b       on host
//   pa <-- inc_a(a)   on w0   -- host wrote `a`         => Sync {host,w0}
//   pb <-- inc_b(b)   on w1   -- w0 wrote pa, w1 reads b => Sync {w0,w1}
//   sink2(pa, pb)     on host -- w1 wrote pb            => Sync {host,w1}
//
// Three barriers with THREE DIFFERENT participant sets — {host,w0},
// {w0,w1}, {host,w1} — a maximally non-uniform / partial barrier set,
// including one ({w0,w1}) that does NOT involve the host at all.
// Before TASK-0172 the backend recovered barrier id by per-worker
// pre-order Sync index: e.g. w0 saw [{host,w0},{w0,w1}] (idx 0,1)
// while w1 saw [{w0,w1},{host,w1}] (idx 0,1) — w0#0 = {host,w0} but
// w1#0 = {w0,w1}, a participant-set disagreement at the same index ->
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
    use compiler::event::{Event, SyncTag, WorkerId};
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
    let acfg = inject_transfers(&linked, acfg);

    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
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

    // The three barriers, with the three different participant sets
    // this schedule's sync-injection rules produce. If the rules
    // drift, these lookups fail loudly (the test would otherwise stop
    // exercising the partial-barrier path).
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
    let tag_hw1 = only_tag(&set_hw1);

    // Three genuinely distinct barriers => three distinct SyncTags.
    let tags: BTreeSet<SyncTag> = [tag_hw0, tag_w0w1, tag_hw1].into_iter().collect();
    assert_eq!(
        tags.len(),
        3,
        "three distinct barriers must carry three distinct SyncTags; \
         got hw0={tag_hw0:?} w0w1={tag_w0w1:?} hw1={tag_hw1:?}"
    );
    // And it is genuinely non-uniform (participant sets differ).
    assert!(
        set_hw0 != set_w0w1 && set_w0w1 != set_hw1 && set_hw0 != set_hw1,
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
    // The bar name is the SyncTag value. All three barriers are
    // 2-party here.
    let (b_hw0, b_w0w1, b_hw1) = (tag_hw0.0, tag_w0w1.0, tag_hw1.0);
    for (b, label) in [
        (b_hw0, "{host,w0}"),
        (b_w0w1, "{w0,w1}"),
        (b_hw1, "{host,w1}"),
    ] {
        assert!(
            main_rs
                .contains(&format!("let bar_{b}: Arc<Barrier> = Arc::new(Barrier::new(2))")),
            "expected a 2-party Barrier for the {label} barrier (tag {b}):\n{main_rs}"
        );
    }
    // Per-worker wiring. Spawned workers clone-capture bars as
    // `w0_bar_<tag>` / `w1_bar_<tag>`; the host uses bare `bar_<tag>`.
    // w0 participates in {host,w0} and {w0,w1}, NOT {host,w1}.
    assert!(
        main_rs.contains(&format!("w0_bar_{b_hw0}.wait()")),
        "w0 must barrier on {{host,w0}}:\n{main_rs}"
    );
    assert!(
        main_rs.contains(&format!("w0_bar_{b_w0w1}.wait()")),
        "w0 must barrier on {{w0,w1}}:\n{main_rs}"
    );
    assert!(
        !main_rs.contains(&format!("w0_bar_{b_hw1}.wait()")),
        "w0 must NOT barrier on {{host,w1}}:\n{main_rs}"
    );
    // w1 participates in {w0,w1} and {host,w1}, NOT {host,w0}.
    assert!(
        main_rs.contains(&format!("w1_bar_{b_w0w1}.wait()")),
        "w1 must barrier on {{w0,w1}}:\n{main_rs}"
    );
    assert!(
        main_rs.contains(&format!("w1_bar_{b_hw1}.wait()")),
        "w1 must barrier on {{host,w1}}:\n{main_rs}"
    );
    assert!(
        !main_rs.contains(&format!("w1_bar_{b_hw0}.wait()")),
        "w1 must NOT barrier on {{host,w0}}:\n{main_rs}"
    );
    // The host participates in {host,w0} and {host,w1}, NOT {w0,w1}.
    // (Host body uses bare `bar_<tag>`; assert it does NOT wait the
    // w0<->w1 barrier — a tighter check than counts alone.)
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
