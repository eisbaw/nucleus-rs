//! Integration tests for the link pass (TASK-0011).
//!
//! Test strategy mirrors `algo_lower.rs` / `sched_lower.rs`:
//!
//! - Positive: every existing (algorithm, schedule) pair that currently
//!   parses and lowers cleanly must also link cleanly. The matrix is
//!   13-cnn-inference × {naive, batch_parallel, pipeline_parallel}
//!   and 14-hearing-aid × {naive}.
//!   `14-hearing-aid/embedded_multimcu.sched.nuc` is excluded because
//!   TASK-0079 has it failing parse, so there is no AST to lower or
//!   link. `05-stencil/*` is excluded because TASK-0078 has its
//!   algorithm failing parse for the same reason.
//!
//! - Negative: at least one hand-written invalid (algorithm, schedule)
//!   pair per [`LinkError`] variant. Inline sources for terseness; the
//!   parse + lower stages are not the system under test here.

use compiler::algo::{lower_algo, parse_algo};
use compiler::sched::{lower_sched, parse_sched};
use compiler::{link, LinkError};

/// Reads a source file at a workspace-relative path. Panics on IO
/// failure — these tests are environment-dependent by design (mirrors
/// the `read_example` helper in `algo_lower.rs`).
fn read_example(relpath: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let full = repo_root.join("nuc-nucleus").join("examples").join(relpath);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", full.display(), e))
}

/// Parse + lower an algorithm source string. Panics on failure — the
/// link tests assume the parser/lowering layers are working.
fn algo_from_str(src: &str) -> compiler::algo::AlgoIR {
    let ast = parse_algo(src).expect("algo must parse");
    lower_algo(&ast).expect("algo must lower")
}

/// Parse + lower a schedule source string.
fn sched_from_str(src: &str) -> compiler::sched::SchedIR {
    let ast = parse_sched(src).expect("sched must parse");
    lower_sched(&ast).expect("sched must lower")
}

// --------------------------------------------------------------------
// Positive: existing example pairs link cleanly
// --------------------------------------------------------------------

fn link_example(algo_rel: &str, sched_rel: &str) {
    let algo = algo_from_str(&read_example(algo_rel));
    let sched = sched_from_str(&read_example(sched_rel));
    match link(algo, sched) {
        Ok(_) => {}
        Err(errs) => panic!("{algo_rel} + {sched_rel}: link must succeed; got errors: {errs:?}"),
    }
}

