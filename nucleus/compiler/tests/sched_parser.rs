//! Integration tests for the schedule-sublanguage parser.
//!
//! Test strategy (hand-rolled assertions, no `insta`):
//! - For each existing example `*.sched.nuc`, assert structural counts
//!   (directives by kind) and a few spot-checks on payload shape.
//!   Snapshotting the full AST would be brittle as we evolve the AST.
//! - Negative tests: hand-written invalid sources must return an `Err`
//!   with the expected `ParseErrorKind`.
//! - Time literal: separate positive test exercises `ns`/`us`/`ms`/`s`
//!   and the chosen normalisation to nanoseconds.
//!
//! Known-failing example: `14-hearing-aid/schedules/embedded_multimcu.sched.nuc`
//! writes `check frame : ...;` without the grammar-mandated `loop`
//! keyword. The parser MUST reject it. TASK-0079 owns the
//! reconciliation; once TASK-0079 lands (either grammar relaxation or
//! example fix), the assertion here flips from `Err` to `Ok`.

use compiler::sched::{
    parse_sched, CheckAssert, Directive, LoopOption, PlaceTarget, SimdSpec, TimeUnit,
    TransferOption,
};

/// Reads a source file at a workspace-relative path. Panics on IO
/// failure — these tests are environment-dependent by design, and
/// silent skips would hide regressions.
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

// --------------------------------------------------------------------
// Positive: existing example schedule files
// --------------------------------------------------------------------

#[test]
fn parses_01_elementwise_add_naive() {
    // TASK-0013: smallest schedule. One worker (host), four
    // `place` directives, no loops, no transfers.
    let src = read_example("01-elementwise-add/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("01-elementwise-add/naive must parse");
    assert_eq!(ast.algo_path, "../prog.algo.nuc");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 4, "four place directives");
    assert_eq!(ast.count_loops(), 0);
    assert_eq!(ast.count_transfers(), 0);
    assert_eq!(ast.count_checks(), 0);
}

#[test]
fn parses_05_stencil_naive() {
    let src = read_example("05-stencil/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("05-stencil/naive must parse");
    assert_eq!(ast.algo_path, "../prog.algo.nuc");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 3, "three place directives");
    assert_eq!(ast.count_loops(), 0);
    assert_eq!(ast.count_transfers(), 0);
    assert_eq!(ast.count_checks(), 0);
}

#[test]
fn parses_05_stencil_distributed() {
    let src = read_example("05-stencil/schedules/distributed.sched.nuc");
    let ast = parse_sched(&src).expect("05-stencil/distributed must parse");
    assert_eq!(ast.count_workers(), 1);
    assert_eq!(ast.count_places(), 3);
    assert_eq!(ast.count_loops(), 2);
    assert_eq!(ast.count_transfers(), 2);

    // Spot-check: the distributed place is on a 4-worker set.
    let blur3 = ast
        .directives
        .iter()
        .find_map(|d| match d {
            Directive::Place(p) if p.kernel == "blur3" => Some(p),
            _ => None,
        })
        .expect("blur3 place");
    match &blur3.target {
        PlaceTarget::Many(v) => assert_eq!(v.len(), 4, "blur3 distributed over 4 workers"),
        other => panic!("expected Many, got {:?}", other),
    }

    // Spot-check: loop x has three options.
    let loop_x = ast
        .directives
        .iter()
        .find_map(|d| match d {
            Directive::Loop(l) if l.var == "x" => Some(l),
            _ => None,
        })
        .expect("loop x");
    assert_eq!(loop_x.options.len(), 3);
    assert!(loop_x.options.contains(&LoopOption::Block(64)));
    assert!(loop_x.options.contains(&LoopOption::Vectorize(8)));
    assert!(loop_x.options.contains(&LoopOption::Reuse));
}

