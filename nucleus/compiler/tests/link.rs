//! Integration tests for the link pass (TASK-0011).
//!
//! Test strategy mirrors `algo_lower.rs` / `sched_lower.rs`:
//!
//! - Positive: every existing (algorithm, schedule) pair that currently
//!   parses and lowers cleanly must also link cleanly. The matrix
//!   includes 05-stencil × {naive, blocked, distributed},
//!   13-cnn-inference × {naive, batch_parallel, pipeline_parallel},
//!   and 14-hearing-aid × {naive}.
//!   `14-hearing-aid/embedded_multimcu.sched.nuc` is excluded by
//!   scope: it is a far-future M11 multi-MCU schedule, not part of
//!   the M3 lower/link matrix. (It now parses cleanly — TASK-0079
//!   reconciled its `check loop` form; covered by the parser test.
//!   Bringing it into the lower/link matrix is follow-up TASK-0192.)
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
fn links_02_split_add_naive() {
    // TASK-0021: smoke-test schedule — single worker, no transfers.
    link_example(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/naive.sched.nuc",
    );
}

#[test]
fn links_02_split_add_split() {
    // TASK-0021: the load-bearing two-worker link. Verifies that the
    // three `transfer` directives (a, b, c) satisfy the
    // MissingCrossWorkerTransfer check for the cross-worker edges
    // host -> w0 (inputs) and w0 -> host (output).
    //
    // If this test ever starts failing with MissingCrossWorkerTransfer,
    // re-read split.sched.nuc: any missing `transfer` for a data
    // symbol that crosses worker boundaries is a HARD compile error
    // per PRD §6.3.4, and is what this test pins.
    link_example(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
}

#[test]
fn derived_data_for_split_add() {
    // Spot-check that the link pass routes the data correctly:
    // - `a`, `b` produced on host, consumed on w0.
    // - `c` produced on w0, consumed on host.
    let algo = algo_from_str(&read_example("02-split-add/prog.algo.nuc"));
    let sched = sched_from_str(&read_example("02-split-add/schedules/split.sched.nuc"));
    let linked = link(algo, sched).expect("split must link");

    // Every algorithm kernel placed.
    assert_eq!(linked.placements.len(), linked.algo.kernels.len());

    // load_input / load_input_b / save_output on {host}; add on {w0}.
    assert_eq!(linked.kernel_workers["load_input"].display(), "{host}");
    assert_eq!(linked.kernel_workers["load_input_b"].display(), "{host}");
    assert_eq!(linked.kernel_workers["save_output"].display(), "{host}");
    assert_eq!(linked.kernel_workers["add"].display(), "{w0}");

    // Data flow: a, b produced by host kernels, consumed by add on w0.
    assert_eq!(linked.data_producers["a"].display(), "{host}");
    assert_eq!(linked.data_producers["b"].display(), "{host}");
    let a_cons: Vec<_> = linked.data_consumers["a"].iter().collect();
    assert_eq!(a_cons.len(), 1);
    assert_eq!(a_cons[0].display(), "{w0}");
    let b_cons: Vec<_> = linked.data_consumers["b"].iter().collect();
    assert_eq!(b_cons.len(), 1);
    assert_eq!(b_cons[0].display(), "{w0}");

    // c produced on w0, consumed by save_output on host.
    assert_eq!(linked.data_producers["c"].display(), "{w0}");
    let c_cons: Vec<_> = linked.data_consumers["c"].iter().collect();
    assert_eq!(c_cons.len(), 1);
    assert_eq!(c_cons[0].display(), "{host}");
}

#[test]
fn split_add_missing_transfer_is_link_error() {
    // The load-bearing negative case for the example: drop one of
    // the transfer directives and the link MUST fail with
    // MissingCrossWorkerTransfer. This pins the contract that
    // split.sched.nuc's three transfers are not decoration — each is
    // load-bearing.
    let algo = algo_from_str(&read_example("02-split-add/prog.algo.nuc"));
    // Inline a schedule identical to split.sched.nuc but with
    // `transfer c : sync;` removed.
    let sched = sched_from_str(
        "\
schedule for \"../prog.algo.nuc\" {
    workers = { host, w0 };
    place load_input    on host;
    place load_input_b  on host;
    place save_output   on host;
    place add           on w0;
    transfer a : sync;
    transfer b : sync;
    // transfer c : sync;   <-- deliberately removed
}
",
    );
    let errs = link(algo, sched).expect_err("dropped transfer must fail");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            LinkError::MissingCrossWorkerTransfer { data, .. } if data == "c"
        )),
        "expected MissingCrossWorkerTransfer(c); got {errs:?}"
    );
}