#[test]
fn links_01_elementwise_add_naive() {
    // TASK-0013: end-to-end link of the smallest example.
    link_example(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
}

#[test]
fn links_13_cnn_naive() {
    link_example(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/naive.sched.nuc",
    );
}

#[test]
fn links_13_cnn_batch_parallel() {
    link_example(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    );
}

#[test]
fn links_13_cnn_pipeline_parallel() {
    link_example(
        "13-cnn-inference/prog.algo.nuc",
        "13-cnn-inference/schedules/pipeline_parallel.sched.nuc",
    );
}

#[test]
fn links_14_hearing_aid_naive() {
    link_example(
        "14-hearing-aid/prog.algo.nuc",
        "14-hearing-aid/schedules/naive.sched.nuc",
    );
}

/// Spot-check the resolved derived data for the CNN batch-parallel
/// case: `input` flows host -> {w0,w1,w2,w3}; `feat1`/`feat2` flow
/// within {w0..w3} (no transfer needed); `output` flows {w0..w3} ->
/// host (transfer declared). The link must succeed AND the derived
/// maps should reflect this.
#[test]
fn derived_data_for_cnn_batch_parallel() {
    let algo = algo_from_str(&read_example("13-cnn-inference/prog.algo.nuc"));
    let sched = sched_from_str(&read_example(
        "13-cnn-inference/schedules/batch_parallel.sched.nuc",
    ));
    let linked = link(algo, sched).expect("must link");

    // Every algorithm kernel has a placement.
    assert_eq!(linked.placements.len(), linked.algo.kernels.len());

    // load_input is on host (singleton).
    assert_eq!(linked.kernel_workers["load_input"].display(), "{host}");
    // conv_block_1 is on {w0,w1,w2,w3} (sorted set display).
    assert_eq!(
        linked.kernel_workers["conv_block_1"].display(),
        "{w0,w1,w2,w3}"
    );

    // input is produced on host (load_input) and consumed by
    // conv_block_1 on {w0..w3}.
    let input_prod = &linked.data_producers["input"];
    assert_eq!(input_prod.display(), "{host}");
    let input_cons = &linked.data_consumers["input"];
    assert_eq!(input_cons.len(), 1);
    let cons_iter: Vec<_> = input_cons.iter().collect();
    assert_eq!(cons_iter[0].display(), "{w0,w1,w2,w3}");
}

// --------------------------------------------------------------------
// Negative tests — one per LinkError variant
// --------------------------------------------------------------------

#[test]
fn negative_unknown_kernel() {
    // Algorithm has kernel `foo`. Schedule places `bar`.
    let algo = algo_from_str(
        "\
kernel foo : () -> () effectful;
foo();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place bar on host;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    // Must contain UnknownKernel(bar). Also UnplacedKernel(foo) is
    // expected (foo is declared but not placed) — that's a feature
    // of the multi-error one-pass policy, not a bug.
    assert!(
        errs.contains(&LinkError::UnknownKernel("bar".into())),
        "want UnknownKernel(bar) in {errs:?}"
    );
    assert!(
        errs.contains(&LinkError::UnplacedKernel("foo".into())),
        "want UnplacedKernel(foo) in {errs:?}"
    );
}

#[test]
fn negative_unplaced_kernel() {
    // Algorithm declares two kernels; schedule places only one.
    let algo = algo_from_str(
        "\
kernel a : () -> () effectful;
kernel b : () -> () effectful;
a();
b();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place a on host;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(errs, vec![LinkError::UnplacedKernel("b".into())]);
}

#[test]
fn negative_unknown_data() {
    // place_data references a data symbol that doesn't exist.
    let algo = algo_from_str(
        "\
kernel k : () -> () effectful;
k();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    memory_region r { size = 4KB; };
    workers = { host };
    place k on host;
    place_data ghost in r;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(errs, vec![LinkError::UnknownData("ghost".into())]);
}

#[test]
fn negative_unknown_loop() {
    // loop directive names a loop var the algorithm doesn't have.
    let algo = algo_from_str(
        "\
kernel k : () -> () effectful;
k();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    loop y : block=64;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(errs, vec![LinkError::UnknownLoop("y".into())]);
}

#[test]
fn negative_unknown_loop_via_check() {
    // `check loop VAR` is the other surface that names an algorithm
    // loop variable; same error variant.
    let algo = algo_from_str(
        "\
kernel k : () -> () effectful;
k();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    check loop n : latency_max = 10ms;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(errs, vec![LinkError::UnknownLoop("n".into())]);
}

#[test]
fn negative_unknown_transfer_data() {
    let algo = algo_from_str(
        "\
kernel k : () -> () effectful;
k();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    transfer phantom : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(errs, vec![LinkError::UnknownTransferData("phantom".into())]);
}

#[test]
fn negative_missing_cross_worker_transfer() {
    // Algorithm: a producer on one worker, consumer on another. No
    // transfer directive.
    let algo = algo_from_str(
        "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel make_x : () -> f32[N] pure;
kernel use_x : (f32[N]) -> f32[N] pure;
x <-- make_x();
y <-- use_x(x);
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0 };
    place make_x on host;
    place use_x  on w0;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(
        errs,
        vec![LinkError::MissingCrossWorkerTransfer {
            data: "x".into(),
            producer_worker: "{host}".into(),
            consumer_worker: "{w0}".into(),
        }]
    );
}

#[test]
fn no_transfer_required_within_same_worker() {
    // Sanity: a producer and consumer on the SAME worker need no
    // transfer directive.
    let algo = algo_from_str(
        "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel make_x : () -> f32[N] pure;
kernel use_x : (f32[N]) -> f32[N] pure;
x <-- make_x();
y <-- use_x(x);
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place make_x on host;
    place use_x  on host;
}
",
    );
    link(algo, sched).expect("same-worker dataflow needs no transfer");
}

#[test]
fn no_transfer_required_within_same_distributed_set() {
    // PRD/instructions: distributed placement = treat the worker set
    // as one entity for the cross-worker check.
    let algo = algo_from_str(
        "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel make_x : () -> f32[N] pure;
kernel use_x : (f32[N]) -> f32[N] pure;
x <-- make_x();
y <-- use_x(x);
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { w0, w1, w2, w3 };
    place make_x on { w0, w1, w2, w3 };
    place use_x  on { w0, w1, w2, w3 };
}
",
    );
    link(algo, sched).expect("same distributed set needs no transfer");
}

#[test]
fn multi_error_one_pass() {
    // Demonstrate the "collect all errors in one pass" policy:
    // a single link call surfaces multiple distinct variants.
    let algo = algo_from_str(
        "\
kernel a : () -> () effectful;
kernel b : () -> () effectful;
a();
b();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place a on host;
    transfer phantom : sync;
    loop z : block=8;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    // Expect UnplacedKernel(b), UnknownTransferData(phantom),
    // UnknownLoop(z) all reported together.
    assert!(errs.contains(&LinkError::UnplacedKernel("b".into())));
    assert!(errs.contains(&LinkError::UnknownTransferData("phantom".into())));
    assert!(errs.contains(&LinkError::UnknownLoop("z".into())));
    assert!(
        errs.len() >= 3,
        "expected ≥3 errors in one pass, got {errs:?}"
    );
}

#[test]
fn cross_worker_transfer_present_links() {
    // Adding the transfer directive turns the negative case clean.
    let algo = algo_from_str(
        "\
const N : usize = 4;
data x : f32[N];
data y : f32[N];
kernel make_x : () -> f32[N] pure;
kernel use_x : (f32[N]) -> f32[N] pure;
x <-- make_x();
y <-- use_x(x);
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0 };
    place make_x on host;
    place use_x  on w0;
    transfer x : sync;
}
",
    );
    link(algo, sched).expect("transfer-present cross-worker case must link");
}
