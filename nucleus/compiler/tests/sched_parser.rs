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
//! `14-hearing-aid/schedules/embedded_multimcu.sched.nuc` now writes
//! the grammar-conformant `check loop frame : ...;`. TASK-0079
//! reconciled the example with the PRD/grammar (the example was fixed,
//! the grammar was NOT relaxed — the `check`-qualifier slot is kept
//! mandatory for future `check transfer X : buffer_max = N;`). The
//! parser MUST accept this file; see `parses_14_hearing_aid_embedded_multimcu`.

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
fn parses_02_split_add_naive() {
    // TASK-0021: smoke-test variant of example 02 — single worker,
    // same shape as 01-elementwise-add/naive. Verifies the example
    // file parses under the trivial single-worker schedule.
    let src = read_example("02-split-add/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("02-split-add/naive must parse");
    assert_eq!(ast.algo_path, "../prog.algo.nuc");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 4, "four place directives");
    assert_eq!(ast.count_loops(), 0);
    assert_eq!(ast.count_transfers(), 0);
    assert_eq!(ast.count_checks(), 0);
}

#[test]
fn parses_02_split_add_split() {
    // TASK-0021: the load-bearing two-worker schedule. The first
    // schedule in the example matrix to declare actual `transfer`
    // directives. If counts here drift, look at split.sched.nuc and
    // confirm both still-required transfers (a host->w0, b host->w0,
    // c w0->host) are still listed.
    let src = read_example("02-split-add/schedules/split.sched.nuc");
    let ast = parse_sched(&src).expect("02-split-add/split must parse");
    assert_eq!(ast.algo_path, "../prog.algo.nuc");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 4, "four place directives");
    assert_eq!(ast.count_loops(), 0);
    assert_eq!(
        ast.count_transfers(),
        3,
        "three transfers — a, b host->w0; c w0->host"
    );
    assert_eq!(ast.count_checks(), 0);

    // Spot-check: all three transfers carry exactly `[Sync]`.
    let transfers: Vec<_> = ast
        .directives
        .iter()
        .filter_map(|d| match &d.node {
            Directive::Transfer(t) => Some(t),
            _ => None,
        })
        .collect();
    let names: std::collections::BTreeSet<&str> =
        transfers.iter().map(|t| t.data.as_str()).collect();
    assert_eq!(
        names,
        ["a", "b", "c"].iter().copied().collect(),
        "expected transfers a, b, c"
    );
    for t in &transfers {
        assert_eq!(
            t.options,
            vec![TransferOption::Sync],
            "transfer {} should be sync-only",
            t.data.node
        );
    }

    // Spot-check: `add` is placed on the single worker `w0` (Simple
    // form: PlaceTarget::One), not a worker set.
    let add = ast
        .directives
        .iter()
        .find_map(|d| match &d.node {
            Directive::Place(p) if p.kernel.node == "add" => Some(p),
            _ => None,
        })
        .expect("add place");
    match &add.target {
        PlaceTarget::One(w) => assert_eq!(w.node, "w0", "add should be on w0"),
        other => panic!("expected single-worker target for add, got {:?}", other),
    }
}

#[test]
fn parses_03_reduction_naive() {
    // TASK-0022: smoke-test schedule for example 03 — single worker
    // (host), four placements (load_input, save_output, accumulate,
    // combine). No loops, no transfers, no checks.
    let src = read_example("03-reduction/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("03-reduction/naive must parse");
    assert_eq!(ast.algo_path, "../prog.algo.nuc");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 4, "four place directives");
    assert_eq!(ast.count_loops(), 0);
    assert_eq!(ast.count_transfers(), 0);
    assert_eq!(ast.count_checks(), 0);
}

