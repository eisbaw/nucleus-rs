//! Integration tests for the schedule AST → SchedIR lowering pass
//! (TASK-0010).
//!
//! Test strategy mirrors `algo_lower.rs`:
//! - Positive: each existing `*.sched.nuc` lowers cleanly (counts and
//!   spot-checks; no whole-IR snapshots — those would freeze the
//!   encoding).
//! - Negative: one hand-written invalid source per [`SchedLowerError`]
//!   variant we care to defend.
//!
//! `14-hearing-aid/schedules/embedded_multimcu.sched.nuc` is excluded
//! from the positive set by scope (far-future M11 multi-MCU schedule,
//! not the M3 lower matrix). It now parses cleanly — TASK-0079
//! reconciled its `check loop` form, pinned by the parser test.
//! Follow-up TASK-0192 tracks adding it to the lower matrix.

use compiler::error::offset_to_line_col;
use compiler::sched::{
    lower_sched, parse_sched, NotifyKind, PartitionKind, ResolvedLoopOption, ResolvedPlaceTarget,
    ResolvedTransferOption, SchedIR, SchedLowerError, SchedLowerErrorKind, SchedLowerErrors,
    ViolationKind, DEFAULT_WORKER_CLASS,
};

/// Reads a source file at a workspace-relative path. Panics on IO
/// failure — these tests are environment-dependent by design.
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

/// Parse + lower in one helper. Panics if parsing fails (the negative
/// inputs in this file must parse — they exercise lowering, not the
/// parser).
///
/// Returns [`SchedLowerErrors`] (the multi-error bundle, TASK-0200) on
/// failure. Negative tests that assert a single first error migrate to
/// `.first().clone()`-style chaining (mirrors the algo-side
/// TASK-0092 migration), preserving the SAME discriminating match —
/// no loss of assertion strength.
fn lower_str(src: &str) -> Result<SchedIR, SchedLowerErrors> {
    let ast = parse_sched(src).expect("source must parse for this lowering test");
    lower_sched(&ast)
}

// --------------------------------------------------------------------
// Positive: existing example schedules lower
// --------------------------------------------------------------------
//
// 14-hearing-aid/embedded_multimcu.sched.nuc is omitted by scope
// (far-future M11 multi-MCU schedule). It parses cleanly post
// TASK-0079; lower-matrix inclusion is follow-up TASK-0192.

