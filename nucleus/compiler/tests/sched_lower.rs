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

use compiler::sched::{
    lower_sched, parse_sched, NotifyKind, PartitionKind, ResolvedLoopOption, ResolvedPlaceTarget,
    ResolvedTransferOption, SchedIR, SchedLowerError, ViolationKind, DEFAULT_WORKER_CLASS,
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
fn lower_str(src: &str) -> Result<SchedIR, SchedLowerError> {
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
    let err = lower_str(src).expect_err("missing workers decl must fail");
    assert_eq!(err, SchedLowerError::MissingWorkersDecl);
}

#[test]
fn negative_duplicate_workers_decl() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    workers = { w0 };
}
";
    let err = lower_str(src).expect_err("two workers decls must fail");
    assert_eq!(err, SchedLowerError::DuplicateWorkersDecl);
}

#[test]
fn negative_duplicate_worker_name_in_one_decl() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host, host };
}
";
    let err = lower_str(src).expect_err("duplicate worker name must fail");
    assert_eq!(err, SchedLowerError::DuplicateWorker("host".into()));
}

#[test]
fn negative_unknown_worker_class_reference() {
    // Typed worker entry names a class that has no `worker_class` decl.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { fe : missing_class };
}
";
    let err = lower_str(src).expect_err("unknown worker_class must fail");
    assert_eq!(
        err,
        SchedLowerError::UnknownWorkerClass {
            worker: "fe".into(),
            class: "missing_class".into(),
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
    let err = lower_str(src).expect_err("duplicate worker_class must fail");
    assert_eq!(err, SchedLowerError::DuplicateWorkerClass("core".into()));
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
    let err = lower_str(src).expect_err("unknown memory_region must fail");
    assert_eq!(
        err,
        SchedLowerError::UnknownMemoryRegion {
            data: "foo".into(),
            region: "nowhere".into(),
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
    let err = lower_str(src).expect_err("duplicate memory_region must fail");
    assert_eq!(err, SchedLowerError::DuplicateMemoryRegion("sram".into()));
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
    let err = lower_str(src).expect_err("duplicate place must fail");
    assert_eq!(err, SchedLowerError::DuplicatePlace { kernel: "k".into() });
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
    let err = lower_str(src).expect_err("duplicate place_data must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicatePlaceData { data: "foo".into() }
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
    let err = lower_str(src).expect_err("duplicate loop must fail");
    assert_eq!(err, SchedLowerError::DuplicateLoop { var: "y".into() });
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
    let err = lower_str(src).expect_err("duplicate transfer must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicateTransfer { data: "img".into() }
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
    let err = lower_str(src).expect_err("duplicate check must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicateCheck {
            var: "frame".into()
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
    let err = lower_str(src).expect_err("block=0 must fail");
    assert_eq!(
        err,
        SchedLowerError::ZeroLoopOption {
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
    let err = lower_str(src).expect_err("pipeline=0 must fail");
    assert_eq!(
        err,
        SchedLowerError::ZeroLoopOption {
            var: "y".into(),
            option: "pipeline".into()
        }
    );
}

#[test]
fn negative_zero_vectorize_loop_option() {
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop y : vectorize=0;
}
";
    let err = lower_str(src).expect_err("vectorize=0 must fail");
    assert_eq!(
        err,
        SchedLowerError::ZeroLoopOption {
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
    let err = lower_str(src).expect_err("buffer=0 must fail");
    assert_eq!(
        err,
        SchedLowerError::ZeroBufferOption { data: "img".into() }
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
    let err = lower_str(src).expect_err("unknown worker must fail");
    assert_eq!(
        err,
        SchedLowerError::UnknownPlaceWorker {
            kernel: "k".into(),
            worker: "bogus".into()
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
    let err = lower_str(src).expect_err("unknown worker in set must fail");
    assert_eq!(
        err,
        SchedLowerError::UnknownPlaceWorker {
            kernel: "k".into(),
            worker: "bogus".into()
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
    let err = lower_str(src).expect_err("default-class collision must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicateWorkerClass(DEFAULT_WORKER_CLASS.to_string())
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
    let err = lower_str(src).expect_err("duplicate loop option must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicateLoopOption {
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
    let err = lower_str(src).expect_err("sync+async must fail");
    assert_eq!(
        err,
        SchedLowerError::ConflictingTransferMode { data: "x".into() }
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
    let err = lower_str(src).expect_err("duplicate buffer option must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicateTransferOption {
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
    let err = lower_str(src).expect_err("duplicate place worker must fail");
    assert_eq!(
        err,
        SchedLowerError::DuplicatePlaceWorker {
            kernel: "k".into(),
            worker: "w0".into(),
        }
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
    let err = lower_str(src).expect_err("undeclared accessible_by name must fail");
    assert_eq!(
        err,
        SchedLowerError::UnknownAccessibleByName {
            region: "R".into(),
            name: "ghost".into(),
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