#[test]
fn parses_13_cnn_naive() {
    let src = read_example("13-cnn-inference/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("13-cnn/naive must parse");
    assert_eq!(ast.count_places(), 5);
    assert_eq!(ast.count_transfers(), 0);
}

#[test]
fn parses_13_cnn_batch_parallel() {
    let src = read_example("13-cnn-inference/schedules/batch_parallel.sched.nuc");
    let ast = parse_sched(&src).expect("13-cnn/batch_parallel must parse");
    assert_eq!(ast.count_places(), 5);
    assert_eq!(ast.count_loops(), 1);
    assert_eq!(ast.count_transfers(), 2);
}

#[test]
fn parses_13_cnn_pipeline_parallel() {
    let src = read_example("13-cnn-inference/schedules/pipeline_parallel.sched.nuc");
    let ast = parse_sched(&src).expect("13-cnn/pipeline_parallel must parse");
    assert_eq!(ast.count_places(), 5);
    assert_eq!(ast.count_loops(), 1);
    assert_eq!(ast.count_transfers(), 4);

    // Spot-check the pipeline=3 option.
    let loop_n = ast
        .directives
        .iter()
        .find_map(|d| match d {
            Directive::Loop(l) if l.var == "n" => Some(l),
            _ => None,
        })
        .expect("loop n");
    assert_eq!(loop_n.options, vec![LoopOption::Pipeline(3)]);

    // Spot-check `output` is sync.
    let output_xfer = ast
        .directives
        .iter()
        .find_map(|d| match d {
            Directive::Transfer(t) if t.data == "output" => Some(t),
            _ => None,
        })
        .expect("transfer output");
    assert_eq!(output_xfer.options, vec![TransferOption::Sync]);
}

#[test]
fn parses_14_hearing_aid_naive() {
    let src = read_example("14-hearing-aid/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("14-hearing-aid/naive must parse");
    assert_eq!(ast.count_places(), 6);
    assert_eq!(ast.count_transfers(), 0);
}

/// `14-hearing-aid/schedules/embedded_multimcu.sched.nuc` writes
/// `check frame : ...;` (without the grammar-mandated `loop` keyword).
/// The grammar's PRD-faithful form is `check loop VAR : ...;`. Until
/// TASK-0079 reconciles the example with the grammar, the parser
/// rejects this file. When TASK-0079 lands, flip this assertion.
#[test]
fn known_failing_14_hearing_aid_embedded_multimcu_pending_task_0079() {
    let src = read_example("14-hearing-aid/schedules/embedded_multimcu.sched.nuc");
    let err = parse_sched(&src).expect_err(
        "embedded_multimcu uses `check frame : ...` without the `loop` keyword \
         the grammar requires. TASK-0079 owns the reconciliation; flip this test \
         once that lands.",
    );
    // The bad token is `check frame ...` at line 105 in the example.
    // We don't pin the exact column because chumsky may report any
    // alternative; just confirm a position is set.
    assert!(err.line >= 1, "{:?}", err);
}

// --------------------------------------------------------------------
// Negative tests (>= 4)
// --------------------------------------------------------------------

#[test]
fn negative_for_loop_in_schedule_is_rejected() {
    // Control flow belongs in the algorithm. `for` is not a valid
    // SchedItem keyword.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    for y : 0..10 {
        loop y : block=64;
    }
}
";
    let err = parse_sched(src).expect_err("for-loop must be rejected");
    // The unexpected token is on line 3, `for`.
    assert_eq!(err.line, 3, "{:?}", err);
}

#[test]
fn negative_empty_worker_set_in_place_is_rejected() {
    // PlaceTarget := Ident | '{' IdentList '}'; IdentList is
    // non-empty in our reading. `place X on { }` must fail.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place blur3 on { };
}
";
    let err = parse_sched(src).expect_err("empty worker set must be rejected");
    // The empty `{ }` is on line 3.
    assert_eq!(err.line, 3, "{:?}", err);
}

#[test]
fn negative_loop_with_no_options_is_rejected() {
    // Grammar `LoopStmt ::= 'loop' Ident ':' LoopOptList ';'`. The
    // option list is non-empty.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : ;
}
";
    let err = parse_sched(src).expect_err("loop without options must be rejected");
    assert_eq!(err.line, 3, "{:?}", err);
}

#[test]
fn negative_wrong_time_unit_suffix_is_rejected() {
    // `10minutes` is not a legal time literal. The grammar offers
    // only ns/us/ms/s.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on host;
    check loop frame : latency_max = 10minutes;
}
";
    let err = parse_sched(src).expect_err("bad time-unit suffix must be rejected");
    // `10minutes` is on line 4.
    assert_eq!(err.line, 4, "{:?}", err);
}