#[test]
fn lowers_01_elementwise_add_naive() {
    // TASK-0013: lowers cleanly. Single worker, 4 placements, no
    // loops, no transfers, no checks.
    let src = read_example("01-elementwise-add/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("01-elementwise-add/naive must lower");

    assert_eq!(ir.workers.len(), 1);
    assert_eq!(ir.workers["host"].class, DEFAULT_WORKER_CLASS);
    assert_eq!(ir.places.len(), 4);
    assert!(ir.transfers.is_empty());
    assert!(ir.loops.is_empty());
    assert!(ir.checks.is_empty());
}

#[test]
fn lowers_02_split_add_naive() {
    // TASK-0021: smoke variant.
    let src = read_example("02-split-add/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("02-split-add/naive must lower");

    assert_eq!(ir.workers.len(), 1);
    assert_eq!(ir.workers["host"].class, DEFAULT_WORKER_CLASS);
    assert_eq!(ir.places.len(), 4);
    assert!(ir.transfers.is_empty());
    assert!(ir.loops.is_empty());
    assert!(ir.checks.is_empty());
}

#[test]
fn lowers_02_split_add_split() {
    // TASK-0021: two-worker schedule with three sync transfers.
    let src = read_example("02-split-add/schedules/split.sched.nuc");
    let ir = lower_str(&src).expect("02-split-add/split must lower");

    assert_eq!(ir.workers.len(), 2);
    assert_eq!(ir.workers["host"].class, DEFAULT_WORKER_CLASS);
    assert_eq!(ir.workers["w0"].class, DEFAULT_WORKER_CLASS);

    assert_eq!(ir.places.len(), 4);
    // Spot-check placements after lowering.
    match &ir.places["add"].target {
        ResolvedPlaceTarget::One(w) => assert_eq!(w, "w0"),
        other => panic!("expected single-worker target for add, got {:?}", other),
    }
    match &ir.places["load_input"].target {
        ResolvedPlaceTarget::One(w) => assert_eq!(w, "host"),
        other => panic!(
            "expected single-worker target for load_input, got {:?}",
            other
        ),
    }

    assert_eq!(ir.transfers.len(), 3);
    for name in ["a", "b", "c"] {
        let t = ir
            .transfers
            .get(name)
            .unwrap_or_else(|| panic!("missing transfer {}", name));
        assert_eq!(
            t.options,
            vec![ResolvedTransferOption::Sync],
            "transfer {} should be sync-only",
            name
        );
    }

    assert!(ir.loops.is_empty());
    assert!(ir.checks.is_empty());
}

#[test]
fn lowers_03_reduction_naive() {
    // TASK-0022: smoke-test schedule.
    let src = read_example("03-reduction/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("03-reduction/naive must lower");

    assert_eq!(ir.workers.len(), 1);
    assert_eq!(ir.workers["host"].class, DEFAULT_WORKER_CLASS);
    assert_eq!(ir.places.len(), 4);
    assert!(ir.transfers.is_empty());
    assert!(ir.loops.is_empty());
    assert!(ir.checks.is_empty());
}

#[test]
fn lowers_03_reduction_distributed() {
    // TASK-0022: stretch schedule lowers cleanly even though the
    // backend's emit currently rejects distributed placement.
    let src = read_example("03-reduction/schedules/distributed.sched.nuc");
    let ir = lower_str(&src).expect("03-reduction/distributed must lower");

    assert_eq!(ir.workers.len(), 5);
    assert_eq!(ir.places.len(), 4);
    assert_eq!(ir.loops.len(), 1);
    assert_eq!(ir.transfers.len(), 2);

    // The distributed place targets four compute workers.
    match &ir.places["accumulate"].target {
        ResolvedPlaceTarget::Many(v) => assert_eq!(v.len(), 4),
        other => panic!("expected Many target for accumulate, got {:?}", other),
    }

    // loop w : partition=workers
    let loop_w = &ir.loops["w"];
    assert_eq!(
        loop_w.options,
        vec![ResolvedLoopOption::Partition(PartitionKind::Workers)]
    );

    // Both transfers sync-only.
    for name in ["a", "partials"] {
        let t = ir
            .transfers
            .get(name)
            .unwrap_or_else(|| panic!("missing transfer {}", name));
        assert_eq!(
            t.options,
            vec![ResolvedTransferOption::Sync],
            "transfer {} should be sync-only",
            name
        );
    }
}

#[test]
fn lowers_05_stencil_naive() {
    let src = read_example("05-stencil/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("05-stencil/naive must lower");

    // Workers = { host }. Simple form -> default class injected.
    assert_eq!(ir.workers.len(), 1);
    assert_eq!(ir.workers["host"].class, DEFAULT_WORKER_CLASS);
    assert_eq!(ir.worker_classes.len(), 1);
    assert!(ir.worker_classes[DEFAULT_WORKER_CLASS].is_default);

    // 3 places, no transfers/loops/checks.
    assert_eq!(ir.places.len(), 3);
    assert!(ir.transfers.is_empty());
    assert!(ir.loops.is_empty());
    assert!(ir.checks.is_empty());
}

#[test]
fn lowers_05_stencil_distributed() {
    let src = read_example("05-stencil/schedules/distributed.sched.nuc");
    let ir = lower_str(&src).expect("05-stencil/distributed must lower");

    assert_eq!(ir.workers.len(), 5);
    assert_eq!(ir.places.len(), 3);
    assert_eq!(ir.loops.len(), 2);
    assert_eq!(ir.transfers.len(), 2);

    // The distributed place targets four compute workers.
    match &ir.places["blur3"].target {
        ResolvedPlaceTarget::Many(v) => assert_eq!(v.len(), 4),
        other => panic!("expected Many target for blur3, got {:?}", other),
    }

    // loop x : block=64, vectorize=8, reuse
    let loop_x = &ir.loops["x"];
    assert!(loop_x.options.contains(&ResolvedLoopOption::Block(64)));
    assert!(loop_x.options.contains(&ResolvedLoopOption::Vectorize(8)));
    assert!(loop_x.options.contains(&ResolvedLoopOption::Reuse));

    // loop y : partition=rows
    let loop_y = &ir.loops["y"];
    assert_eq!(
        loop_y.options,
        vec![ResolvedLoopOption::Partition(PartitionKind::Rows)]
    );

    // transfer img_in : async, buffer=2, notify=event
    let img_in = &ir.transfers["img_in"];
    assert!(img_in.options.contains(&ResolvedTransferOption::Async));
    assert!(img_in.options.contains(&ResolvedTransferOption::Buffer(2)));
    assert!(img_in
        .options
        .contains(&ResolvedTransferOption::Notify(NotifyKind::Event)));
}

#[test]
fn lowers_07_matmul_naive() {
    // TASK-0032: smoke-test schedule. Single host worker, four
    // placements, no loops/transfers/checks.
    let src = read_example("07-matmul/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("07-matmul/naive must lower");

    assert_eq!(ir.workers.len(), 1);
    assert_eq!(ir.workers["host"].class, DEFAULT_WORKER_CLASS);
    assert_eq!(ir.places.len(), 4);
    assert!(ir.transfers.is_empty());
    assert!(ir.loops.is_empty());
    assert!(ir.checks.is_empty());
}

#[test]
fn lowers_07_matmul_blocked() {
    // TASK-0032: 2D blocking — `loop i : block=8; loop j : block=8`.
    // Two loop directives, no transfers, single worker.
    let src = read_example("07-matmul/schedules/blocked.sched.nuc");
    let ir = lower_str(&src).expect("07-matmul/blocked must lower");

    assert_eq!(ir.workers.len(), 1);
    assert_eq!(ir.places.len(), 4);
    assert_eq!(ir.loops.len(), 2);
    assert!(ir.transfers.is_empty());

    for var in ["i", "j"] {
        let l = &ir.loops[var];
        assert_eq!(
            l.options,
            vec![ResolvedLoopOption::Block(8)],
            "loop `{}` must carry exactly block=8",
            var
        );
    }
}

#[test]
fn lowers_13_cnn_naive() {
    let src = read_example("13-cnn-inference/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("13-cnn/naive must lower");
    assert_eq!(ir.places.len(), 5);
    assert!(ir.transfers.is_empty());
}

#[test]
fn lowers_13_cnn_batch_parallel() {
    let src = read_example("13-cnn-inference/schedules/batch_parallel.sched.nuc");
    let ir = lower_str(&src).expect("13-cnn/batch_parallel must lower");
    assert_eq!(ir.places.len(), 5);
    assert_eq!(ir.loops.len(), 1);
    assert_eq!(ir.transfers.len(), 2);
}

#[test]
fn lowers_13_cnn_pipeline_parallel() {
    let src = read_example("13-cnn-inference/schedules/pipeline_parallel.sched.nuc");
    let ir = lower_str(&src).expect("13-cnn/pipeline_parallel must lower");
    assert_eq!(ir.places.len(), 5);
    assert_eq!(ir.loops.len(), 1);
    assert_eq!(ir.transfers.len(), 4);

    // loop n : pipeline=3
    let loop_n = &ir.loops["n"];
    assert_eq!(loop_n.options, vec![ResolvedLoopOption::Pipeline(3)]);

    // transfer output : sync
    let output = &ir.transfers["output"];
    assert_eq!(output.options, vec![ResolvedTransferOption::Sync]);
}

#[test]
fn lowers_14_hearing_aid_naive() {
    let src = read_example("14-hearing-aid/schedules/naive.sched.nuc");
    let ir = lower_str(&src).expect("14-hearing-aid/naive must lower");
    assert_eq!(ir.places.len(), 6);
    assert!(ir.transfers.is_empty());
}

// --------------------------------------------------------------------
// Positive: typed worker form + memory regions + place_data
// --------------------------------------------------------------------
//
// Inline source (no example file is currently parse-able with the
// typed form for the algorithm-side check directive; this is the
// stripped-down sched-parser test reused here for the lowering side).

#[test]
fn lowers_typed_workers_and_memory_regions() {
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
    let ir = lower_str(src).expect("typed-form schedule must lower");
    // 2 declared classes, no synthetic default (all entries typed).
    assert_eq!(ir.worker_classes.len(), 2);
    assert!(!ir.worker_classes["fe_core"].is_default);
    assert!(!ir.worker_classes["dsp_core"].is_default);

    assert_eq!(ir.workers.len(), 2);
    assert_eq!(ir.workers["fe"].class, "fe_core");
    assert_eq!(ir.workers["dsp"].class, "dsp_core");

    assert_eq!(ir.memory_regions.len(), 2);
    assert_eq!(ir.place_data.len(), 1);
    assert_eq!(ir.place_data["mic_in"].region, "sram_shared");

    assert_eq!(ir.places.len(), 2);
    assert_eq!(ir.loops.len(), 1);
    assert_eq!(ir.transfers.len(), 1);
    assert_eq!(ir.checks.len(), 1);

    let frame_check = &ir.checks["frame"];
    assert_eq!(frame_check.asserts.len(), 2);
    assert!(frame_check.asserts.iter().any(|a| matches!(
        a,
        compiler::sched::ResolvedCheckAssert::OnViolation(ViolationKind::Panic)
    )));
}

// --------------------------------------------------------------------
// Negative tests — one per defended SchedLowerError variant
// --------------------------------------------------------------------

#[test]
fn negative_missing_workers_decl() {
    // A schedule with no `workers = ...` is rejected.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    place k on host;
}
";
    let err = lower_str(src).expect_err("missing workers decl must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::MissingWorkersDecl);
}

#[test]
fn negative_duplicate_workers_decl() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    workers = { w0 };
}
";
    let err = lower_str(src).expect_err("two workers decls must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::DuplicateWorkersDecl);
}

#[test]
fn negative_duplicate_worker_name_in_one_decl() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host, host };
}
";
    let err = lower_str(src).expect_err("duplicate worker name must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::DuplicateWorker("host".into()));
}

#[test]
fn dup_worker_beats_unknown_class_pins_ref_recording_ordering() {
    // TASK-0197: pin the dup-before-ref-recording invariant that the
    // SchedLowerError Err-path first-error ordering depends on
    // (TASK-0196 option (b) proved first-error ordering byte-equivalent
    // ONLY because worker name uniqueness holds, which holds ONLY
    // because `DuplicateWorker` early-returns in the pass-1 AST walk
    // BEFORE any tuple is pushed into `worker_class_refs`
    // (sched/lower.rs: dup guards at ~178/186, push at ~217), so the
    // post-collection `UnknownWorkerClass` validation never runs once a
    // duplicate is present).
    //
    // Multi-fault schedule (all entries typed-form so the worker list
    // parses — typed and simple forms cannot mix in one `{ ... }`):
    // the FIRST worker entry `fe : missing_class` references an
    // undeclared class (would be `UnknownWorkerClass` if
    // ref-recording/validation ran), and a LATER entry duplicates
    // `host`. The documented behaviour is that `DuplicateWorker` fires
    // first — the pass-1 walk early-returns on the duplicate before the
    // post-collection unknown-class validation is ever reached.
    //
    // A regression test (not a debug_assert) is the right pin here:
    // the property is a *control-flow ordering* between two passes, not
    // a state predicate evaluable at a single program point — a
    // meaningful runtime assertion cannot be phrased without
    // re-encoding the very ordering it would guard. A refactor that
    // moves ref-recording before the dup guards (or relocates
    // dup-detection after it) would silently change which error the
    // user sees first on a multi-fault schedule; the determinism gate
    // cannot catch it (Err-path only). This test fails loudly instead.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class core { simd = none; };
    workers = { fe : missing_class, host : core, host : core };
}
";
    let err = lower_str(src).expect_err("multi-fault schedule must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateWorker("host".into()),
        "DuplicateWorker must fire BEFORE UnknownWorkerClass: dup-detection \
         early-returns before ref-recording/validation (the invariant \
         TASK-0196's ordering-equivalence proof depends on)"
    );
    // Strength guard: explicitly assert it is NOT the unknown-class
    // variant, so a refactor reordering the two passes cannot pass by
    // coincidence (a loosened kind match would still bite here).
    assert!(
        !matches!(
            err.kind,
            SchedLowerErrorKind::UnknownWorkerClass { .. }
        ),
        "must NOT surface UnknownWorkerClass first on a dup+unknown-class \
         schedule; got {:?}",
        err.kind
    );
}

#[test]
fn negative_unknown_worker_class_reference() {
    // Typed worker entry names a class that has no `worker_class` decl.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { fe : missing_class };
}
";
    let err = lower_str(src).expect_err("unknown worker_class must fail").first().clone();
    // TASK-0198: the only declared class is the synthetic
    // `__default` (no user `worker_class` decl); `missing_class`
    // (len 13) vs `__default` is far above the bound
    // max(1, 13/3) = 4 → no suggestion. Whole-`.kind` assert_eq!
    // strength preserved; the suggestion field is asserted as part
    // of the expected value (AC#2).
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownWorkerClass {
            worker: "fe".into(),
            class: "missing_class".into(),
            suggestion: None,
        }
    );
}