#[test]
fn links_03_reduction_naive() {
    // TASK-0022: naive schedule — single worker, no transfers.
    link_example(
        "03-reduction/prog.algo.nuc",
        "03-reduction/schedules/naive.sched.nuc",
    );
}

#[test]
fn links_03_reduction_distributed() {
    // TASK-0022: the stretch schedule lifts cleanly through link —
    // every algorithm kernel placed, every cross-worker data symbol
    // (a, partials) has a `transfer` directive, no unknown loops.
    // Emit currently rejects distributed placement (TASK-0117 +
    // TASK-0126); the e2e test stays `#[ignore]`'d. This link test
    // pins that the upstream pipeline is wired correctly.
    link_example(
        "03-reduction/prog.algo.nuc",
        "03-reduction/schedules/distributed.sched.nuc",
    );
}

#[test]
fn links_05_stencil_naive() {
    // TASK-0031: 3x3 stencil naive schedule — single worker, every
    // kernel on host. No transfers, no loop directives.
    link_example(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/naive.sched.nuc",
    );
}

#[test]
fn links_05_stencil_blocked() {
    // TASK-0031: blocked schedule — same placement as naive plus a
    // `loop y : block=4;` directive. Link should succeed; the
    // block-transform pass (TASK-0030) and emit run later in the
    // pipeline (and that's where the divisibility check bites for
    // the blocked cell — see e2e_example_05.rs::blocked_*).
    link_example(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/blocked.sched.nuc",
    );
}

#[test]
fn links_05_stencil_distributed() {
    // TASK-0031: distributed schedule — four compute workers, two
    // loop directives, two transfer directives. Link must pin that
    // the transfers cover the cross-worker dataflow (`img_in`
    // host -> {w0..w3}; `img_out` {w0..w3} -> host); the e2e gate is
    // blocked on TASK-0117 + halo synthesis follow-ups.
    link_example(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/distributed.sched.nuc",
    );
}

#[test]
fn links_07_matmul_naive() {
    // TASK-0032: matmul naive schedule — single worker, four
    // kernels (madd + load_a + load_b + save_c) all on host. No
    // transfers, no loop directives.
    link_example(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/naive.sched.nuc",
    );
}