#[test]
fn parses_03_reduction_distributed() {
    // TASK-0022: the stretch distributed schedule for example 03.
    // Parses cleanly even though emit currently rejects distributed
    // placement (TASK-0117/TASK-0126). Five workers (host + w0..w3),
    // four placements, one loop directive (partition=blocks), two
    // transfers (a, partials), no checks.
    let src = read_example("03-reduction/schedules/distributed.sched.nuc");
    let ast = parse_sched(&src).expect("03-reduction/distributed must parse");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 4, "four place directives");
    assert_eq!(ast.count_loops(), 1, "loop w : partition=blocks");
    assert_eq!(ast.count_transfers(), 2, "transfer a, transfer partials");
    assert_eq!(ast.count_checks(), 0);

    // Spot-check: accumulate is on a 4-worker set, the rest are
    // single-worker (host).
    let acc = ast
        .directives
        .iter()
        .find_map(|d| match &d.node {
            Directive::Place(p) if p.kernel.node == "accumulate" => Some(p),
            _ => None,
        })
        .expect("accumulate place");
    match &acc.target {
        PlaceTarget::Many(v) => assert_eq!(v.len(), 4, "accumulate distributed over 4 workers"),
        other => panic!("expected Many target for accumulate, got {:?}", other),
    }

    // Both transfers should be sync-only.
    let transfers: Vec<_> = ast
        .directives
        .iter()
        .filter_map(|d| match &d.node {
            Directive::Transfer(t) => Some(t),
            _ => None,
        })
        .collect();
    for t in &transfers {
        assert_eq!(
            t.options,
            vec![TransferOption::Sync],
            "transfer {} should be sync-only",
            t.data.node
        );
    }
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
        .find_map(|d| match &d.node {
            Directive::Place(p) if p.kernel.node == "blur3" => Some(p),
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
        .find_map(|d| match &d.node {
            Directive::Loop(l) if l.var.node == "x" => Some(l),
            _ => None,
        })
        .expect("loop x");
    assert_eq!(loop_x.options.len(), 3);
    assert!(loop_x.options.contains(&LoopOption::Block(64)));
    assert!(loop_x.options.contains(&LoopOption::Vectorize(8)));
    assert!(loop_x.options.contains(&LoopOption::Reuse));
}

#[test]
fn parses_07_matmul_naive() {
    // TASK-0032: smoke-test schedule for example 07 — single worker
    // (host), four placements (load_a, load_b, save_c, madd). No
    // loops, no transfers, no checks.
    let src = read_example("07-matmul/schedules/naive.sched.nuc");
    let ast = parse_sched(&src).expect("07-matmul/naive must parse");
    assert_eq!(ast.algo_path, "../prog.algo.nuc");
    assert_eq!(ast.count_workers(), 1, "one workers decl");
    assert_eq!(ast.count_places(), 4, "four place directives");
    assert_eq!(ast.count_loops(), 0);
    assert_eq!(ast.count_transfers(), 0);
    assert_eq!(ast.count_checks(), 0);
}

#[test]
fn parses_07_matmul_blocked() {
    // TASK-0032: blocked schedule — 2D tiling via two `block=`
    // directives on `i` and `j` (PRD §6.3.3 stacked block=). Single
    // worker; no transfers.
    let src = read_example("07-matmul/schedules/blocked.sched.nuc");
    let ast = parse_sched(&src).expect("07-matmul/blocked must parse");
    assert_eq!(ast.count_workers(), 1);
    assert_eq!(ast.count_places(), 4);
    assert_eq!(ast.count_loops(), 2);
    assert_eq!(ast.count_transfers(), 0);

    // Both loops must carry exactly `block=8`.
    for var in ["i", "j"] {
        let l = ast
            .directives
            .iter()
            .find_map(|d| match &d.node {
                Directive::Loop(l) if l.var.node == var => Some(l),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing loop directive on `{}`", var));
        assert_eq!(l.options.len(), 1);
        assert!(l.options.contains(&LoopOption::Block(8)));
    }
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
        .find_map(|d| match &d.node {
            Directive::Loop(l) if l.var.node == "n" => Some(l),
            _ => None,
        })
        .expect("loop n");
    assert_eq!(loop_n.options, vec![LoopOption::Pipeline(3)]);

    // Spot-check `output` is sync.
    let output_xfer = ast
        .directives
        .iter()
        .find_map(|d| match &d.node {
            Directive::Transfer(t) if t.data.node == "output" => Some(t),
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

/// `14-hearing-aid/schedules/embedded_multimcu.sched.nuc` writes the
/// grammar-conformant `check loop frame : latency_max = 10ms;` (line
/// 105). TASK-0079 reconciled the example with the PRD §6.3.5 grammar
/// by fixing the example (the grammar was NOT relaxed — the
/// `check`-qualifier slot stays mandatory so a future
/// `check transfer X : buffer_max = N;` is unambiguous). The parser
/// MUST accept this file; this test is the AC#4 conformance evidence.
#[test]
fn parses_14_hearing_aid_embedded_multimcu() {
    let src = read_example("14-hearing-aid/schedules/embedded_multimcu.sched.nuc");
    let ast = parse_sched(&src)
        .expect("14-hearing-aid/embedded_multimcu must parse after TASK-0079");
    // The example has exactly one `check` directive (`check loop frame`).
    assert_eq!(ast.count_checks(), 1, "{:?}", ast);
    let check = ast
        .directives
        .iter()
        .find_map(|d| match &d.node {
            Directive::Check(c) => Some(c),
            _ => None,
        })
        .expect("check directive");
    assert_eq!(check.var.node, "frame");
    assert_eq!(check.asserts.len(), 1, "{:?}", check.asserts);
    match check.asserts[0] {
        // `latency_max = 10ms` -> 10_000_000 ns, unit retained.
        CheckAssert::LatencyMax(t) => {
            assert_eq!((t.nanos, t.original_unit), (10 * 1_000_000, TimeUnit::Ms));
        }
        ref other => panic!("expected LatencyMax, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// Negative tests (>= 4)
// --------------------------------------------------------------------

/// TASK-0079 reconciliation evidence: the bare `check VAR : ...;` form
/// (no `loop` qualifier) MUST be rejected. This pins the option-b
/// decision — the grammar was NOT relaxed to make `loop` optional, so
/// the `check`-qualifier slot stays unambiguous for a future
/// `check transfer X : buffer_max = N;`. Sibling positive test:
/// `parses_14_hearing_aid_embedded_multimcu`.
#[test]
fn negative_check_without_loop_qualifier_is_rejected() {
    // Identical to the conformant form except the `loop` keyword is
    // missing after `check`. `frame` is a real loop variable (line 4),
    // so the rejection is purely about the missing qualifier, not an
    // unknown variable.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    place k on host;
    loop frame : pipeline=3;
    check frame : latency_max = 10ms;
}
";
    let err = parse_sched(src).expect_err(
        "`check frame : ...` (no `loop` qualifier) must be rejected; \
         the grammar mandates `check loop VAR : ...` (PRD §6.3.5)",
    );
    // The bad token is on line 5 (`check frame`).
    assert_eq!(err.line, 5, "{:?}", err);
}

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
        .filter_map(|d| match &d.node {
            Directive::Check(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(checks.len(), 4);

    let lat = |var: &str| -> (u64, TimeUnit) {
        let c = checks
            .iter()
            .find(|c| c.var.node == var)
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
        .find_map(|d| match &d.node {
            Directive::WorkerClass(c) if c.name.node == "fe_core" => Some(c),
            _ => None,
        })
        .expect("fe_core");
    assert_eq!(fe.simd, Some(SimdSpec::None));

    let dsp = ast
        .directives
        .iter()
        .find_map(|d| match &d.node {
            Directive::WorkerClass(c) if c.name.node == "dsp_core" => Some(c),
            _ => None,
        })
        .expect("dsp_core");
    assert_eq!(dsp.simd, Some(SimdSpec::Named("neon128".to_string())));
}

/// TASK-0086 AC#3: per-node spans point at the CORRECT source
/// substring, validated against `error::offset_to_line_col`, and are
/// TIGHT (no leading / trailing layout swallowed). This is the
/// load-bearing proof that the `padded_spanned` primitive fixes the
/// span at the bare token/terminator before trailing layout is eaten,
/// and that `ident()`'s `map_with_span` captures just the identifier.
///
/// The source is laid out with one directive per line so byte offsets
/// and (line, column) are predictable.
#[test]
fn spans_point_at_correct_source_substring() {
    use compiler::error::offset_to_line_col;
    use compiler::sched::{PlaceDataDirective, PlaceDirective, WorkerClassDecl};

    // Line 1: `schedule for "p.algo.nuc" {`
    // Line 2: `worker_class cc { simd = none; };`
    // Line 3: `memory_region rgn { accessible_by = { cc, w0 }; };`
    // Line 4: `workers = { w0 : cc };`
    // Line 5: `place k on w0;`
    // Line 6: `place_data d in rgn;`
    // Line 7: `loop i : block=8;`
    // Line 8: `}`
    let src = "\
schedule for \"p.algo.nuc\" {
worker_class cc { simd = none; };
memory_region rgn { accessible_by = { cc, w0 }; };
workers = { w0 : cc };
place k on w0;
place_data d in rgn;
loop i : block=8;
}
";
    let ast = parse_sched(src).expect("must parse");

    // Helper: a span must (a) slice out exactly `want`, (b) start at
    // `(line, col)`, and (c) be TIGHT — the char immediately before
    // `start` is not part of the token and the char at `end` is not
    // whitespace that the span wrongly swallowed.
    let check = |span: &std::ops::Range<usize>, want: &str, lc: (usize, usize)| {
        assert_eq!(&src[span.clone()], want, "span must slice exactly `{want}`");
        assert_eq!(
            offset_to_line_col(src, span.start),
            lc,
            "span start line:col for `{want}`"
        );
        // Tightness: the span must not include a trailing space/newline
        // (the classic `pad(p).map_with_span` bug). `want` itself has
        // no surrounding whitespace, and the equality above already
        // pins that, but assert the boundary char explicitly too.
        assert!(
            !src[span.clone()].starts_with(char::is_whitespace)
                && !src[span.clone()].ends_with(char::is_whitespace),
            "span for `{want}` must be tight (no leading/trailing layout)"
        );
    };

    // --- Directive 0: the whole `worker_class` decl (SpDirective) ---
    let d0 = &ast.directives[0];
    check(&d0.span, "worker_class cc { simd = none; };", (2, 1));
    let wc: &WorkerClassDecl = match &d0.node {
        Directive::WorkerClass(c) => c,
        other => panic!("expected worker_class; got {other:?}"),
    };
    // The class *name* identifier carries its own tight span: just
    // `cc`, line 2 col 14 (`worker_class ` is 13 chars).
    check(&wc.name.span, "cc", (2, 14));

    // --- Directive 1: memory_region; its accessible_by names ---
    let d1 = &ast.directives[1];
    check(
        &d1.span,
        "memory_region rgn { accessible_by = { cc, w0 }; };",
        (3, 1),
    );
    let region = match &d1.node {
        Directive::MemoryRegion(r) => r,
        other => panic!("expected memory_region; got {other:?}"),
    };
    check(&region.name.span, "rgn", (3, 15));
    let acc = region
        .accessible_by
        .as_ref()
        .expect("accessible_by present");
    // `accessible_by = { cc, w0 }` — `cc` and `w0` each tightly
    // spanned at their own columns on line 3.
    check(&acc[0].span, "cc", (3, 39));
    check(&acc[1].span, "w0", (3, 43));

    // --- Directive 3: place k on w0 ---
    let d3 = &ast.directives[3];
    check(&d3.span, "place k on w0;", (5, 1));
    let place: &PlaceDirective = match &d3.node {
        Directive::Place(p) => p,
        other => panic!("expected place; got {other:?}"),
    };
    check(&place.kernel.span, "k", (5, 7));
    match &place.target {
        PlaceTarget::One(w) => check(&w.span, "w0", (5, 12)),
        other => panic!("expected One target; got {other:?}"),
    }

    // --- Directive 4: place_data d in rgn ---
    let d4 = &ast.directives[4];
    check(&d4.span, "place_data d in rgn;", (6, 1));
    let pd: &PlaceDataDirective = match &d4.node {
        Directive::PlaceData(pd) => pd,
        other => panic!("expected place_data; got {other:?}"),
    };
    check(&pd.data.span, "d", (6, 12));
    check(&pd.region.span, "rgn", (6, 17));

    // --- Directive 5: loop i : block=8 — directive + var span ---
    let d5 = &ast.directives[5];
    check(&d5.span, "loop i : block=8;", (7, 1));
    let lp = match &d5.node {
        Directive::Loop(l) => l,
        other => panic!("expected loop; got {other:?}"),
    };
    check(&lp.var.span, "i", (7, 6));

    // Directive 2 is the `workers` decl; pin its entry-name span too
    // (the typed form `w0 : cc`).
    let d2 = &ast.directives[2];
    check(&d2.span, "workers = { w0 : cc };", (4, 1));
    let workers = match &d2.node {
        Directive::Workers(w) => w,
        other => panic!("expected workers; got {other:?}"),
    };
    check(&workers.entries[0].name.span, "w0", (4, 13));
    check(
        &workers.entries[0]
            .class
            .as_ref()
            .expect("typed worker has class")
            .span,
        "cc",
        (4, 18),
    );
}