#[test]
fn negative_duplicate_worker_class_decl() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class core { simd = none; };
    worker_class core { simd = neon128; };
    workers = { fe : core };
}
";
    let err = lower_str(src).expect_err("duplicate worker_class must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::DuplicateWorkerClass("core".into()));
}

#[test]
fn negative_unknown_memory_region_reference() {
    // place_data references a region that has no `memory_region` decl.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place_data foo in nowhere;
}
";
    let err = lower_str(src).expect_err("unknown memory_region must fail").first().clone();
    // TASK-0198: no `memory_region` declared at all → empty
    // candidate set → no suggestion. assert_eq! on the whole `.kind`
    // preserves the exact-variant+payload strength and adds the
    // suggestion field to the asserted value (AC#2).
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownMemoryRegion {
            data: "foo".into(),
            region: "nowhere".into(),
            suggestion: None,
        }
    );
}

#[test]
fn negative_duplicate_memory_region_decl() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    memory_region sram { size = 32KB; };
    memory_region sram { size = 64KB; };
    workers = { host };
}
";
    let err = lower_str(src).expect_err("duplicate memory_region must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::DuplicateMemoryRegion("sram".into()));
}

#[test]
fn negative_duplicate_place_for_same_kernel() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host, w0 };
    place k on host;
    place k on w0;
}
";
    let err = lower_str(src).expect_err("duplicate place must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::DuplicatePlace { kernel: "k".into() });
}

#[test]
fn negative_duplicate_place_data_for_same_data() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    memory_region a { size = 4KB; };
    memory_region b { size = 4KB; };
    workers = { host };
    place_data foo in a;
    place_data foo in b;
}
";
    let err = lower_str(src).expect_err("duplicate place_data must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicatePlaceData { data: "foo".into() }
    );
}

#[test]
fn negative_duplicate_loop_for_same_var() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : block=64;
    loop y : vectorize=8;
}
";
    let err = lower_str(src).expect_err("duplicate loop must fail").first().clone();
    assert_eq!(err.kind, SchedLowerErrorKind::DuplicateLoop { var: "y".into() });
}

#[test]
fn negative_duplicate_transfer_for_same_data() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    transfer img : sync;
    transfer img : async;
}
";
    let err = lower_str(src).expect_err("duplicate transfer must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateTransfer { data: "img".into() }
    );
}

#[test]
fn negative_duplicate_check_for_same_var() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    check loop frame : latency_max = 10ms;
    check loop frame : on_violation = log;
}
";
    let err = lower_str(src).expect_err("duplicate check must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateCheck {
            var: "frame".into()
        }
    );
}

#[test]
fn negative_zero_latency_max_is_rejected() {
    // TASK-0052.01 AC#3 — `latency_max = 0<UNIT>` is semantically
    // degenerate (every iteration violates a zero budget). The
    // typed SchedLowerError names the offending loop var.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    check loop frame : latency_max = 0ms;
}
";
    let err = lower_str(src).expect_err("latency_max=0 must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::ZeroLatencyMax {
            var: "frame".into(),
        }
    );
}

#[test]
fn negative_duplicate_latency_max_within_one_check() {
    // TASK-0052.01 AC#2/AC#3 — two `latency_max` assertions inside
    // one `check loop` directive (which value wins is ambiguous).
    // Typed SchedLowerError, not papered over.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    check loop frame : latency_max = 10ms, latency_max = 5ms;
}
";
    let err = lower_str(src).expect_err("duplicate latency_max must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateCheckAssertion {
            var: "frame".into(),
            kind: "latency_max".into(),
        }
    );
}

#[test]
fn negative_missing_latency_max_in_check() {
    // TASK-0052.02 review-gate finding #1: `check loop V :
    // on_violation=panic;` is grammar-valid (asserts are `at_least(1)`
    // and each is a CheckAssert choice) but semantically empty
    // (on_violation is the action when an assertion fails — there's
    // no measurement to violate). Reject at sched-lower.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    check loop frame : on_violation = panic;
}
";
    let err = lower_str(src).expect_err("on_violation-only check must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::MissingLatencyMax {
            var: "frame".into(),
        }
    );
}

#[test]
fn negative_block_pipeline_combination_on_same_loop_is_rejected() {
    // TASK-0215: `loop V : block=N, pipeline=D` has ambiguous semantics
    // (per-tile vs per-iter pipelining) and is rejected at sched-lower.
    // PRD §6.3.3: "bad combinations rejected at compile time".
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : block=4, pipeline=2;
}
";
    let err = lower_str(src).expect_err("block+pipeline on same loop must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::BlockPipelineConflict {
            var: "n".into(),
        }
    );
}

#[test]
fn negative_unroll_not_divisible_by_block_is_rejected() {
    // TASK-0144 Stage 2: `block=N, unroll=M` with M not dividing N
    // is a compile-time bad combination (PRD §6.3.3). Both values
    // are static integers; the check is purely on option payloads.
    // `block=6, unroll=4`: 6 % 4 == 2 != 0 — refused at sched-lower.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : block=6, unroll=4;
}
";
    let err = lower_str(src)
        .expect_err("unroll not dividing block must fail")
        .first()
        .clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnrollNotDivisibleByBlock {
            var: "n".into(),
            unroll: 4,
            block: 6,
        }
    );
}

#[test]
fn negative_vectorize_not_divisible_by_block_is_rejected() {
    // TASK-0144.01 Stage 3: `block=N, vectorize=M` with M not dividing
    // N is a compile-time bad combination (PRD §6.3.3). Both values
    // are static integers; the check is purely on option payloads.
    // `block=6, vectorize=4`: 6 % 4 == 2 != 0 — refused at sched-lower.
    // The positive divisor case (block=64, vectorize=8) is exercised
    // by the existing 05-stencil/distributed lower test and the
    // `positive_reordered_distinct_loop_options_still_lower` smoke.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : block=6, vectorize=4;
}
";
    let err = lower_str(src)
        .expect_err("vectorize not dividing block must fail")
        .first()
        .clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::VectorizeNotDivisibleByBlock {
            var: "n".into(),
            vectorize: 4,
            block: 6,
        }
    );
}

#[test]
fn negative_check_on_strip_mined_loop_is_rejected() {
    // TASK-0052.02 review-gate finding #3: `loop V : block=N;` +
    // `check loop V : latency_max=T;` would silently drop the check
    // because inject_check_frames skips strip-mined inner Event::Loops
    // by design. Reject at sched-lower with a clear diagnostic
    // pointing at the actionable option (remove block OR remove check).
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : block=4;
    check loop n : latency_max = 10ms;
}
";
    let err = lower_str(src).expect_err("check on strip-mined loop must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::CheckOnStripMinedLoop {
            var: "n".into(),
        }
    );
}