#[test]
fn links_07_matmul_blocked() {
    // TASK-0032: blocked schedule — same placement as naive plus
    // two `block=8` directives on `i` and `j`. Link should succeed;
    // the block-transform pass and emit run later in the pipeline.
    // N=16, block=8 divides cleanly so the divisibility check
    // passes (unlike example 05's deliberately-mismatched block=4
    // on range 14). E2e gate is TASK-0143 (per-tile transfer
    // hoisting) — see e2e_example_07.rs::blocked_*.
    link_example(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/blocked.sched.nuc",
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
    // TASK-0096: `bar` vs the sole declared kernel `foo` is distance 3
    // (b→f, a→o, r→o), bound = max(1, 3/3) = 1 → NO suggestion. This
    // pins the "don't suggest nonsense for an unrelated name" half.
    assert!(
        errs.contains(&LinkError::UnknownKernel {
            name: "bar".into(),
            suggestion: None,
        }),
        "want UnknownKernel{{bar, None}} in {errs:?}"
    );
    assert!(
        errs.contains(&LinkError::UnplacedKernel("foo".into())),
        "want UnplacedKernel(foo) in {errs:?}"
    );
}

#[test]
fn negative_unknown_kernel_with_suggestion() {
    // TASK-0096: a typo-class unknown kernel name → Some(closest).
    // `fooo` vs declared `foo` is distance 1 (one insertion); bound
    // = max(1, 4/3) = 1 → suggested. `barbaz` is unrelated (distance
    // far above bound) → no suggestion for it.
    let algo = algo_from_str(
        "\
kernel foo : () -> () effectful;
kernel barbaz : () -> () effectful;
foo();
barbaz();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place fooo on host;
    place barbaz on host;
    place foo on host;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert!(
        errs.contains(&LinkError::UnknownKernel {
            name: "fooo".into(),
            suggestion: Some("foo".into()),
        }),
        "want UnknownKernel{{fooo, Some(foo)}} in {errs:?}"
    );
    // Display surfaces the hint.
    let rendered = errs
        .iter()
        .map(|e| e.to_string())
        .find(|s| s.contains("fooo"))
        .expect("an error mentioning fooo");
    assert!(
        rendered.contains("did you mean `foo`?"),
        "Display must carry the hint, got: {rendered}"
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
    // No data declared at all → no candidate → suggestion None.
    // assert_eq! on the whole Vec preserves the original strength
    // (exactly one error, exact variant+payload) and adds the
    // suggestion field to the asserted value (AC#3).
    assert_eq!(
        errs,
        vec![LinkError::UnknownData {
            name: "ghost".into(),
            suggestion: None,
        }]
    );
}

#[test]
fn negative_unknown_data_with_suggestion() {
    // TASK-0096: typo'd `place_data` name → Some(closest data).
    let algo = algo_from_str(
        "\
const N : usize = 4;
data weights : f32[N];
kernel k : () -> f32[N] pure;
weights <-- k();
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    memory_region r { size = 4KB; };
    workers = { host };
    place k on host;
    place_data weight in r;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    // `weight` vs `weights`: distance 1; bound = max(1, 6/3) = 2 → Some.
    assert_eq!(
        errs,
        vec![LinkError::UnknownData {
            name: "weight".into(),
            suggestion: Some("weights".into()),
        }]
    );
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
    // No `for` loop in the algorithm → no loop-var candidates → None.
    assert_eq!(
        errs,
        vec![LinkError::UnknownLoop {
            name: "y".into(),
            suggestion: None,
        }]
    );
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
    assert_eq!(
        errs,
        vec![LinkError::UnknownLoop {
            name: "n".into(),
            suggestion: None,
        }]
    );
}

#[test]
fn negative_unknown_loop_with_suggestion() {
    // TASK-0096: algorithm has a `for i : ...`; schedule names `j`.
    // distance(j, i) = 1; bound = max(1, 1/3) = 1 → Some(i).
    let algo = algo_from_str(
        "\
const N : usize = 8;
data a : f32[N];
kernel k : () -> f32 pure;
for i : 0 .. N {
    a[i] <-- k();
}
",
    );
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    loop j : block=4;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    assert_eq!(
        errs,
        vec![LinkError::UnknownLoop {
            name: "j".into(),
            suggestion: Some("i".into()),
        }]
    );
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
    // No data declared → no candidate → None.
    assert_eq!(
        errs,
        vec![LinkError::UnknownTransferData {
            name: "phantom".into(),
            suggestion: None,
        }]
    );
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
    // No data / no loop vars declared → both suggestions None.
    assert!(errs.contains(&LinkError::UnknownTransferData {
        name: "phantom".into(),
        suggestion: None,
    }));
    assert!(errs.contains(&LinkError::UnknownLoop {
        name: "z".into(),
        suggestion: None,
    }));
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

// --------------------------------------------------------------------
// TASK-0134: pipeline-depth vs buffer-capacity constraint
// --------------------------------------------------------------------

/// Common algorithm fixture for the TASK-0134 link tests. Two stages
/// inside one loop, so a cross-worker transfer's Push/Wait pair stays
/// inside the loop (both producer kernel and consumer kernel inside).
const TWO_STAGE_PIPELINE_ALGO: &str = "\
const N : usize = 16;
data input  : i32[N];
data stage1 : i32[N];
data stage2 : i32[N];
kernel load_input : () -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel f1 : (i32) -> i32 pure;
kernel f2 : (i32) -> i32 pure;

input <-- load_input();
for n : 0 .. N {
    stage1[n] <-- f1(input[n]);
    stage2[n] <-- f2(stage1[n]);
}
save_output(stage2);
";

#[test]
fn negative_pipeline_depth_exceeds_buffer() {
    // pipeline=4 with buffer=3 on the inter-stage transfer (both
    // producer and consumer inside the loop) -> hard error.
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=4;
    transfer input  : async, buffer=4, notify=event;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("D=4 > buffer=3 must fail");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            LinkError::PipelineExceedsBuffer {
                loop_var,
                data,
                depth: 4,
                buffer: 3
            } if loop_var == "n" && data == "stage1"
        )),
        "expected PipelineExceedsBuffer for (n, stage1, 4, 3); got {:?}",
        errs
    );
}

#[test]
fn positive_pipeline_depth_equals_buffer() {
    // AC#3 positive: D=N is allowed (exactly fills the buffer).
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=3;
    transfer input  : async, buffer=3, notify=event;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    link(algo, sched).expect("D=3 == buffer=3 must link");
}

#[test]
fn positive_pipeline_depth_less_than_buffer() {
    // AC#3 positive: D=N-1 is allowed.
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=2;
    transfer input  : async, buffer=3, notify=event;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    link(algo, sched).expect("D=2 < buffer=3 must link");
}

#[test]
fn pipeline_does_not_check_loop_invariant_transfer() {
    // For example 13's `input` symbol: load_input is OUTSIDE the
    // pipelined loop, the consumer is INSIDE. The Push/Wait pair
    // gets hoisted out by transfer_inject, so the IR-level pipeline
    // depth annotation does NOT apply — the link-step check must
    // also skip this case. Otherwise we'd reject `input` carrying
    // `buffer=1` (default) under `pipeline=3`.
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=3;
    // `input` has the default buffer=1; producer (load_input) is
    // outside the loop, so this must NOT trigger PipelineExceedsBuffer.
    transfer input  : sync;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    link(algo, sched).expect(
        "loop-invariant transfer (producer outside the loop) is not policed by the pipeline-depth constraint",
    );
}

#[test]
fn pipeline_check_uses_default_buffer_when_unspecified() {
    // No buffer=N on the transfer -> default buffer=1. pipeline=3 on
    // both-endpoints-inside fires the constraint with buffer=1.
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=3;
    transfer input  : async, buffer=3, notify=event;
    // No buffer= on stage1: default is 1. D=3 > 1 -> hard error.
    transfer stage1 : sync;
    transfer stage2 : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("default buffer=1 vs pipeline=3 must fail");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            LinkError::PipelineExceedsBuffer {
                loop_var,
                data,
                depth: 3,
                buffer: 1
            } if loop_var == "n" && data == "stage1"
        )),
        "expected PipelineExceedsBuffer for (n, stage1, 3, 1); got {:?}",
        errs
    );
}