#[test]
fn negative_missing_semicolon_after_workers() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host }
}
";
    let err = parse_sched(src).expect_err("missing `;` must be rejected");
    assert!(err.line >= 2, "{:?}", err);
}

// --------------------------------------------------------------------
// Time-literal handling
// --------------------------------------------------------------------

/// Time literals normalise to nanoseconds; the original unit is
/// retained for diagnostics. See `sched/ast.rs`.
#[test]
fn time_literals_normalise_to_nanoseconds() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on host;
    check loop a : latency_max = 10ms;
    check loop b : latency_max = 500us;
    check loop c : latency_max = 2s;
    check loop d : latency_max = 100ns;
}
";
    let ast = parse_sched(src).expect("must parse");
    let checks: Vec<_> = ast
        .directives
        .iter()
        .filter_map(|d| match d {
            Directive::Check(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(checks.len(), 4);

    let lat = |var: &str| -> (u64, TimeUnit) {
        let c = checks
            .iter()
            .find(|c| c.var == var)
            .unwrap_or_else(|| panic!("missing check for {}", var));
        match c.asserts[0] {
            CheckAssert::LatencyMax(t) => (t.nanos, t.original_unit),
            ref other => panic!("expected LatencyMax, got {:?}", other),
        }
    };

    assert_eq!(lat("a"), (10 * 1_000_000, TimeUnit::Ms));
    assert_eq!(lat("b"), (500 * 1_000, TimeUnit::Us));
    assert_eq!(lat("c"), (2 * 1_000_000_000, TimeUnit::S));
    assert_eq!(lat("d"), (100, TimeUnit::Ns));
}

// --------------------------------------------------------------------
// Typed worker form & memory regions (sanity)
// --------------------------------------------------------------------

#[test]
fn typed_workers_and_memory_regions_parse() {
    // Stripped-down variant of embedded_multimcu, with the `check`
    // form the GRAMMAR requires (`check loop frame : ...`). Validates
    // that the typed worker form, memory regions, and place_data all
    // parse, independent of the TASK-0079 example divergence.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class fe_core { simd = none; memory = shared; };
    worker_class dsp_core { simd = neon128; memory = tightly_coupled[64KB] + shared; };

    memory_region sram_shared {
        size = 128KB;
        accessible_by = { fe_core, dsp_core };
    };
    memory_region dsp_tcm {
        size = 64KB;
        accessible_by = { dsp_core };
        per_worker = true;
    };

    workers = {
        fe  : fe_core,
        dsp : dsp_core,
    };

    place_data mic_in in sram_shared;

    place fe_capture on fe;
    place denoise    on dsp;

    loop frame : pipeline=3;
    transfer mic_in : async, buffer=2, notify=event;

    check loop frame : latency_max = 10ms, on_violation = panic;
}
";
    let ast = parse_sched(src).expect("typed-form schedule must parse");
    assert_eq!(ast.count_worker_classes(), 2);
    assert_eq!(ast.count_memory_regions(), 2);
    assert_eq!(ast.count_workers(), 1);
    assert_eq!(ast.count_place_data(), 1);
    assert_eq!(ast.count_places(), 2);
    assert_eq!(ast.count_loops(), 1);
    assert_eq!(ast.count_transfers(), 1);
    assert_eq!(ast.count_checks(), 1);

    // Spot-check the worker_class shapes.
    let fe = ast
        .directives
        .iter()
        .find_map(|d| match d {
            Directive::WorkerClass(c) if c.name == "fe_core" => Some(c),
            _ => None,
        })
        .expect("fe_core");
    assert_eq!(fe.simd, Some(SimdSpec::None));

    let dsp = ast
        .directives
        .iter()
        .find_map(|d| match d {
            Directive::WorkerClass(c) if c.name == "dsp_core" => Some(c),
            _ => None,
        })
        .expect("dsp_core");
    assert_eq!(dsp.simd, Some(SimdSpec::Named("neon128".to_string())));
}