#[test]
fn negative_duplicate_on_violation_within_one_check() {
    // Same DuplicateCheckAssertion variant, on_violation slot.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    check loop frame : latency_max = 10ms, on_violation = panic, on_violation = log;
}
";
    let err = lower_str(src).expect_err("duplicate on_violation must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateCheckAssertion {
            var: "frame".into(),
            kind: "on_violation".into(),
        }
    );
}

#[test]
fn negative_zero_block_loop_option() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : block=0;
}
";
    let err = lower_str(src).expect_err("block=0 must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::ZeroLoopOption {
            var: "y".into(),
            option: "block".into()
        }
    );
}

#[test]
fn negative_zero_pipeline_loop_option() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : pipeline=0;
}
";
    let err = lower_str(src).expect_err("pipeline=0 must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::ZeroLoopOption {
            var: "y".into(),
            option: "pipeline".into()
        }
    );
}

#[test]
fn negative_unit_pipeline_loop_option() {
    // TASK-0134: pipeline=1 is rejected as a no-op pipeline. The
    // schedule author must either specify pipeline=D with D >= 2 or
    // omit the option entirely.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : pipeline=1;
}
";
    let err = lower_str(src).expect_err("pipeline=1 must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnitPipelineOption {
            var: "y".into()
        }
    );
    // The error message names the option and tells the user how to fix.
    let msg = format!("{}", err.kind);
    assert!(msg.contains("pipeline=1"), "msg should name pipeline=1: {msg}");
    assert!(msg.contains("D >= 2") || msg.contains(">= 2"), "msg should suggest D >= 2: {msg}");
}

#[test]
fn positive_pipeline_two_lowers_ok() {
    // The minimum legal pipeline depth is 2 — the smallest value
    // distinguishable from the default sequential mode.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : pipeline=2;
}
";
    let ir = lower_str(src).expect("pipeline=2 must lower");
    assert_eq!(ir.loops.len(), 1);
}

#[test]
fn negative_zero_vectorize_loop_option() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : vectorize=0;
}
";
    let err = lower_str(src).expect_err("vectorize=0 must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::ZeroLoopOption {
            var: "y".into(),
            option: "vectorize".into()
        }
    );
}

#[test]
fn negative_zero_buffer_transfer_option() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    transfer img : async, buffer=0;
}
";
    let err = lower_str(src).expect_err("buffer=0 must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::ZeroBufferOption { data: "img".into() }
    );
}

#[test]
fn negative_place_references_unknown_worker() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on bogus;
}
";
    let err = lower_str(src).expect_err("unknown worker must fail").first().clone();
    // TASK-0198: `bogus` vs the sole declared worker `host` is far
    // above the bound max(1, 5/3) = 1 → no suggestion. Whole-`.kind`
    // assert_eq! strength preserved; suggestion asserted (AC#2).
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownPlaceWorker {
            kernel: "k".into(),
            worker: "bogus".into(),
            suggestion: None,
        }
    );
}

#[test]
fn negative_place_set_references_unknown_worker() {
    // The `Many` placement target gets the same per-name validation.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host, w0 };
    place k on { w0, bogus };
}
";
    let err = lower_str(src).expect_err("unknown worker in set must fail").first().clone();
    // TASK-0198: `bogus` vs declared workers {host, w0} — both far
    // above bound max(1, 5/3) = 1 → no suggestion. Strength
    // preserved; suggestion asserted (AC#2).
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownPlaceWorker {
            kernel: "k".into(),
            worker: "bogus".into(),
            suggestion: None,
        }
    );
}

#[test]
fn negative_user_class_collides_with_default() {
    // A user-declared class named the same as the synthetic default is
    // rejected loudly. Otherwise a simple-form worker would silently
    // pick up the user's class. The lowering surfaces the collision
    // via `DuplicateWorkerClass(DEFAULT_WORKER_CLASS)`.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class __default { simd = none; };
    workers = { host };
}
";
    let err = lower_str(src).expect_err("default-class collision must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateWorkerClass(DEFAULT_WORKER_CLASS.to_string())
    );
}

// --------------------------------------------------------------------
// TASK-0093: duplicate / mutually-exclusive directive options
// --------------------------------------------------------------------

#[test]
fn negative_duplicate_loop_option() {
    // `block=64, block=128` on one loop: value-bearing key twice.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop i : block=64, block=128;
}
";
    let err = lower_str(src).expect_err("duplicate loop option must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateLoopOption {
            var: "i".into(),
            option: "block".into(),
        }
    );
}

#[test]
fn negative_mutually_exclusive_transfer_sync_async() {
    // grammar §2 note 5 / §5.3: sync and async cannot coexist.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    transfer x : sync, async;
}
";
    let err = lower_str(src).expect_err("sync+async must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::ConflictingTransferMode { data: "x".into() }
    );
}

#[test]
fn negative_duplicate_transfer_buffer_option() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    transfer x : async, buffer=1, buffer=2;
}
";
    let err = lower_str(src).expect_err("duplicate buffer option must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicateTransferOption {
            data: "x".into(),
            option: "buffer".into(),
        }
    );
}

#[test]
fn positive_reordered_distinct_loop_options_still_lower() {
    // §2 note 7 / §5.1: order is insignificant; distinct options OK.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop x : reuse, vectorize=8, block=64;
}
";
    let ir = lower_str(src).expect("distinct reordered options must lower");
    assert_eq!(ir.loops.get("x").unwrap().options.len(), 3);
}

#[test]
fn positive_repeated_reuse_flag_is_not_a_conflict() {
    // `reuse` is a bare idempotent flag — repetition is harmless
    // redundancy, not the value conflict note 7 targets.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop x : reuse, reuse;
}
";
    lower_str(src).expect("repeated bare `reuse` must lower");
}

// --------------------------------------------------------------------
// TASK-0094: duplicate worker in a placement set
// --------------------------------------------------------------------

#[test]
fn negative_duplicate_place_worker() {
    // `place k on { w0, w0 }` — hard error, not a silent fold.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { w0, w1 };
    place k on { w0, w0 };
}
";
    let err = lower_str(src).expect_err("duplicate place worker must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicatePlaceWorker {
            kernel: "k".into(),
            worker: "w0".into(),
        }
    );
}

#[test]
fn negative_duplicate_place_worker_beats_undeclared() {
    // TASK-0193: pin the documented dup-before-undeclared ordering.
    // `place k on { ghost, ghost }` where `ghost` is NOT declared in
    // `workers`. Both faults are present: the worker is repeated AND
    // undeclared. The lowering pass runs the duplicate-scan loop
    // (sched/ir.rs:409 / sched/lower.rs ~428) to completion BEFORE
    // the undeclared-worker scan (sched/ir.rs:382 /
    // `check_worker_declared`), so the user sees the specific
    // `DuplicatePlaceWorker` message, not `UnknownPlaceWorker`. This
    // ordering was code-correct-by-inspection but UNpinned
    // (TASK-0093/0094 review); a refactor swapping the two scans would
    // silently change which error a user sees first — this test bites.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on { ghost, ghost };
}
";
    let err = lower_str(src).expect_err("dup+undeclared place worker must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::DuplicatePlaceWorker {
            kernel: "k".into(),
            worker: "ghost".into(),
        },
        "duplicate-worker detection must fire BEFORE undeclared-worker \
         detection on a placement set that is both repeated and undeclared"
    );
    // Strength guard: explicitly assert it is NOT the undeclared
    // variant, so a future change that loosens the kind match (or
    // reorders the scans) cannot pass by coincidence.
    assert!(
        !matches!(
            err.kind,
            SchedLowerErrorKind::UnknownPlaceWorker { .. }
        ),
        "must NOT surface UnknownPlaceWorker first; got {:?}",
        err.kind
    );
}

#[test]
fn positive_distinct_place_set_still_lowers() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { w0, w1 };
    place k on { w0, w1 };
}
";
    lower_str(src).expect("distinct place set must lower");
}

// --------------------------------------------------------------------
// TASK-0095: accessible_by names must resolve schedule-internally
// --------------------------------------------------------------------