#[test]
fn pipeline_exceeds_buffer_coexists_with_other_link_errors() {
    // Cascade-safety: the new PipelineExceedsBuffer error rides the
    // independent-errors path in link.rs (just `errors.push`, no
    // cascade-suppression keyed on a failed-decl set). Confirm it
    // surfaces alongside an unrelated LinkError variant in one pass.
    //
    // Setup: TWO defects — (a) pipeline=4 vs stage1's buffer=3
    // (PipelineExceedsBuffer); (b) `unknown_loop` named in `loop`
    // does not appear in the algorithm (UnknownLoop). Both must
    // appear in the same link() result.
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=4;
    loop unknown_loop : block=2;
    transfer input  : async, buffer=4, notify=event;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("two defects must fail");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            LinkError::PipelineExceedsBuffer {
                loop_var, data, depth: 4, buffer: 3
            } if loop_var == "n" && data == "stage1"
        )),
        "expected PipelineExceedsBuffer; got {:?}",
        errs
    );
    assert!(
        errs.iter().any(|e| matches!(
            e,
            LinkError::UnknownLoop { name, .. } if name == "unknown_loop"
        )),
        "expected UnknownLoop(unknown_loop) reported in the same pass; got {:?}",
        errs
    );
    assert!(
        errs.len() >= 2,
        "expected >=2 distinct errors in one pass, got {} ({:?})",
        errs.len(),
        errs
    );
}