#[test]
fn negative_undeclared_accessible_by_name() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    memory_region R { size = 1KB; accessible_by = { ghost }; };
}
";
    let err = lower_str(src).expect_err("undeclared accessible_by name must fail").first().clone();
    // TASK-0198: candidate union is {`__default` (synthetic class),
    // `host` (the declared worker)}. `ghost` → `host` is exactly one
    // deletion (distance 1), within bound max(1, 5/3) = 1 → the hint
    // is `host`. Whole-`.kind` assert_eq! strength preserved; the
    // computed suggestion is asserted as part of the expected value
    // (AC#2 — this migrated case is the positive half for this
    // variant).
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownAccessibleByName {
            region: "R".into(),
            name: "ghost".into(),
            suggestion: Some("host".into()),
        }
    );
}

#[test]
fn positive_accessible_by_resolves_class_and_worker_names() {
    // A worker_class name and a worker name both resolve internally.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class core { simd = none; };
    workers = { host : core, w0 : core };
    memory_region R { size = 1KB; accessible_by = { core, host }; };
}
";
    let ir = lower_str(src).expect("declared accessible_by names must lower");
    assert!(ir.memory_regions.contains_key("R"));
}

// --------------------------------------------------------------------
// TASK-0198: deterministic did-you-mean suggestions for the four
// unknown-name SchedLowerError variants (sched sibling of TASK-0096).
// Each variant gets a typo-class positive case (Some(closest)) and an
// unrelated-name negative case (None). The helper itself is unit-
// tested in `compiler::error::fuzzy_tests` — these pin only the
// SchedLowerError wiring and per-variant candidate set.
// --------------------------------------------------------------------

#[test]
fn negative_unknown_worker_class_with_suggestion() {
    // `core` is declared; the worker entry typos it as `cor`.
    // distance(cor, core) = 1 (one insertion); bound max(1, 3/3) = 1
    // → Some("core"). (`__default` is the only other candidate and
    // is far away.)
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class core { simd = none; };
    workers = { fe : cor };
}
";
    let err = lower_str(src).expect_err("typo'd worker_class must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownWorkerClass {
            worker: "fe".into(),
            class: "cor".into(),
            suggestion: Some("core".into()),
        }
    );
    // Display surfaces the hint.
    assert!(
        err.to_string().contains("did you mean `core`?"),
        "Display must carry the hint, got: {err}"
    );
}

#[test]
fn negative_unknown_worker_class_unrelated_no_suggestion() {
    // `zzzzzzzz` vs declared {core, __default} — far above the bound
    // → None (the "don't suggest nonsense" half).
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class core { simd = none; };
    workers = { fe : zzzzzzzz };
}
";
    let err = lower_str(src).expect_err("unrelated worker_class must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownWorkerClass {
            worker: "fe".into(),
            class: "zzzzzzzz".into(),
            suggestion: None,
        }
    );
}

#[test]
fn negative_unknown_memory_region_with_suggestion() {
    // `sram` declared; `place_data` typos it as `sra`.
    // distance(sra, sram) = 1; bound max(1, 3/3) = 1 → Some("sram").
    let src = "\
schedule for \"../prog.algo.nuc\" {
    memory_region sram { size = 32KB; };
    workers = { host };
    place_data d in sra;
}
";
    let err = lower_str(src).expect_err("typo'd memory_region must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownMemoryRegion {
            data: "d".into(),
            region: "sra".into(),
            suggestion: Some("sram".into()),
        }
    );
    assert!(
        err.to_string().contains("did you mean `sram`?"),
        "Display must carry the hint, got: {err}"
    );
}

#[test]
fn negative_unknown_memory_region_unrelated_no_suggestion() {
    // `zzzzzzzz` vs declared {sram} — far above bound → None.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    memory_region sram { size = 32KB; };
    workers = { host };
    place_data d in zzzzzzzz;
}
";
    let err = lower_str(src).expect_err("unrelated memory_region must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownMemoryRegion {
            data: "d".into(),
            region: "zzzzzzzz".into(),
            suggestion: None,
        }
    );
}

#[test]
fn negative_unknown_place_worker_with_suggestion() {
    // `host` declared; `place k on hostt` typos it.
    // distance(hostt, host) = 1; bound max(1, 5/3) = 1 → Some("host").
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on hostt;
}
";
    let err = lower_str(src).expect_err("typo'd place worker must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownPlaceWorker {
            kernel: "k".into(),
            worker: "hostt".into(),
            suggestion: Some("host".into()),
        }
    );
    assert!(
        err.to_string().contains("did you mean `host`?"),
        "Display must carry the hint, got: {err}"
    );
}

#[test]
fn negative_unknown_accessible_by_unrelated_no_suggestion() {
    // The None half for UnknownAccessibleByName (the migrated
    // `negative_undeclared_accessible_by_name` covers the Some half).
    // `zzzzzzzz` vs the union {__default, host} — far above bound
    // → None.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    memory_region R { size = 1KB; accessible_by = { zzzzzzzz }; };
}
";
    let err = lower_str(src).expect_err("unrelated accessible_by must fail").first().clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnknownAccessibleByName {
            region: "R".into(),
            name: "zzzzzzzz".into(),
            suggestion: None,
        }
    );
}

#[test]
fn suggestion_is_deterministic_across_repeated_lowering() {
    // Determinism guarantee (reproducibility gate / TASK-0145
    // lineage): the same (offending name, schedule) lowers to the
    // byte-identical suggestion every time — the candidate tables are
    // `BTreeMap`s and `suggest` sorts + uses a strict-< tie-break, so
    // no hash-iteration order can leak in. Two equal-distance
    // candidates (`hosta`/`hostb`) force the lexicographic tie-break;
    // it must pick `hosta` every run.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { hostb, hosta };
    place k on host;
}
";
    let first = lower_str(src).expect_err("must fail").first().clone();
    for _ in 0..16 {
        let again = lower_str(src).expect_err("must fail").first().clone();
        assert_eq!(
            first.kind, again.kind,
            "suggestion must be deterministic across runs"
        );
    }
    // `host` is distance 1 from BOTH `hosta` and `hostb`; the
    // lexicographically-first (`hosta`) wins, deterministically.
    assert_eq!(
        first.kind,
        SchedLowerErrorKind::UnknownPlaceWorker {
            kernel: "k".into(),
            worker: "host".into(),
            suggestion: Some("hosta".into()),
        }
    );
}

// --------------------------------------------------------------------
// Sanity: an algo_path round-trips
// --------------------------------------------------------------------

#[test]
fn algo_path_is_preserved() {
    let src = "\
schedule for \"my/path.algo.nuc\" {
    workers = { host };
}
";
    let ir = lower_str(src).expect("must lower");
    assert_eq!(ir.algo_path, "my/path.algo.nuc");
}

// --------------------------------------------------------------------
// TASK-0196: located schedule-lowering diagnostics
//
// SchedLowerError now carries the byte span of the offending source
// node. These tests pin the EXACT (line, column) — recomputed from the
// crafted source by finding the offending token and feeding its offset
// through `offset_to_line_col`, so the assertion tracks the real source
// position, not a guessed constant — and the driver-facing
// `display_with_src` rendering. A second test pins the genuinely
// position-less set (honest-partial; the TASK-0090 doc-lie lesson:
// docs/code/test must agree exactly).
// --------------------------------------------------------------------

/// Resolve a lowered error's stored byte span to a 1-based
/// `(line, column)` against `src`, the way the driver does
/// (`SchedLowerError::display_with_src`). Panics if the variant is
/// position-less — every case asserted here is a single-offending-node
/// variant that MUST carry a span (AC#2), so a `None` is a real
/// regression, not test noise.
fn sched_err_line_col(src: &str, err: &SchedLowerError) -> (usize, usize) {
    let span = err
        .span
        .clone()
        .unwrap_or_else(|| panic!("expected a located error, got position-less: {err:?}"));
    offset_to_line_col(src, span.start)
}

#[test]
fn located_sched_errors_carry_correct_line_col() {
    // Case 1: duplicate `worker_class`. The diagnostic must point at
    // the *second* (duplicate) `core` identifier, on line 3.
    {
        let src = "\
schedule for \"../p.algo.nuc\" {
    worker_class core { simd = none; };
    worker_class core { simd = neon128; };
    workers = { fe : core };
}
";
        let err = lower_str(src).expect_err("duplicate worker_class must error").first().clone();
        assert!(
            matches!(err.kind, SchedLowerErrorKind::DuplicateWorkerClass(ref n) if n == "core"),
            "got {err:?}"
        );
        // The duplicate decl is the second `worker_class core`; its
        // identifier `core` is the offending token.
        let second_decl = src
            .match_indices("worker_class core")
            .nth(1)
            .expect("two `worker_class core`")
            .0;
        let core_off = second_decl + "worker_class ".len();
        let expected = offset_to_line_col(src, core_off);
        assert_eq!(expected, (3, 18), "sanity: duplicate `core` at line 3 col 18");
        assert_eq!(
            sched_err_line_col(src, &err),
            expected,
            "DuplicateWorkerClass must point at the duplicate decl's identifier"
        );
        assert_eq!(
            err.display_with_src(src),
            "duplicate `worker_class` declaration `core` at 3:18"
        );
    }

    // Case 2: `place` on an undeclared worker. Points at the worker
    // token `bogus` on line 3.
    {
        let src = "\
schedule for \"../p.algo.nuc\" {
    workers = { host };
    place k on bogus;
}
";
        let err = lower_str(src).expect_err("unknown place worker must error").first().clone();
        assert!(
            matches!(
                err.kind,
                SchedLowerErrorKind::UnknownPlaceWorker { ref kernel, ref worker, .. }
                    if kernel == "k" && worker == "bogus"
            ),
            "got {err:?}"
        );
        // TASK-0198: this span-locating test keeps its full
        // variant+payload discriminating power (the `matches!` above,
        // `..` only admits the new field) AND additionally pins the
        // suggestion: `bogus` vs the sole worker `host` is above the
        // bound max(1, 5/3) = 1 → None (strengthened, not weakened).
        assert!(
            matches!(
                err.kind,
                SchedLowerErrorKind::UnknownPlaceWorker { ref suggestion, .. }
                    if suggestion.is_none()
            ),
            "expected no suggestion for `bogus`, got {err:?}"
        );
        let bogus_at = src.find("bogus").expect("`bogus` in source");
        let expected = offset_to_line_col(src, bogus_at);
        assert_eq!(expected, (3, 16), "sanity: `bogus` at line 3 col 16");
        assert_eq!(
            sched_err_line_col(src, &err),
            expected,
            "UnknownPlaceWorker must point at the undeclared worker token"
        );
        assert_eq!(
            err.display_with_src(src),
            "`place k on bogus` references undeclared worker `bogus` at 3:16"
        );
    }

    // Case 3: undeclared `accessible_by` name (the relocated (b)
    // check). Points at the offending name `ghost` on line 3.
    {
        let src = "\
schedule for \"../p.algo.nuc\" {
    workers = { host };
    memory_region R { size = 1KB; accessible_by = { ghost }; };
}
";
        let err = lower_str(src).expect_err("undeclared accessible_by must error").first().clone();
        assert!(
            matches!(
                err.kind,
                SchedLowerErrorKind::UnknownAccessibleByName { ref region, ref name, .. }
                    if region == "R" && name == "ghost"
            ),
            "got {err:?}"
        );
        // TASK-0198: candidate union {`__default`, `host`}; `ghost`
        // → `host` is one deletion (distance 1 ≤ bound 1) → the hint
        // is `host`. Span discriminating power preserved (matches!
        // above), suggestion additionally pinned (strengthened).
        assert!(
            matches!(
                err.kind,
                SchedLowerErrorKind::UnknownAccessibleByName { ref suggestion, .. }
                    if suggestion.as_deref() == Some("host")
            ),
            "expected suggestion `host` for `ghost`, got {err:?}"
        );
        let ghost_at = src.find("ghost").expect("`ghost` in source");
        let expected = offset_to_line_col(src, ghost_at);
        assert_eq!(expected, (3, 53), "sanity: `ghost` at line 3 col 53");
        assert_eq!(
            sched_err_line_col(src, &err),
            expected,
            "UnknownAccessibleByName must point at the offending name token"
        );
        // TASK-0198: the located display now also carries the
        // did-you-mean hint (the suggestion is `Some("host")`); the
        // ` at L:C` suffix is unchanged (span handling untouched —
        // TASK-0196 contract intact).
        assert_eq!(
            err.display_with_src(src),
            "`memory_region R` `accessible_by` lists `ghost`, \
             which is not a declared `worker_class` or worker \
             -- did you mean `host`? at 3:53"
        );
    }

    // Case 4: duplicate worker name across the entry list. Points at
    // the *second* `host` on line 2.
    {
        let src = "\
schedule for \"../p.algo.nuc\" {
    workers = { host, host };
}
";
        let err = lower_str(src).expect_err("duplicate worker must error").first().clone();
        assert!(
            matches!(err.kind, SchedLowerErrorKind::DuplicateWorker(ref n) if n == "host"),
            "got {err:?}"
        );
        let second_host = src.match_indices("host").nth(1).expect("two `host`").0;
        let expected = offset_to_line_col(src, second_host);
        assert_eq!(expected, (2, 23), "sanity: 2nd `host` at line 2 col 23");
        assert_eq!(
            sched_err_line_col(src, &err),
            expected,
            "DuplicateWorker must point at the repeated entry's token"
        );
    }
}

/// The two genuinely position-less cases stay `span: None` on purpose
/// (honest-partial — see `SchedLowerError` type docs). This pins that
/// decision so a future change that silently attaches a (likely wrong)
/// span is caught, and asserts the docs/code/test agree exactly (the
/// TASK-0090 doc-lie lesson, applied).
#[test]
fn position_less_variants_have_no_span() {
    // (1) MissingWorkersDecl — the error is the *absence* of a
    // `workers = ...` directive; no source token to point at.
    {
        let src = "\
schedule for \"../p.algo.nuc\" {
    place k on host;
}
";
        let err = lower_str(src).expect_err("missing workers decl must error").first().clone();
        assert!(
            matches!(err.kind, SchedLowerErrorKind::MissingWorkersDecl),
            "got {err:?}"
        );
        assert!(
            err.span.is_none(),
            "MissingWorkersDecl is documented position-less; got {:?}",
            err.span
        );
        // Display falls back to the kind alone — no fabricated location.
        assert_eq!(err.display_with_src(src), err.kind.to_string());
    }

    // (2) DuplicateWorkerClass raised from the SYNTHETIC default-class
    // collision branch (a user class literally named the synthetic
    // default). The collision is against a synthesised class with no
    // source token, and that branch has no user-decl `Spanned` in
    // scope — documented position-less. (The common
    // `DuplicateWorkerClass` from two real decls IS located — see
    // `located_sched_errors_carry_correct_line_col` case 1.)
    {
        let src = "\
schedule for \"../p.algo.nuc\" {
    worker_class __default { simd = none; };
    workers = { host };
}
";
        let err = lower_str(src).expect_err("default-class collision must error").first().clone();
        assert!(
            matches!(
                err.kind,
                SchedLowerErrorKind::DuplicateWorkerClass(ref n) if n == DEFAULT_WORKER_CLASS
            ),
            "got {err:?}"
        );
        assert!(
            err.span.is_none(),
            "synthetic default-class collision is documented position-less; got {:?}",
            err.span
        );
        assert_eq!(err.display_with_src(src), err.kind.to_string());
    }
}

// --------------------------------------------------------------------
// TASK-0200: multi-error reporting (sched analog of TASK-0092)
//
// These tests pin the AC#1 / AC#3 multi-error counting contract: the
// pass returns ALL genuinely-independent SchedLowerError violations in
// one bundle, the parametric fixture iterates BOTH dimensions (K
// duplicate-decl errors × L zero-buffer-option errors) over ≥3
// distinct values each, and the cascade-suppression infrastructure
// (the algo cycle-3 design transferred verbatim) is wired but has no
// live trigger on today's variant set (honest-partial — disclosed in
// the SchedLowerErrors / lower_sched docs).
//
// Single-shape OR single-dimension fixtures are the masking-defect
// class that bit TASK-0080/0081/0087 and the prior TASK-0092 cycles.
// Both dimensions are iterated here from the start.
// --------------------------------------------------------------------