#[test]
fn pipeline_check_message_names_offending_quartet() {
    // The error message must name the loop, data, depth, and buffer.
    let algo = algo_from_str(TWO_STAGE_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=5;
    transfer input  : async, buffer=5, notify=event;
    transfer stage1 : async, buffer=2, notify=event;
    transfer stage2 : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    let msg = errs
        .iter()
        .find_map(|e| match e {
            err @ LinkError::PipelineExceedsBuffer { .. } => Some(format!("{err}")),
            _ => None,
        })
        .expect("PipelineExceedsBuffer present");
    assert!(msg.contains("`n`"), "names loop_var: {msg}");
    assert!(msg.contains("stage1"), "names data: {msg}");
    assert!(msg.contains("pipeline=5"), "names depth: {msg}");
    assert!(msg.contains("buffer=2"), "names buffer: {msg}");
}

// --------------------------------------------------------------------
// TASK-0217: pipeline=D > iteration_count
// --------------------------------------------------------------------

/// Algorithm fixture with an EXPLICIT-small loop range (N=2) so we can
/// trigger `pipeline=D > iter_count` with D=3 — the IR-level oddity
/// TASK-0217 closes.
const SMALL_LOOP_PIPELINE_ALGO: &str = "\
const N : usize = 2;
data input  : i32[N];
data stage1 : i32[N];
data stage2 : i32[N];
kernel load_input : () -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel f1 : (i32) -> i32 pure;
kernel f2 : (i32) -> i32 pure;

input <-- load_input();
for n : 0 .. N {
    stage1[n] <-- f1(input[n]);
    stage2[n] <-- f2(stage1[n]);
}
save_output(stage2);
";

#[test]
fn negative_pipeline_depth_exceeds_iteration_count() {
    // pipeline=3 on a 2-iteration loop — D > iter_count.
    // TASK-0217 says reject at link time so the diagnostic points at the
    // user-visible loop_var instead of surfacing as analysis-net
    // leftover initial-marking tokens at acfg_to_petri.
    let algo = algo_from_str(SMALL_LOOP_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=3;
    transfer input  : async, buffer=3, notify=event;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("D=3 > iter_count=2 must fail");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            LinkError::PipelineExceedsIterationCount {
                loop_var,
                depth: 3,
                iteration_count: 2,
            } if loop_var == "n"
        )),
        "expected PipelineExceedsIterationCount for (n, 3, 2); got {:?}",
        errs
    );
}

#[test]
fn pipeline_iter_count_check_message_names_loop_and_numbers() {
    let algo = algo_from_str(SMALL_LOOP_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=3;
    transfer input  : async, buffer=3, notify=event;
    transfer stage1 : async, buffer=3, notify=event;
    transfer stage2 : sync;
}
",
    );
    let errs = link(algo, sched).expect_err("must fail");
    let msg = errs
        .iter()
        .find_map(|e| match e {
            err @ LinkError::PipelineExceedsIterationCount { .. } => Some(format!("{err}")),
            _ => None,
        })
        .expect("PipelineExceedsIterationCount present");
    assert!(msg.contains("`n`"), "names loop_var: {msg}");
    assert!(msg.contains("pipeline=3"), "names depth: {msg}");
    assert!(msg.contains("2 iteration"), "names iter_count: {msg}");
}

#[test]
fn positive_pipeline_depth_equals_iteration_count() {
    // pipeline=2 on a 2-iteration loop = OK boundary. D == iter_count
    // is the tightest legal pipeline (the head-start exactly fills the
    // loop). With buffer=2, this should link cleanly.
    let algo = algo_from_str(SMALL_LOOP_PIPELINE_ALGO);
    let sched = sched_from_str(
        "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place f1 on w0;
    place f2 on w1;
    loop n : pipeline=2;
    transfer input  : async, buffer=2, notify=event;
    transfer stage1 : async, buffer=2, notify=event;
    transfer stage2 : sync;
}
",
    );
    link(algo, sched).expect("D=iter_count=2, buffer=2: must link cleanly");
}