/// A WELL-FORMED schedule still lowers to `Ok(SchedIR)` under
/// multi-error (AC#3: zero behaviour change for valid input at the
/// unit level — the determinism gate proves byte-identical at the
/// integration level). Locks in the multi-error infrastructure
/// against an accidental "always-Err" regression that the negative
/// tests alone would not catch.
#[test]
fn valid_schedule_still_lowers_under_multi_error() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class core { simd = none; };
    memory_region sram { size = 32KB; };
    workers = { host : core, w0 : core };
    place k on host;
    place_data foo in sram;
}
";
    let ir = lower_str(src).expect("a well-formed schedule must lower under multi-error");
    assert_eq!(ir.workers.len(), 2);
    assert_eq!(ir.worker_classes.len(), 1);
    assert_eq!(ir.memory_regions.len(), 1);
    assert_eq!(ir.places.len(), 1);
    assert_eq!(ir.place_data.len(), 1);
}

/// **K × L parametric fixture** (TASK-0200, mirrors TASK-0092 cycle-3
/// `transitive_cascade_collapses_for_any_k_l`).
///
/// Both dimensions iterate ≥3 distinct values, exactly the discipline
/// that closed the masking-defect class:
///
/// - **K dimension** (∈ {1, 2, 3, 5}): K duplicate `worker_class`
///   decls, each pair `worker_class ck_i { ... }; worker_class ck_i
///   { ... };` — the SECOND decl fires
///   [`SchedLowerErrorKind::DuplicateWorkerClass`]. K independent
///   class-level errors.
/// - **L dimension** (∈ {1, 2, 3}): L `transfer` directives each with
///   `buffer=0` — each fires
///   [`SchedLowerErrorKind::ZeroBufferOption`]. L independent
///   option-level errors.
///
/// Expected count: **EXACTLY K + L** errors for every (K, L)
/// combination. The pre-fix single-error pass would have aborted at
/// the first violation; the multi-error pass must surface every one.
///
/// Two dimensions of different error CLASSES (decl-level vs
/// option-level) means a defect that under-reports either class
/// shows up at K = 0 OR L = 0 boundary; a defect that over-reports
/// (cascades into spurious extras) shows up at large K + L. Both
/// failure modes bite this fixture.
#[test]
fn sched_multi_error_independents_count_for_any_k_l() {
    for k in [1usize, 2, 3, 5] {
        for l in [1usize, 2, 3] {
            let mut src = String::from("schedule for \"../prog.algo.nuc\" {\n");
            // K independent DuplicateWorkerClass — each `ck_i` pair
            // produces ONE duplicate error (the second decl). Each
            // pair is independent: distinct class names, no
            // cross-pair dependencies.
            for i in 0..k {
                src.push_str(&format!(
                    "    worker_class ck{i} {{ simd = none; }};\n"
                ));
                src.push_str(&format!(
                    "    worker_class ck{i} {{ simd = none; }};\n"
                ));
            }
            // The L independent ZeroBufferOption transfers need
            // distinct data names so each fires its own
            // ZeroBufferOption (rather than collapsing into a single
            // DuplicateTransfer).
            for j in 0..l {
                src.push_str(&format!(
                    "    transfer x{j} : sync, buffer=0;\n"
                ));
            }
            // Minimal workers decl so the schedule isn't also
            // rejected with MissingWorkersDecl.
            src.push_str("    workers = { host };\n");
            src.push_str("}\n");

            let errs = lower_str(&src)
                .expect_err("K duplicate-classes + L zero-buffer transfers must error");
            assert_eq!(
                errs.errors().len(),
                k + l,
                "(K={k}, L={l}): expected EXACTLY K+L={} independent errors, got {} — \
                 source:\n{src}",
                k + l,
                errs.errors().len()
            );

            // Verify the error kinds are exactly the expected mix.
            let dup_class_count = errs
                .errors()
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        SchedLowerErrorKind::DuplicateWorkerClass(_)
                    )
                })
                .count();
            let zero_buf_count = errs
                .errors()
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        SchedLowerErrorKind::ZeroBufferOption { .. }
                    )
                })
                .count();
            assert_eq!(
                dup_class_count, k,
                "(K={k}, L={l}): expected EXACTLY K={k} DuplicateWorkerClass errors, got {dup_class_count}"
            );
            assert_eq!(
                zero_buf_count, l,
                "(K={k}, L={l}): expected EXACTLY L={l} ZeroBufferOption errors, got {zero_buf_count}"
            );
        }
    }
}

/// Every error retains its own correct `(line, column)` after
/// multi-error accumulation (AC#1: each error carries its located
/// span). Three independent duplicate-decl errors at known line
/// positions; each must point at the duplicate (second) decl's
/// identifier token on its own line.
#[test]
fn sched_multi_error_each_error_carries_its_own_line_col() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class c0 { simd = none; };
    worker_class c0 { simd = none; };
    worker_class c1 { simd = none; };
    worker_class c1 { simd = none; };
    worker_class c2 { simd = none; };
    worker_class c2 { simd = none; };
    workers = { host };
}
";
    let errs = lower_str(src).expect_err("three independent duplicate-class decls must error");
    assert_eq!(
        errs.errors().len(),
        3,
        "expected exactly 3 DuplicateWorkerClass errors, got {} — source:\n{src}",
        errs.errors().len()
    );
    // Sanity: each error is DuplicateWorkerClass at the right name.
    for (i, e) in errs.errors().iter().enumerate() {
        match &e.kind {
            SchedLowerErrorKind::DuplicateWorkerClass(n) => {
                assert_eq!(n, &format!("c{i}"), "error {i} should be for c{i}");
            }
            other => panic!("error {i} must be DuplicateWorkerClass, got {other:?}"),
        }
        // Each error has its own span pointing at the SECOND `c<i>`
        // identifier token. We compute the expected (line, col)
        // against the source (no guessed constant) and assert.
        let nth_pair_second_at = src
            .match_indices(&format!("worker_class c{i}"))
            .nth(1)
            .expect("two `worker_class c<i>` per pair")
            .0;
        let ident_at = nth_pair_second_at + "worker_class ".len();
        let expected = offset_to_line_col(src, ident_at);
        let actual = offset_to_line_col(
            src,
            e.span
                .clone()
                .unwrap_or_else(|| panic!("error {i} must carry a span: {e:?}"))
                .start,
        );
        assert_eq!(
            actual, expected,
            "error {i} must point at the duplicate `c{i}` identifier on its own line"
        );
    }
}

/// Multi-error determinism (AC#3, the PRD §10.1 reproducibility
/// guarantee at the err-path): the SAME input produces the SAME
/// ordered error sequence on every run. No `HashMap`/`HashSet` in
/// the error-collection path — the `Accum::failed_decls` is a
/// `BTreeMap`, the `worker_class_refs` / `accessible_by_refs`
/// post-collection tables are `Vec`s sorted with stable
/// `sort_by(name)`, and the directive walk is in source order.
/// Running lowering 16× and asserting bundle equality each time
/// proves no hash-iteration order can leak in (the chumsky
/// nondeterminism class that bit TASK-0080/0081 cannot recur here).
#[test]
fn sched_multi_error_is_deterministic_across_repeated_lowering() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    worker_class ca { simd = none; };
    worker_class ca { simd = none; };
    transfer t0 : sync, buffer=0;
    worker_class cb { simd = none; };
    worker_class cb { simd = none; };
    transfer t1 : sync, buffer=0;
    workers = { host };
}
";
    let first = lower_str(src).expect_err("must error");
    for _ in 0..16 {
        let again = lower_str(src).expect_err("must error");
        assert_eq!(
            first, again,
            "multi-error bundle must be deterministic across repeated lowering — \
             same input, identical ordered error sequence (PartialEq forwards \
             through SchedLowerError to .kind only, span informational)"
        );
    }
    // Sanity-check the bundle shape so a refactor that breaks the
    // accumulation order is caught here, not in a flaky downstream
    // assertion.
    //
    // Pass 1 collects declarations (worker_class, memory_region,
    // workers) in source order; pass 2 walks `place` / `place_data` /
    // `loop` / `transfer` / `check` in source order. So `ca` and `cb`
    // duplicate-class errors (pass 1) come BEFORE `t0` / `t1`
    // zero-buffer errors (pass 2), even though `t0` is textually
    // between `ca` and `cb` in source. This is two-pass-source-order,
    // not strict-source-order — a deliberate property of the design
    // (separating declaration collection from validation, the same
    // shape as the algorithm side), and the determinism guarantee
    // holds: every run produces the same order.
    assert_eq!(first.errors().len(), 4);
    assert!(matches!(
        first.errors()[0].kind,
        SchedLowerErrorKind::DuplicateWorkerClass(ref n) if n == "ca"
    ));
    assert!(matches!(
        first.errors()[1].kind,
        SchedLowerErrorKind::DuplicateWorkerClass(ref n) if n == "cb"
    ));
    assert!(matches!(
        first.errors()[2].kind,
        SchedLowerErrorKind::ZeroBufferOption { ref data } if data == "t0"
    ));
    assert!(matches!(
        first.errors()[3].kind,
        SchedLowerErrorKind::ZeroBufferOption { ref data } if data == "t1"
    ));
}

/// Cascade-suppression Path 1 (`failed_decls`-keyed name cascade —
/// the algo cycle-3 design transferred verbatim) is wired but has NO
/// live trigger on today's sched-lowering variant set: every
/// `worker_class` / `memory_region` / worker entry that survives its
/// duplicate check is unconditionally inserted into the symbol table,
/// so `Accum::failed_decls` stays empty in practice (no decl-level
/// "evaluation failure" path exists). This test pins the
/// honest-partial disclosure recorded in the [`SchedLowerErrors`]
/// type docs and the [`lower_sched`] module doc.
///
/// Concretely: a `place_data foo in nowhere` (UnknownMemoryRegion) is
/// reported as an INDEPENDENT error — there is no upstream cascade
/// root for it (the user never declared a region that itself failed;
/// they simply typed an unknown name). Path 1 has nothing to
/// suppress.
///
/// NOTE: Path 2 (`workers_missing`-keyed UnknownPlaceWorker
/// suppression) IS live today and is pinned by the parametric
/// [`workers_missing_cascade_collapses_place_unknown_worker_for_any_n`]
/// test below. The two paths are intentionally separate disclosures.
///
/// This test must be updated WHEN a sched construct gains a Path-1
/// poison-source path (e.g. a memory_region body that evaluates an
/// expression that can fail). Until then it stands as the Path-1
/// disclosure pin.
#[test]
fn sched_failed_decls_cascade_path_has_no_live_trigger_today() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place_data foo in nowhere;
    place_data bar in elsewhere;
}
";
    let errs = lower_str(src).expect_err("two unknown-region refs must error");
    // BOTH UnknownMemoryRegion errors surface independently — there
    // is no Path-1 upstream cascade root in today's variant set, so
    // the failed_decls-keyed suppression has nothing to fire on.
    assert_eq!(
        errs.errors().len(),
        2,
        "two independent UnknownMemoryRegion errors must BOTH surface; Path 1 \
         (failed_decls-keyed) cascade-suppression is forward-looking, not active"
    );
    assert!(matches!(
        errs.errors()[0].kind,
        SchedLowerErrorKind::UnknownMemoryRegion { ref region, .. } if region == "nowhere"
    ));
    assert!(matches!(
        errs.errors()[1].kind,
        SchedLowerErrorKind::UnknownMemoryRegion { ref region, .. } if region == "elsewhere"
    ));
}

/// Cascade-suppression Path 2 (`workers_missing`-keyed
/// UnknownPlaceWorker suppression) — the ONE in-pass cascade trigger
/// that FIRES today. When the schedule has no `workers = ...`
/// directive, `ir.workers` stays empty by construction, and every
/// subsequent `place X on W` necessarily fires `UnknownPlaceWorker{W}`
/// as a pure cascade of the already-reported MissingWorkersDecl
/// root. Path 2 suppresses those so the user sees the single ROOT
/// diagnostic instead of N cascade lines.
///
/// PARAMETRIC over N in {1, 2, 3, 5} (the cycle-3 masking-defect-class
/// discipline: a single-shape fixture would let the suppression
/// silently regress to "first one only" or "all N leaked"). For every
/// N: assert errors().len() == 1 AND the surviving error is the
/// position-less MissingWorkersDecl root AND no UnknownPlaceWorker
/// leaks. Determinism asserted across two runs per N.
///
/// Without the cycle-2 transitive-fix at lower.rs case-1
/// (`workers_missing` flag + Path-2 branch in
/// `is_cascade_of_failed_decl`), this fixture would FAIL with
/// errors().len() == 1 + N (the leaked UnknownPlaceWorker per place).
/// The K×L methodology from TASK-0092 cycle-3 / TASK-0087 cycle-4
/// applied at the sched-lowering layer.
#[test]
fn workers_missing_cascade_collapses_place_unknown_worker_for_any_n() {
    for n in [1usize, 2, 3, 5] {
        let mut src = String::from("schedule for \"../prog.algo.nuc\" {\n");
        // No `workers = ...` directive (the cascade root).
        // N `place k_i on w_i` directives — each would emit
        // `UnknownPlaceWorker{w_i}` as a pure cascade of
        // MissingWorkersDecl absent Path-2 suppression.
        for i in 0..n {
            src.push_str(&format!("    place k{i} on w{i};\n"));
        }
        src.push_str("}\n");

        let errs = lower_str(&src)
            .expect_err("missing workers + N places must error");
        // Determinism cross-check: re-lower the same source; the
        // bundle must be byte-identical.
        let errs2 = lower_str(&src)
            .expect_err("missing workers + N places must error (run 2)");
        assert_eq!(
            errs, errs2,
            "(N={n}): multi-error bundle must be deterministic"
        );

        assert_eq!(
            errs.errors().len(),
            1,
            "(N={n}): expected EXACTLY 1 root error (MissingWorkersDecl); \
             the N=`{n}` UnknownPlaceWorker cascades MUST be suppressed \
             by the workers_missing Path-2 rule. Got {} — source:\n{src}",
            errs.errors().len()
        );
        assert!(
            matches!(errs.errors()[0].kind, SchedLowerErrorKind::MissingWorkersDecl),
            "(N={n}): surviving error must be MissingWorkersDecl root, got {:?}",
            errs.errors()[0].kind
        );
        // Explicit non-leak guard: no UnknownPlaceWorker survived.
        let leaked_unknown_worker = errs.errors().iter().any(|e| {
            matches!(e.kind, SchedLowerErrorKind::UnknownPlaceWorker { .. })
        });
        assert!(
            !leaked_unknown_worker,
            "(N={n}): no UnknownPlaceWorker may leak — every `place X on W` \
             is a transitive cascade of the already-reported \
             MissingWorkersDecl root: {:?}",
            errs.errors()
        );
    }
}

/// Negative-control for Path 2: when `workers = ...` IS present but a
/// `place k on W` references a worker name NOT in the symbol table,
/// that is a GENUINE INDEPENDENT error (user typo, not a cascade of
/// MissingWorkersDecl), and the Path-2 rule must NOT suppress it. This
/// pins that the workers_missing suppression is narrow — it triggers
/// ONLY when the workers decl itself is absent, never when the decl
/// is present but a reference is wrong. Guards against over-
/// suppression regression.
#[test]
fn workers_present_but_unknown_place_worker_surfaces_independently() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on no_such_worker_typo;
}
";
    let errs = lower_str(src)
        .expect_err("unknown-worker reference with workers-decl present must error");
    assert_eq!(
        errs.errors().len(),
        1,
        "expected exactly 1 independent UnknownPlaceWorker (no MissingWorkersDecl, \
         no over-suppression by Path 2) — got {:?}",
        errs.errors()
    );
    assert!(
        matches!(
            &errs.errors()[0].kind,
            SchedLowerErrorKind::UnknownPlaceWorker { worker, .. } if worker == "no_such_worker_typo"
        ),
        "surviving error must be UnknownPlaceWorker{{no_such_worker_typo}}, got {:?}",
        errs.errors()[0].kind
    );
}
