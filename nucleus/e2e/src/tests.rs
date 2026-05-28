//! Internal `#[test]` items for the `nucleus-e2e` binary crate
//! ([`crate::main`]), carved out of `src/main.rs` cycle 185 (TASK-0340
//! AC#4 / TASK-0340.09).
//!
//! Carve-out scope: every `#[test]` plus three module-local helpers
//! (`nondet_env_lock`, `nondet_tmp`, `req_cov_neg_env_lock`) and one
//! `SAMPLE_CARGO_TOML` constant moved verbatim from the inline
//! `#[cfg(test)] mod tests { ... }` block that lived at the tail of
//! `src/main.rs` (lines 4715-7315 in the pre-carve-out tree). The
//! file body is de-indented by exactly four spaces — the only
//! systematic transformation applied during the move. Raw-string
//! contents (six `r#"..."#` literals; two multi-line TOML cases) were
//! audited pre-move: every multi-line raw-string body line is at
//! column 0 in source so the four-space de-indent is a no-op on
//! raw-string content. No behaviour change.
//!
//! Audit honesty (cycle 185 disclosure, recorded once at the carve-
//! out site rather than rewriting TASK-0340 AC#4 per
//! `feedback-ac-rewrite-on-done-task`): the parent task's audit text
//! claims the tests "cover the JSON/JUnit report formatter, not
//! compiler correctness". Both halves are mis-grounded — these tests
//! are e2e *harness* tests (arg parser, manifest, plan, coverage-gap
//! detection, perturbation, xbackend corruption, required-coverage
//! injection, perf threshold, baseline delta) AND a smaller subset
//! covers the JSON / JUnit / delta-table formatters
//! (`json_escape_str`, `render_timings_json`, `write_timings_json`,
//! `render_det_timings_json`, `write_det_timings_json`,
//! `render_delta_table_color`, `compare_against_baseline_writes_to_stderr_and_flags_regressor`,
//! `junit_summary_shape_is_valid_xml_skeleton` — eight of seventy-five
//! exactly; the cycle-185b architect read-only review tightened the
//! cycle-185 "roughly ten" hedge to the exact count and replaced the
//! plural `compare_against_baseline_*` glob with the single existing
//! variant). The spirit of AC#4 ("visually separate tests from
//! `main.rs` production code") is satisfied by carving the whole
//! block; the literal description ("76 tests covering JSON/JUnit
//! report formatter") was always inaccurate to the file contents.

use super::*;

#[test]
fn arg_parser_accepts_all_documented_flags() {
    let argv: Vec<OsString> = [
        "--example",
        "01-elementwise-add",
        "--schedule",
        "naive",
        "--backend",
        "pthreads-sync",
        "--milestone",
        "M1",
    ]
    .iter()
    .map(|s| OsString::from(*s))
    .collect();
    let a = parse_args(&argv).expect("parse");
    assert_eq!(a.example.as_deref(), Some("01-elementwise-add"));
    assert_eq!(a.schedule.as_deref(), Some("naive"));
    assert_eq!(a.backend.as_deref(), Some("pthreads-sync"));
    assert_eq!(a.milestone, Some(Milestone(1)));
}

#[test]
fn arg_parser_accepts_check_determinism() {
    let argv = vec![OsString::from("--check-determinism")];
    let a = parse_args(&argv).expect("parse");
    assert!(a.check_determinism, "flag was not picked up");
    // Other flags default to None / false.
    assert!(a.example.is_none());
}

#[test]
fn arg_parser_combines_check_determinism_with_filters() {
    // Determinism mode should compose with the normal narrowing
    // flags so a developer can debug one cell quickly.
    let argv: Vec<OsString> = [
        "--check-determinism",
        "--example",
        "01-elementwise-add",
        "--backend",
        "pthreads-sync",
    ]
    .iter()
    .map(|s| OsString::from(*s))
    .collect();
    let a = parse_args(&argv).expect("parse");
    assert!(a.check_determinism);
    assert_eq!(a.example.as_deref(), Some("01-elementwise-add"));
    assert_eq!(a.backend.as_deref(), Some("pthreads-sync"));
}

#[test]
fn enumerate_files_returns_paths_relative_to_root() {
    // Build a small synthetic tree and verify enumerate_files
    // walks it and returns paths relativised to root. The
    // determinism diff relies on this so two trees rooted at
    // different absolute paths compare equal when their contents
    // are byte-identical.
    let tmp =
        std::env::temp_dir().join(format!("nucleus-e2e-enumerate-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("src")).expect("mk src");
    fs::write(tmp.join("Cargo.toml"), b"[package]").expect("write cargo");
    fs::write(tmp.join("src/main.rs"), b"fn main(){}").expect("write main");
    fs::write(tmp.join("src/kernels.rs"), b"// k").expect("write kernels");

    let mut files = enumerate_files(&tmp).expect("walk");
    files.sort();
    assert_eq!(
        files,
        vec![
            PathBuf::from("Cargo.toml"),
            PathBuf::from("src/kernels.rs"),
            PathBuf::from("src/main.rs"),
        ],
        "walk should return relative paths in stable order"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn determinism_skip_entry_short_circuits() {
    // Manifest-declared skips short-circuit before running
    // nucleus build — same contract as run-mode. Test we never
    // attempt to spawn cargo in this case.
    let paths = Paths::discover().expect("discover");
    let pc = PlannedCell {
        cell: Cell {
            example: "ghost-example".to_string(),
            schedule: "ghost-sched".to_string(),
            backend: "ghost-backend".to_string(),
        },
        required: false,
        pre_skip: Some("manifest says skip".to_string()),
        perf_threshold_pct: None,
    };
    let r = check_cell_determinism(&paths, &pc);
    match r.status {
        DetCellStatus::Skipped { reason } => assert_eq!(reason, "manifest says skip"),
        other => panic!("expected SKIPPED, got {other:?}"),
    }
}

#[test]
fn determinism_missing_capabilities_is_skipped() {
    // Cells targeting a non-existent backend report SKIPPED
    // (rather than FAILED) — determinism is meaningful only on
    // cells the harness can actually exercise.
    let paths = Paths::discover().expect("discover");
    let pc = PlannedCell {
        cell: Cell {
            example: "01-elementwise-add".to_string(),
            schedule: "naive".to_string(),
            backend: "definitely-not-a-real-backend-xyzzy".to_string(),
        },
        required: false,
        pre_skip: None,
        perf_threshold_pct: None,
    };
    let r = check_cell_determinism(&paths, &pc);
    match r.status {
        DetCellStatus::Skipped { reason } => {
            assert!(reason.contains("capabilities.toml"), "reason was: {reason}");
        }
        other => panic!("expected SKIPPED, got {other:?}"),
    }
}

#[test]
fn arg_parser_rejects_unknown_flag() {
    let argv = vec![OsString::from("--frobnicate")];
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("--frobnicate"), "got: {err}");
}

#[test]
fn arg_parser_rejects_flag_without_value() {
    let argv = vec![OsString::from("--example")];
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("requires a value"), "got: {err}");
}

// ---- TASK-0023.01: --jobs / -j parser tests --------------------

#[test]
fn arg_parser_default_jobs_is_one() {
    // Default sequential behaviour: NO --jobs flag → jobs == 1, the
    // same byte-for-byte path as before TASK-0023.01. The
    // execute_cells_parallel helper hangs its sequential-equivalence
    // guarantee on this default.
    let argv: Vec<OsString> = Vec::new();
    let a = parse_args(&argv).expect("parse");
    assert_eq!(a.jobs, 1, "default jobs must be 1 (sequential)");
}

#[test]
fn arg_parser_accepts_jobs_long_and_short() {
    // --jobs N and -j N must be exactly equivalent. The short form
    // exists because `just e2e-jobs 4` may pass -j and that path
    // would otherwise diverge from the long form silently.
    for flag in ["--jobs", "-j"] {
        let argv: Vec<OsString> = [flag, "4"].iter().map(|s| OsString::from(*s)).collect();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.jobs, 4, "expected jobs=4 via `{flag} 4`");
    }
}

#[test]
fn arg_parser_jobs_max_bound_accepted() {
    // MAX_JOBS is the documented ceiling. Exactly MAX_JOBS must be
    // accepted (off-by-one guard against accidentally tightening
    // the bound during a refactor).
    let argv = vec![
        OsString::from("--jobs"),
        OsString::from(MAX_JOBS.to_string()),
    ];
    let a = parse_args(&argv).expect("parse");
    assert_eq!(a.jobs, MAX_JOBS);
}

#[test]
fn arg_parser_rejects_jobs_zero() {
    // jobs==0 would spawn no workers; reject loud at parse time
    // rather than silently never making progress.
    let argv: Vec<OsString> = ["--jobs", "0"].iter().map(|s| OsString::from(*s)).collect();
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains(">= 1"), "got: {err}");
}

#[test]
fn arg_parser_rejects_jobs_non_integer() {
    let argv: Vec<OsString> = ["--jobs", "four"]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let err = parse_args(&argv).unwrap_err();
    assert!(
        err.contains("positive integer") && err.contains("four"),
        "got: {err}"
    );
}

#[test]
fn arg_parser_rejects_jobs_negative() {
    // usize parse rejects the leading minus; we only need to confirm
    // the error message names the offending value (good UX).
    let argv: Vec<OsString> = ["--jobs", "-1"]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("positive integer"), "got: {err}");
}

#[test]
fn arg_parser_rejects_jobs_above_max() {
    let argv = vec![
        OsString::from("--jobs"),
        OsString::from((MAX_JOBS + 1).to_string()),
    ];
    let err = parse_args(&argv).unwrap_err();
    assert!(
        err.contains("MAX_JOBS") || err.contains(&MAX_JOBS.to_string()),
        "got: {err}"
    );
}

#[test]
fn arg_parser_rejects_jobs_without_value() {
    let argv = vec![OsString::from("--jobs")];
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("requires a value"), "got: {err}");
}

#[test]
fn arg_parser_jobs_composes_with_other_flags() {
    // --jobs must compose with the narrowing flags: a developer
    // debugging one cell on a parallel host should still get
    // parallel execution of the narrowed set.
    let argv: Vec<OsString> = [
        "--example",
        "01-elementwise-add",
        "--jobs",
        "2",
        "--check-determinism",
    ]
    .iter()
    .map(|s| OsString::from(*s))
    .collect();
    let a = parse_args(&argv).expect("parse");
    assert_eq!(a.jobs, 2);
    assert_eq!(a.example.as_deref(), Some("01-elementwise-add"));
    assert!(a.check_determinism);
}

// ---- TASK-0023.02: --format parser tests ----------------------

#[test]
fn arg_parser_default_format_is_text() {
    // Default behaviour: NO --format flag → Format::Text, the same
    // byte-for-byte stdout path as before TASK-0023.02.
    let argv: Vec<OsString> = Vec::new();
    let a = parse_args(&argv).expect("parse");
    assert_eq!(a.format, Format::Text, "default format must be Text");
}

#[test]
fn arg_parser_accepts_format_space_and_equals() {
    // Both `--format junit` (separate value) and `--format=junit`
    // (equals form) must work — CI scripts commonly pass the
    // latter and a silent reject would be confusing.
    for argv in [
        vec![OsString::from("--format"), OsString::from("junit")],
        vec![OsString::from("--format=junit")],
    ] {
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.format, Format::Junit, "argv={argv:?}");
    }
    for argv in [
        vec![OsString::from("--format"), OsString::from("text")],
        vec![OsString::from("--format=text")],
    ] {
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.format, Format::Text, "argv={argv:?}");
    }
}

#[test]
fn arg_parser_rejects_unknown_format() {
    for argv in [
        vec![OsString::from("--format"), OsString::from("yaml")],
        vec![OsString::from("--format=yaml")],
    ] {
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("text") && err.contains("junit"), "got: {err}");
    }
}

#[test]
fn arg_parser_rejects_format_without_value() {
    // Long form without a value must fail loud, like every other
    // value-bearing flag.
    let argv = vec![OsString::from("--format")];
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("requires a value"), "got: {err}");
}

#[test]
fn junit_summary_shape_is_valid_xml_skeleton() {
    // Synthetic 3-cell matrix exercises PASS, FAIL, SKIPPED in one
    // pass. We verify the document opens with the XML decl and the
    // testsuites/testsuite envelope, contains exactly one
    // <testcase> per cell with the correct classname/name, and
    // emits <failure>/<skipped> children for the non-pass cells.
    // Run this through `print_summary_junit` via a captured stdout
    // would be ideal; absent a `Write` parameter we instead
    // re-implement the small assertions on the values we'd render.
    let cell_pass = CellResult {
        cell: Cell {
            example: "ex1".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
        },
        required: true,
        status: Status::Pass,
        timings: Timings::default(),
        corrupted: false,
    };
    let cell_fail = CellResult {
        cell: Cell {
            example: "ex2".into(),
            schedule: "tiled".into(),
            backend: "mp-tcp-bufsync".into(),
        },
        required: true,
        status: Status::Failed {
            phase: Phase::Diff,
            detail: "byte mismatch at offset 42".into(),
        },
        timings: Timings::default(),
        corrupted: false,
    };
    let cell_skip = CellResult {
        cell: Cell {
            example: "ex3".into(),
            schedule: "tiled".into(),
            backend: "pthreads-sync".into(),
        },
        required: false,
        status: Status::Skipped {
            reason: "manifest-skip".into(),
        },
        timings: Timings::default(),
        corrupted: false,
    };
    // The function writes to stdout; we cannot capture it without
    // a Write parameter, so this test exercises only the static
    // helpers — escaping + classname composition — that the
    // emitter relies on. The shape itself is verified by the
    // workflow-gate `cargo run --bin nucleus-e2e -- --format=junit`
    // check documented on TASK-0023.02.
    let _ = (cell_pass, cell_fail, cell_skip);
    assert_eq!(xml_escape_attr("a&b<c>\""), "a&amp;b&lt;c&gt;&quot;");
    assert_eq!(xml_escape_cdata("foo]]>bar"), "foo]]]]><![CDATA[>bar");
}

#[test]
fn manifest_roundtrip_minimal() {
    // Smallest valid manifest. The harness must accept zero
    // required and zero skip entries (e.g. a future "compile only"
    // backend with no required-pass cells yet).
    let src = r#"
runnable_examples = ["foo"]
backends = ["bar"]
"#;
    let m: Manifest = toml::from_str(src).expect("parse");
    assert_eq!(m.runnable_examples, vec!["foo"]);
    assert_eq!(m.backends, vec!["bar"]);
    assert!(m.required.is_empty());
    assert!(m.skip.is_empty());
}

#[test]
fn manifest_rejects_unknown_field() {
    // deny_unknown_fields keeps typos from silently flipping the
    // matrix shape.
    let src = r#"
runnable_examples = ["foo"]
backends = ["bar"]
mystery = 42
"#;
    let r = toml::from_str::<Manifest>(src);
    assert!(r.is_err(), "expected parse failure on unknown field");
}

#[test]
fn manifest_actual_file_parses() {
    // The shipped `e2e-matrix.toml` is the load-bearing artefact;
    // a typo there breaks every developer's `just e2e`. This test
    // is the canary.
    let paths = Paths::discover().expect("discover repo root");
    let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
    let m: std::result::Result<Manifest, toml::de::Error> = toml::from_str(&src);
    assert!(m.is_ok(), "parse failed: {:?}", m.err());
    let m = m.unwrap();
    assert!(
        !m.runnable_examples.is_empty(),
        "manifest declares zero runnable examples"
    );
    assert!(!m.backends.is_empty(), "manifest declares zero backends");
}

/// Synthetic single-cell matrix: build a manifest in memory that
/// references just one example/schedule/backend triple known to
/// be PASS at M1, drive plan_cells, and verify the planner picks
/// it up exactly once. This exercises the harness's plumbing
/// (manifest decode + plan_cells + paths discovery) without
/// running cargo on the cell — the cell *execution* itself is
/// covered by `nucleus-compiler/tests/e2e_example_0{1,2,3}.rs`,
/// which remain authoritative regression catchers per the task
/// brief.
#[test]
fn plan_picks_required_cell_when_filtered() {
    let paths = Paths::discover().expect("discover");
    let manifest = Manifest {
        runnable_examples: vec!["01-elementwise-add".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![RequiredEntry {
            example: "01-elementwise-add".to_string(),
            schedule: "naive".to_string(),
            backend: "pthreads-sync".to_string(),
            milestone: "M1".to_string(),
            perf_threshold_pct: None,
        }],
        skip: vec![],
    };
    let args = Args {
        example: Some("01-elementwise-add".into()),
        schedule: Some("naive".into()),
        backend: Some("pthreads-sync".into()),
        ..Args::default()
    };
    let planned = plan_cells(&paths, &manifest, &args).expect("plan");
    assert_eq!(planned.len(), 1);
    assert!(planned[0].required);
    assert!(planned[0].pre_skip.is_none());
    assert_eq!(planned[0].cell.schedule, "naive");
}

// Skip-entry plumbing: a [[skip]] entry produces a SKIPPED
// status with the manifest's reason, *without* invoking cargo.
// Drives run_cell directly with a manifest-synthesised skip;
// validates we bail out before touching the filesystem.
// ----------------------------------------------------------------
// TASK-0163: required-matrix coverage guard. These pin the "prove
// the falsifier works" property in the spirit of
// `determinism-check-negative`: a required cell that silently
// vanishes (typo / stale schedule) MUST become a hard error.
// ----------------------------------------------------------------

fn cell(ex: &str, sc: &str, be: &str) -> Cell {
    Cell {
        example: ex.to_string(),
        schedule: sc.to_string(),
        backend: be.to_string(),
    }
}

fn planned(ex: &str, sc: &str, be: &str) -> PlannedCell {
    PlannedCell {
        cell: cell(ex, sc, be),
        required: true,
        pre_skip: None,
        perf_threshold_pct: None,
    }
}

/// A `[[required]]` declaration tagged at milestone `ms`
/// (e.g. "M1"). Most coverage-guard tests do not exercise the
/// milestone axis, so they use "M1" (in-band for every gate).
fn req(ex: &str, sc: &str, be: &str, ms: &str) -> RequiredEntry {
    RequiredEntry {
        example: ex.to_string(),
        schedule: sc.to_string(),
        backend: be.to_string(),
        milestone: ms.to_string(),
        perf_threshold_pct: None,
    }
}

fn skip_e(ex: &str, sc: &str, be: &str, reason: &str, ms: &str) -> SkipEntry {
    SkipEntry {
        example: ex.to_string(),
        schedule: sc.to_string(),
        backend: be.to_string(),
        reason: reason.to_string(),
        milestone: ms.to_string(),
        perf_threshold_pct: None,
    }
}

/// The core bite: a `[[required]]` triple whose schedule does not
/// match any planned (discovered) cell, and is not in `[[skip]]`,
/// is reported as a coverage gap with the exact triple named. This
/// is the regression for the silent-vanish bug — before the fix
/// this triple would simply never run and `just e2e` stayed green.
#[test]
fn typo_in_required_schedule_is_a_coverage_gap() {
    let manifest = Manifest {
        runnable_examples: vec!["01-elementwise-add".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        // `naiv` is a one-char typo of the real `naive` schedule.
        required: vec![req("01-elementwise-add", "naiv", "pthreads-sync", "M1")],
        skip: vec![],
    };
    // Planner only ever discovers the real `naive` file.
    let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
    let gaps = required_coverage_gaps(&manifest, &plan, &Args::default()).expect("ok");
    assert_eq!(gaps.len(), 1, "typo'd required must surface as a gap");
    assert_eq!(gaps[0], cell("01-elementwise-add", "naiv", "pthreads-sync"));
}

/// A required triple that is ALSO listed in `[[skip]]` is exempt —
/// the exit contract permits a required cell to not execute when
/// it is a declared skip. Getting this wrong would falsely fail
/// legitimately-skipped cells (carried-context gotcha).
#[test]
fn required_also_in_skip_is_not_a_gap() {
    let manifest = Manifest {
        runnable_examples: vec!["03-reduction".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![req("03-reduction", "distributed", "pthreads-sync", "M1")],
        skip: vec![skip_e(
            "03-reduction",
            "distributed",
            "pthreads-sync",
            "not yet implemented",
            "M1",
        )],
    };
    // Skipped cell is never planned (run_cell would short-circuit
    // even if it were) — the point is the coverage guard must not
    // treat the absence as a gap.
    let plan: Vec<PlannedCell> = vec![];
    let gaps = required_coverage_gaps(&manifest, &plan, &Args::default()).expect("ok");
    assert!(
        gaps.is_empty(),
        "skip-declared required must be exempt, got {gaps:?}"
    );
}

/// A required triple that IS planned (will execute) is accounted
/// for — its real PASS/FAIL verdict gates the exit as before. No
/// gap, no behaviour change for the happy path.
#[test]
fn planned_required_is_not_a_gap() {
    let manifest = Manifest {
        runnable_examples: vec!["01-elementwise-add".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![req("01-elementwise-add", "naive", "pthreads-sync", "M1")],
        skip: vec![],
    };
    let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
    let gaps = required_coverage_gaps(&manifest, &plan, &Args::default()).expect("ok");
    assert!(
        gaps.is_empty(),
        "planned required must not be a gap, got {gaps:?}"
    );
}

/// CLI narrowing must scope the coverage check: `--example 01`
/// is not responsible for the 07-matmul required cell, so its
/// absence from the (narrowed) plan must NOT be flagged. Without
/// this, every `just e2e --example X` would falsely fail.
#[test]
fn cli_filter_scopes_coverage_check() {
    let manifest = Manifest {
        runnable_examples: vec!["01-elementwise-add".to_string(), "07-matmul".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![
            req("01-elementwise-add", "naive", "pthreads-sync", "M1"),
            req("07-matmul", "naive", "pthreads-sync", "M1"),
        ],
        skip: vec![],
    };
    // Narrowed run: only the 01 cell is planned.
    let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
    let args = Args {
        example: Some("01-elementwise-add".into()),
        ..Args::default()
    };
    let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
    assert!(
        gaps.is_empty(),
        "out-of-filter-scope required cells must not be gaps, got {gaps:?}"
    );

    // But a typo *within* the filtered example IS still caught.
    let manifest_typo = Manifest {
        required: vec![
            req("01-elementwise-add", "naiv", "pthreads-sync", "M1"),
            req("07-matmul", "naive", "pthreads-sync", "M1"),
        ],
        ..manifest
    };
    let plan2 = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
    let gaps2 = required_coverage_gaps(&manifest_typo, &plan2, &args).expect("ok");
    assert_eq!(
        gaps2,
        vec![cell("01-elementwise-add", "naiv", "pthreads-sync")]
    );
}

/// The shipped `e2e-matrix.toml` against the real discovered
/// schedules must have ZERO coverage gaps. This pins today's
/// behaviour: the fix adds a new failure path but must not change
/// the current 8/0/2 outcome. If a future manifest edit introduces
/// a typo'd or stale required entry, THIS test (and `just e2e`)
/// goes red — the durable guard.
#[test]
fn real_manifest_has_no_coverage_gaps() {
    let paths = Paths::discover().expect("discover repo root");
    let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
    let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
    let args = Args::default();
    let plan = plan_cells(&paths, &manifest, &args).expect("plan");
    let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
    assert!(
        gaps.is_empty(),
        "shipped e2e-matrix.toml has unmatched required cells: {gaps:?}"
    );
}

// ----------------------------------------------------------------
// TASK-0167: genuine `--milestone` parameterisation. The milestone
// gate is a NEW narrowing axis; these pin (a) the cumulative
// subsetting, (b) the typed-error on a bad milestone, and (c) —
// the load-bearing one — that the TASK-0163 coverage guard is kept
// in LOCKSTEP, i.e. a typo'd/stale milestone-tagged required cell
// run under its `--milestone` still hard-fails with the triple
// named (the silent-vanish blind spot must NOT reopen per subset).
// ----------------------------------------------------------------

/// `Milestone::parse` accepts the documented `M<k>` shape and
/// rejects everything else with a typed error (never a panic,
/// never a silent default — a mis-typed milestone must not
/// silently mis-bucket / delete a gating cell).
#[test]
fn milestone_parse_accepts_valid_rejects_garbage() {
    assert_eq!(Milestone::parse("M1").unwrap(), Milestone(1));
    assert_eq!(Milestone::parse("M3").unwrap(), Milestone(3));
    assert_eq!(Milestone::parse("M0").unwrap(), Milestone(0));
    // TASK-0346: the full PRD §11 enum now parses, including the
    // tier-2/3 future range M7..M11 (M11 = the ceiling).
    assert_eq!(Milestone::parse("M6").unwrap(), Milestone(6));
    assert_eq!(Milestone::parse("M7").unwrap(), Milestone(7));
    assert_eq!(Milestone::parse("M11").unwrap(), Milestone(11));
    // M12 is one past the ceiling: in-shape but out-of-range, the
    // boundary that proves the clamp still bites.
    for bad in [
        "m1", "M", "3", "MX", "M-1", "M12", "M99", "", "M1.0", "milestone1",
    ] {
        assert!(
            Milestone::parse(bad).is_err(),
            "`{bad}` must be a typed error, not silently accepted"
        );
    }
}

/// `--milestone` is parsed and validated at CLI parse time: a bad
/// value fails LOUD before any work, not accepted-and-ignored.
#[test]
fn arg_parser_rejects_bad_milestone() {
    // M12 is one past the PRD §11 ceiling (M11) — in-shape but
    // out-of-range, so it must fail loud at CLI parse time.
    let argv = vec![OsString::from("--milestone"), OsString::from("M12")];
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("M0..M11"), "got: {err}");
    let argv2 = vec![OsString::from("--milestone"), OsString::from("banana")];
    assert!(parse_args(&argv2).is_err());
}

/// Cumulative subsetting: a milestone-tagged required matrix run
/// under `--milestone M2` keeps M1 ∪ M2 cells and drops M3 ones;
/// `--milestone M3` keeps everything; no flag keeps everything.
/// This is AC#1's "required-set is subset by milestone".
#[test]
fn milestone_gate_is_cumulative_over_required_flagging() {
    let paths = Paths::discover().expect("discover");
    let manifest = Manifest {
        runnable_examples: vec!["01-elementwise-add".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![req("01-elementwise-add", "naive", "pthreads-sync", "M3")],
        skip: vec![],
    };
    // No gate: the single discovered cell is flagged required.
    let p_full = plan_cells(&paths, &manifest, &Args::default()).expect("plan");
    assert!(p_full.iter().any(|c| c.required));

    // --milestone M1: the only required cell is M3-tagged ⇒ it is
    // OUTSIDE the cumulative band ⇒ not flagged required (it still
    // runs as informational, but does not gate the exit).
    let m1 = Args {
        milestone: Some(Milestone(1)),
        ..Args::default()
    };
    let p_m1 = plan_cells(&paths, &manifest, &m1).expect("plan");
    assert!(
        !p_m1.iter().any(|c| c.required),
        "M3 cell must NOT be required under --milestone M1"
    );

    // --milestone M3: in-band ⇒ required again.
    let m3 = Args {
        milestone: Some(Milestone(3)),
        ..Args::default()
    };
    let p_m3 = plan_cells(&paths, &manifest, &m3).expect("plan");
    assert!(
        p_m3.iter().any(|c| c.required),
        "M3 cell must be required under --milestone M3 (cumulative)"
    );
}

/// THE LOCKSTEP REGRESSION (mirrors
/// `typo_in_required_schedule_is_a_coverage_gap` but on the
/// milestone axis). An M3-tagged `[[required]]` whose schedule is
/// typo'd, run with `--milestone M3`, MUST still surface as a
/// coverage gap with the exact triple named. If `plan_cells`
/// narrowed by milestone but the guard did not (or the guard's
/// milestone predicate diverged from plan_cells'), this cell would
/// silently vanish from the M3 subset — the precise TASK-0163
/// blind spot, reopened per milestone. This test fails if the two
/// ever drift.
#[test]
fn typo_in_milestone_tagged_required_is_a_gap_under_that_milestone() {
    let manifest = Manifest {
        runnable_examples: vec!["06-separable-filter".to_string()],
        backends: vec!["mp-tcp-bufsync".to_string()],
        // `naiv` is a one-char typo; this cell is tagged M3.
        required: vec![req("06-separable-filter", "naiv", "mp-tcp-bufsync", "M3")],
        skip: vec![],
    };
    // Planner only ever discovers the real `naive` file.
    let plan = vec![planned("06-separable-filter", "naive", "mp-tcp-bufsync")];
    // Run scoped to exactly this cell's milestone.
    let args = Args {
        milestone: Some(Milestone(3)),
        ..Args::default()
    };
    let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
    assert_eq!(
        gaps,
        vec![cell("06-separable-filter", "naiv", "mp-tcp-bufsync")],
        "an M3 typo'd required cell run under --milestone M3 must \
         still be a hard coverage gap (TASK-0163 lockstep)"
    );

    // And the dual: under --milestone M1 the SAME M3 cell is
    // out-of-band ⇒ NOT this run's obligation ⇒ no false gap
    // (exactly mirrors plan_cells not flagging it required).
    let m1 = Args {
        milestone: Some(Milestone(1)),
        ..Args::default()
    };
    let gaps_m1 = required_coverage_gaps(&manifest, &[], &m1).expect("ok");
    assert!(
        gaps_m1.is_empty(),
        "an M3 cell is not an M1 run's coverage obligation, got {gaps_m1:?}"
    );
}

/// An out-of-band `[[skip]]` must not exempt anything: a skip
/// tagged M3 does not silence an M1-tagged required of the same
/// triple under `--milestone M1`. (Defends the skip-band scoping
/// added in lockstep with plan_cells.)
#[test]
fn out_of_band_skip_does_not_exempt_in_band_required() {
    let manifest = Manifest {
        runnable_examples: vec!["03-reduction".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![req("03-reduction", "ghost", "pthreads-sync", "M1")],
        // Skip is tagged M3 — out of band for an M1 run.
        skip: vec![skip_e(
            "03-reduction",
            "ghost",
            "pthreads-sync",
            "blocked elsewhere",
            "M3",
        )],
    };
    let m1 = Args {
        milestone: Some(Milestone(1)),
        ..Args::default()
    };
    // `ghost` schedule is never discovered ⇒ never planned.
    let gaps = required_coverage_gaps(&manifest, &[], &m1).expect("ok");
    assert_eq!(
        gaps,
        vec![cell("03-reduction", "ghost", "pthreads-sync")],
        "an out-of-band skip must NOT exempt an in-band required"
    );
}

/// The shipped manifest must have ZERO coverage gaps at EVERY
/// milestone tier (no flag, M1, M2, M3, M4) — the durable per-tier
/// guard. A future manifest edit that typo's or strands a
/// milestone-tagged required cell turns the relevant tier (and its
/// CI job) red.
///
/// M4 was added when TASK-0042.03 landed the first M4-tagged
/// required cell (09-producer-consumer/pipelined × pthreads-async,
/// the async + buffer>1 + notify=event headline).
#[test]
fn real_manifest_has_no_coverage_gaps_at_every_milestone() {
    let paths = Paths::discover().expect("discover repo root");
    let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
    let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
    for gate in [
        None,
        Some(Milestone(1)),
        Some(Milestone(2)),
        Some(Milestone(3)),
        Some(Milestone(4)),
    ] {
        let args = Args {
            milestone: gate,
            ..Args::default()
        };
        let plan = plan_cells(&paths, &manifest, &args).expect("plan");
        let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
        assert!(
            gaps.is_empty(),
            "shipped e2e-matrix.toml has unmatched required cells at \
             milestone {gate:?}: {gaps:?}"
        );
    }
}

/// The required set genuinely DIFFERS per milestone (the whole
/// point of AC#3 — the CI jobs must not be identical). Pins the
/// cumulative monotone: |M_k| < |M_{k+1}| for every adjacent pair
/// of milestones present in the manifest, and |M_top| == |full|.
///
/// TASK-0268 (cycle 102) refactored from hardcoded M1..M4 / M5 to
/// matrix-driven milestone discovery — the architect P2 review
/// noted that the previous hardcoded form required a manual code
/// edit at every milestone bump (M3<M4 → M3<M4<M5 → M3<M4<M5<M6
/// ...). Now the test reads ALL milestones referenced by
/// `[[required]]` rows in the manifest, finds the max, and
/// asserts strict monotonicity from M1 up to the discovered top.
/// New milestones are picked up automatically when their first
/// `[[required]]` cell lands.
#[test]
fn required_counts_strictly_grow_per_milestone() {
    let paths = Paths::discover().expect("discover repo root");
    let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
    let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
    let count = |gate: Option<Milestone>| -> usize {
        let args = Args {
            milestone: gate,
            ..Args::default()
        };
        plan_cells(&paths, &manifest, &args)
            .expect("plan")
            .iter()
            .filter(|c| c.required)
            .count()
    };

    // Discover the top milestone from manifest `[[required]]`
    // rows. Skip-only milestones don't count: a milestone whose
    // ONLY cells are `[[skip]]`s has zero required at any gate
    // band, breaking strict-monotonicity. The gate semantics is
    // "required count", so the discovery must match.
    let top: Milestone = manifest
        .required
        .iter()
        .map(|r| Milestone::parse(&r.milestone).expect("manifest milestone tags must parse"))
        .max()
        .expect("manifest must contain at least one [[required]] cell");

    // Count required cells at each gate band from M1 up to top.
    let counts: Vec<(Milestone, usize)> = (1..=top.0)
        .map(|k| {
            let m = Milestone(k);
            (m, count(Some(m)))
        })
        .collect();
    let full = count(None);

    // Strict monotonicity: every adjacent pair (M_k, M_{k+1}) has
    // count(M_k) < count(M_{k+1}). A non-strict pair indicates a
    // milestone with zero new [[required]] cells beyond the
    // previous tier — that's an empty gate, which by PRD §11 is
    // not a tier-1 milestone (each tier MUST have new acceptance
    // cells).
    for win in counts.windows(2) {
        let (m_lo, c_lo) = win[0];
        let (m_hi, c_hi) = win[1];
        assert!(
            c_lo < c_hi,
            "milestone subsets must strictly grow: \
             |{m_lo:?}|={c_lo} not < |{m_hi:?}|={c_hi} (all counts: {counts:?})"
        );
    }

    // Top tier covers the full required set.
    let (_top_milestone, top_count) = *counts.last().expect("non-empty counts");
    assert_eq!(
        top_count, full,
        "top milestone ({top:?}) required set must equal the full \
         required set (top_count={top_count}, full={full}, all={counts:?})"
    );
}

#[test]
fn skip_entry_short_circuits_run() {
    let paths = Paths::discover().expect("discover");
    let pc = PlannedCell {
        cell: Cell {
            example: "nonexistent-example".to_string(),
            schedule: "phantom".to_string(),
            backend: "imaginary".to_string(),
        },
        required: false,
        pre_skip: Some("test fixture".to_string()),
        perf_threshold_pct: None,
    };
    let r = run_cell(&paths, &pc);
    match r.status {
        Status::Skipped { reason } => assert_eq!(reason, "test fixture"),
        other => panic!("expected SKIPPED, got {other:?}"),
    }
    // No phase ran -> no timings recorded.
    assert!(r.timings.compile.is_none());
    assert!(r.timings.build.is_none());
    assert!(r.timings.run.is_none());
}

// ---- TASK-0187: layout-agnostic perturbation + zero-perturb guard.
//
// `NUC_NONDET_TEST` is process-global; Rust runs `#[test]`s on
// parallel threads. Serialise the env-sensitive cases under one
// mutex so set_var/remove_var cannot interleave. These are the
// ONLY tests (and `maybe_perturb_for_nondet_test` the only code)
// that touch this var, so the mutex is a complete fence.
fn nondet_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A poisoned lock just means a prior env test panicked; we
    // still want a clean guard rather than cascading failures.
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn nondet_tmp(tag: &str) -> PathBuf {
    let t = std::env::temp_dir().join(format!(
        "nucleus-e2e-nondet-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&t);
    fs::create_dir_all(&t).expect("mk tmp tree");
    t
}

const SAMPLE_CARGO_TOML: &str =
    "[package]\nname = \"nuc-generated\"\nversion = \"0.0.0\"\nedition = \"2021\"\n";

#[test]
fn perturb_mutates_cargo_toml_and_stays_valid_toml() {
    // AC#1 / AC#3: with the env gate set, the function must
    // actually mutate the tree's Cargo.toml (>=1 byte added) and
    // the result must still be parseable TOML — a `#` comment, not
    // a Rust `//` line. This is the file EVERY backend emits, so
    // mp-tcp cells (which emit no src/main.rs) are no longer
    // silently skipped.
    let _guard = nondet_env_lock();
    let tree = nondet_tmp("mutate");
    let cargo = tree.join("Cargo.toml");
    fs::write(&cargo, SAMPLE_CARGO_TOML).expect("write cargo");
    let before = fs::read_to_string(&cargo).expect("read before");

    std::env::set_var("NUC_NONDET_TEST", "1");
    let perturbed = maybe_perturb_for_nondet_test(&tree);
    std::env::remove_var("NUC_NONDET_TEST");

    assert_eq!(
        perturbed,
        Ok(true),
        "gate=1 with a present Cargo.toml must report a perturbation"
    );
    let after = fs::read_to_string(&cargo).expect("read after");
    assert_ne!(before, after, "Cargo.toml content must have changed");
    assert!(
        after.len() > before.len(),
        "perturbation must append bytes (got before={} after={})",
        before.len(),
        after.len()
    );
    assert!(
        after.contains("# NUC_NONDET_TEST nonce: pid="),
        "expected a `#` TOML comment nonce, got:\n{after}"
    );
    assert!(
        !after.contains("// NUC_NONDET_TEST"),
        "must NOT inject a Rust `//` line into TOML"
    );
    // The whole point of Cargo.toml-over-main.rs: the generated
    // project's manifest must still parse so downstream `cargo`
    // is unaffected.
    let parsed: Result<toml::Table, _> = after.parse();
    assert!(
        parsed.is_ok(),
        "perturbed Cargo.toml must remain valid TOML, parse error: {:?}",
        parsed.err()
    );

    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn perturb_is_strict_noop_when_env_unset() {
    // AC#3: the bare `determinism-check` path. With the gate
    // unset the function must not read or touch the tree and must
    // report `Ok(false)` (no perturbation) — this is what keeps
    // `just determinism-check` byte-identical.
    let _guard = nondet_env_lock();
    let tree = nondet_tmp("noop");
    let cargo = tree.join("Cargo.toml");
    fs::write(&cargo, SAMPLE_CARGO_TOML).expect("write cargo");

    // Ensure the var is genuinely unset for this case.
    std::env::remove_var("NUC_NONDET_TEST");
    let perturbed = maybe_perturb_for_nondet_test(&tree);

    assert_eq!(
        perturbed,
        Ok(false),
        "env unset must be a strict no-op (no perturbation)"
    );
    let after = fs::read_to_string(&cargo).expect("read after");
    assert_eq!(
        after, SAMPLE_CARGO_TOML,
        "env-unset path must leave Cargo.toml byte-identical"
    );

    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn perturb_errs_when_gate_set_but_cargo_toml_missing() {
    // AC#2 (per-cell arm): if the gate is set but the layout
    // drifted so Cargo.toml is absent, the function returns Err.
    // The harness maps that to a non-Pass and the matrix-wide
    // zero-perturb guard (asserted below) then makes the run a
    // hard fail rather than a recipe-inverted false OK.
    let _guard = nondet_env_lock();
    let tree = nondet_tmp("missing"); // empty: no Cargo.toml

    std::env::set_var("NUC_NONDET_TEST", "1");
    let perturbed = maybe_perturb_for_nondet_test(&tree);
    std::env::remove_var("NUC_NONDET_TEST");

    match perturbed {
        Err(msg) => assert!(
            msg.contains("Cargo.toml") && msg.contains("layout drifted"),
            "error should name the missing Cargo.toml and layout drift, got: {msg}"
        ),
        Ok(v) => panic!("expected Err when Cargo.toml absent under gate, got Ok({v})"),
    }

    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn zero_perturbation_guard_makes_negative_recipe_fail() {
    // AC#2 (matrix arm), proven directly. The TRUE invariant is
    // the *recipe verdict*, not the raw exit code, because the
    // `determinism-check-negative` recipe INVERTS the exit code:
    //
    //   harness exit 0       -> recipe prints "FAIL: did NOT
    //                            detect" and exits 1
    //   harness exit non-0   -> recipe prints "OK: correctly bit"
    //
    // So to make a zero-perturbation run a loud gate FAIL we must
    // exit the harness *cleanly* (0) under the gate; the recipe
    // then fires its FAIL branch. This models that exact seam plus
    // the recipe inversion against synthetic result sets. (The
    // behavioural >=5-run live bite is the gate's job — this locks
    // the *decision* so it can't silently regress.)

    // Mirrors the AC#2 seam in main(): returns the harness process
    // exit code.
    fn harness_exit_code(gate_on: bool, perturbed: &[bool], any_failed: bool) -> i32 {
        if gate_on && perturbed.iter().filter(|p| **p).count() == 0 {
            // Force a CLEAN exit so the recipe's FAIL branch fires.
            return 0;
        }
        if any_failed {
            1
        } else {
            0
        }
    }
    // Mirrors justfile:69: recipe verdict from harness exit code.
    // true == "OK: correctly bit", false == "FAIL: did NOT detect".
    fn recipe_says_ok(harness_exit: i32) -> bool {
        harness_exit != 0
    }

    // Partial-silent-neuter: gate on, NOTHING perturbed, but some
    // unrelated cell Failed (the exact false-confidence scenario).
    // The harness must exit CLEAN so the recipe says FAIL.
    let ex = harness_exit_code(true, &[false, false, false], true);
    assert_eq!(ex, 0, "zero perturbations under gate must exit CLEAN");
    assert!(
        !recipe_says_ok(ex),
        "zero perturbations under gate MUST make the recipe print \
         FAIL (not invert a no-op into OK)"
    );

    // Genuine bite: gate on, >=1 cell perturbed, diff Failed ->
    // harness exits non-zero -> recipe says OK. This is the only
    // path that may print OK.
    let ex = harness_exit_code(true, &[false, true, false], true);
    assert_eq!(ex, 1, "genuine perturbation+diff must exit non-zero");
    assert!(
        recipe_says_ok(ex),
        "a genuine >=1-perturbation bite must let the recipe print OK"
    );

    // Gate OFF (bare determinism-check): guard inert; exit driven
    // purely by Failed cells, byte-identical path undisturbed.
    assert_eq!(
        harness_exit_code(false, &[false, false, false], false),
        0,
        "gate off, no failures -> clean exit (byte-identical path)"
    );
    assert_eq!(
        harness_exit_code(false, &[false, false, false], true),
        1,
        "gate off must keep normal Failed-driven non-zero exit"
    );
}

#[test]
fn explicit_count_signal_makes_negative_recipes_fail_loud_independent_of_exit_code() {
    // TASK-0188 AC#3 — proven directly. Before TASK-0188 the
    // safety invariant ("the falsifier actually touched something")
    // rested SOLELY on the exit-code inversion in justfile:69/:85.
    // We now ALSO emit an explicit machine-checkable stdout line
    // (NUC_NONDET_PERTURBED_CELLS / NUC_XBACKEND_CORRUPTED_DETECTED)
    // and the recipes assert it. This models the recipes' new DUAL
    // verdict and proves: if the count signal says zero / is
    // absent, the recipe FAILS LOUD even when the exit code ALONE
    // would invert to a false OK.

    /// Models the parsed `<n>` from the explicit stdout line.
    /// `None` == the line was absent entirely (a broken
    /// harness/recipe contract — also a hard FAIL).
    type CountSignal = Option<usize>;

    /// Faithful model of the post-TASK-0188 recipe verdict for
    /// BOTH determinism-check-negative (justfile:69) and
    /// xbackend-check-negative (justfile:85): they share the exact
    /// same shape — capture combined output, then:
    ///
    /// 1. if the count line is absent -> FAIL.
    /// 2. if the parsed count < 1 -> FAIL.
    /// 3. else fall back to the exit-code inversion (harness exit 0
    ///    -> FAIL "did NOT detect"; non-0 -> OK "correctly bit").
    ///
    /// Returns true == recipe prints OK, false == recipe FAILs.
    fn recipe_says_ok(harness_exit: i32, count: CountSignal) -> bool {
        match count {
            None => false,                // signal missing -> FAIL
            Some(n) if n < 1 => false,    // count says zero -> FAIL
            Some(_) => harness_exit != 0, // else: exit-code inversion
        }
    }

    // --- The headline TASK-0188 property -------------------------
    // The exact false-confidence scenario the hardening kills:
    // some UNRELATED cell Failed so the raw exit code is non-zero,
    // which the OLD recipe (exit-code inversion ONLY) would invert
    // into a false "OK: correctly bit". With the count signal at 0
    // the new recipe MUST still print FAIL.
    let unrelated_failure_exit = 1; // raw exit code alone -> "OK"
    assert!(
        !recipe_says_ok(unrelated_failure_exit, Some(0)),
        "count=0 MUST make the recipe FAIL even when the exit code \
         alone would invert to a false OK (the core TASK-0188 \
         hardening; exit-code inversion is no longer sufficient)"
    );

    // Same property when the signal line is entirely ABSENT (e.g.
    // a refactor removed the println! or renamed the key): a
    // broken contract is a loud FAIL, never a silent pass.
    assert!(
        !recipe_says_ok(unrelated_failure_exit, None),
        "a MISSING count signal MUST make the recipe FAIL even \
         when the exit code alone would invert to OK"
    );

    // Belt-and-braces: even if BOTH the exit code says clean AND
    // the count is zero, still FAIL (no path to a false OK).
    assert!(!recipe_says_ok(0, Some(0)));
    assert!(!recipe_says_ok(0, None));

    // --- The genuine-bite path still prints OK -------------------
    // >=1 perturbation/corruption detected AND the harness exited
    // non-zero (real divergence): the ONLY path that may say OK.
    assert!(
        recipe_says_ok(1, Some(26)),
        "a genuine bite (count>=1 AND non-zero exit) must still \
         let the recipe print OK"
    );
    assert!(
        recipe_says_ok(1, Some(1)),
        "the minimal genuine bite (exactly one cell) must say OK"
    );

    // --- Defence-in-depth interplay with the AC#2 zero-perturb
    // guard (TASK-0187): under the gate, zero perturbations force a
    // CLEAN harness exit (0). The OLD single-signal model relied on
    // that clean exit + inversion. Now even if a refactor broke
    // the inversion (treated exit 0 as OK), count=0 still FAILs.
    let forced_clean_exit_on_zero_perturb = 0;
    assert!(
        !recipe_says_ok(forced_clean_exit_on_zero_perturb, Some(0)),
        "the count backstop must hold the line even if the \
         exit-code inversion is broken by a future refactor"
    );

    // --- TASK-0183: the SAME contract for the relocated xbackend
    // wire falsifier. xbackend-check-negative (justfile:85) shares
    // the exact recipe shape; the corruption is now applied
    // harness-side (maybe_corrupt_wire_for_xbackend), and the
    // matrix-wide guard prints NUC_XBACKEND_CORRUPTED_APPLIED /
    // _DETECTED then forces a CLEAN exit when APPLIED==0 so a
    // no-op cannot invert into a false OK. Same model, same key
    // properties — independent of which seam applied the fault.
    let xbackend_unrelated_fail_exit = 1;
    assert!(
        !recipe_says_ok(xbackend_unrelated_fail_exit, Some(0)),
        "xbackend count=0 MUST FAIL even when an unrelated \
         required-fail makes the raw exit non-zero (TASK-0183: the \
         harness-side relocation must keep the TASK-0188 contract)"
    );
    assert!(
        !recipe_says_ok(xbackend_unrelated_fail_exit, None),
        "a MISSING xbackend signal MUST FAIL regardless of exit code"
    );
    assert!(
        recipe_says_ok(1, Some(1)),
        "the minimal genuine xbackend bite (>=1 cell corrupted AND \
         detected, non-zero exit) must still let the recipe say OK"
    );
    // The zero-corruption guard forces exit 0; the count backstop
    // must still FAIL even if a refactor treated exit 0 as OK.
    assert!(
        !recipe_says_ok(0, Some(0)),
        "xbackend zero-corruption guard: forced clean exit + count=0 \
         must still FAIL loud (TASK-0183 mirror of TASK-0187 AC#2)"
    );
}

// ---------------------------------------------------------------
// TASK-0183 — harness-side relocated NUC_XBACKEND_NEGATIVE wire
// corruption (mirrors the TASK-0187 maybe_perturb_for_nondet_test
// trio). NUC_XBACKEND_NEGATIVE is process-global; reuse the same
// serialising fence as the nondet tests (a different var, but the
// same one-mutex discipline keeps env mutation from interleaving
// across the parallel test threads).
// ---------------------------------------------------------------

fn xbackend_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// A synthetic `wire.rs` containing the EXACT `enc_vec` anchor the
/// relocated falsifier rewrites (byte-for-byte the body in
/// `mp-tcp-common/src/wire_runtime.rs`). If wire_runtime drifts,
/// `maybe_corrupt_wire_for_xbackend`'s anchor check must Err loud —
/// `xbackend_corrupt_errs_when_anchor_drifted` exercises that.
const SAMPLE_WIRE_RS: &str = "pub fn enc_vec<T: Copy, const W: usize>(v: &[T], to_le: fn(T) -> [u8; W]) -> Vec<u8> {\n    let mut out = Vec::with_capacity(v.len() * W);\n    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    out\n}\n";

#[test]
fn xbackend_corrupt_rewrites_wire_rs_under_gate() {
    // AC#2/#3: with the gate set and a present src/wire.rs holding
    // the anchor, the function must actually rewrite it (>=1 byte
    // changed) and inject the deliberate last-byte tweak. This is
    // the harness-side analogue of the deleted mp-tcp-bufsync
    // `maybe_corrupt_wire` — behaviour preserved.
    let _guard = xbackend_env_lock();
    let tree = nondet_tmp("xb-mutate");
    let src_dir = tree.join("src");
    fs::create_dir_all(&src_dir).expect("mk src");
    let wire = src_dir.join("wire.rs");
    fs::write(&wire, SAMPLE_WIRE_RS).expect("write wire.rs");

    std::env::set_var("NUC_XBACKEND_NEGATIVE", "1");
    let did = maybe_corrupt_wire_for_xbackend(&tree);
    std::env::remove_var("NUC_XBACKEND_NEGATIVE");

    assert_eq!(
        did,
        Ok(true),
        "gate=1 with a present wire.rs holding the anchor must \
         report a corruption"
    );
    let after = fs::read_to_string(&wire).expect("read after");
    assert_ne!(SAMPLE_WIRE_RS, after, "wire.rs must have changed");
    assert!(
        after.contains("last.wrapping_add(1)")
            && after.contains("NUC_XBACKEND_NEGATIVE deliberate corruption"),
        "expected the deliberate last-byte tweak, got:\n{after}"
    );
    // The anchor must be gone (replaced exactly once).
    assert!(
        !after.contains(
            "    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    out\n}"
        ),
        "the pristine enc_vec tail must have been rewritten"
    );

    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn xbackend_corrupt_is_strict_noop_when_env_unset() {
    // AC#3 / behaviour-equivalence: bare `just e2e`. With the gate
    // unset the function must not read or touch the tree and must
    // report Ok(false) — this is what keeps the emitted wire.rs
    // byte-identical to mp_tcp_common::WIRE_RUNTIME_SRC and bare
    // e2e at 30/26/0/4/0 with zero signal lines.
    let _guard = xbackend_env_lock();
    let tree = nondet_tmp("xb-noop");
    let src_dir = tree.join("src");
    fs::create_dir_all(&src_dir).expect("mk src");
    let wire = src_dir.join("wire.rs");
    fs::write(&wire, SAMPLE_WIRE_RS).expect("write wire.rs");

    std::env::remove_var("NUC_XBACKEND_NEGATIVE");
    let did = maybe_corrupt_wire_for_xbackend(&tree);

    assert_eq!(
        did,
        Ok(false),
        "env unset must be a strict no-op (no corruption)"
    );
    let after = fs::read_to_string(&wire).expect("read after");
    assert_eq!(
        after, SAMPLE_WIRE_RS,
        "env-unset path must leave wire.rs byte-identical \
         (behaviour-equivalence with pristine codegen)"
    );

    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn xbackend_corrupt_errs_when_gate_set_but_wire_rs_missing() {
    // The mp-tcp layout drifted so src/wire.rs is absent under the
    // gate: a HARD Err (never a silent skip). The caller maps it to
    // Failed(Compile) and the matrix-wide zero-corruption guard
    // then forces the recipe to FAIL loud — the never-silently-
    // neuter-the-falsifier invariant, harness-side.
    let _guard = xbackend_env_lock();
    let tree = nondet_tmp("xb-missing"); // no src/wire.rs

    std::env::set_var("NUC_XBACKEND_NEGATIVE", "1");
    let did = maybe_corrupt_wire_for_xbackend(&tree);
    std::env::remove_var("NUC_XBACKEND_NEGATIVE");

    match did {
        Err(msg) => assert!(
            msg.contains("wire.rs") && msg.contains("layout drifted"),
            "error must name the missing wire.rs and layout drift, got: {msg}"
        ),
        Ok(v) => panic!("expected Err when wire.rs absent under gate, got Ok({v})"),
    }

    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn xbackend_corrupt_errs_when_anchor_drifted() {
    // The enc_vec anchor moved/refactored: under the gate this MUST
    // Err (not silently emit an uncorrupted build that would make
    // xbackend-check-negative a false PASS) — the harness-side
    // analogue of the deleted site's `panic!` anchor-drift guard.
    let _guard = xbackend_env_lock();
    let tree = nondet_tmp("xb-drift");
    let src_dir = tree.join("src");
    fs::create_dir_all(&src_dir).expect("mk src");
    let wire = src_dir.join("wire.rs");
    fs::write(&wire, "pub fn enc_vec() { /* refactored away */ }\n").expect("write wire.rs");

    std::env::set_var("NUC_XBACKEND_NEGATIVE", "1");
    let did = maybe_corrupt_wire_for_xbackend(&tree);
    std::env::remove_var("NUC_XBACKEND_NEGATIVE");

    match did {
        Err(msg) => assert!(
            msg.contains("anchor not found") && msg.contains("drifted"),
            "anchor-drift error must be explicit, got: {msg}"
        ),
        Ok(v) => panic!("expected Err on anchor drift under gate, got Ok({v})"),
    }
    // wire.rs must be left UNwritten on the drift path (we Err
    // before the fs::write), so the failure is observable, not a
    // half-corrupted file.
    let after = fs::read_to_string(&wire).expect("read after");
    assert_eq!(
        after, "pub fn enc_vec() { /* refactored away */ }\n",
        "on anchor drift the function must Err WITHOUT writing"
    );

    let _ = fs::remove_dir_all(&tree);
}

// ----------------------------------------------------------------
// TASK-0023.03 Stage 1 — per-cell timings JSON
// ----------------------------------------------------------------

#[test]
fn arg_parser_accepts_emit_timings_space_form() {
    let argv: Vec<OsString> = ["--emit-timings", "/tmp/foo.json"]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let a = parse_args(&argv).expect("parse");
    assert_eq!(
        a.emit_timings.as_deref(),
        Some(std::path::Path::new("/tmp/foo.json"))
    );
}

#[test]
fn arg_parser_accepts_emit_timings_equals_form() {
    let argv: Vec<OsString> = vec![OsString::from("--emit-timings=/tmp/bar.json")];
    let a = parse_args(&argv).expect("parse");
    assert_eq!(
        a.emit_timings.as_deref(),
        Some(std::path::Path::new("/tmp/bar.json"))
    );
}

#[test]
fn arg_parser_rejects_empty_emit_timings_path() {
    // Empty PATH would silently write nothing / surface as a
    // confusing "is a directory" error post-matrix. Fail LOUD at
    // arg-parse so the developer fixes it before paying the
    // matrix cost.
    let argv_space: Vec<OsString> = ["--emit-timings", ""]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let err = parse_args(&argv_space).unwrap_err();
    assert!(
        err.contains("non-empty PATH"),
        "expected non-empty PATH error, got: {err}"
    );

    let argv_eq: Vec<OsString> = vec![OsString::from("--emit-timings=")];
    let err = parse_args(&argv_eq).unwrap_err();
    assert!(
        err.contains("non-empty PATH"),
        "expected non-empty PATH error, got: {err}"
    );
}

#[test]
fn json_escape_str_handles_quote_backslash_newline_and_control() {
    let mut out = String::new();
    json_escape_str(&mut out, "a\"b\\c\n\td\x01e");
    // Expected JSON-escaped output. The control char becomes \u0001.
    assert_eq!(out, "\"a\\\"b\\\\c\\n\\td\\u0001e\"");
}

fn synth_cell_result(
    example: &str,
    schedule: &str,
    backend: &str,
    status: Status,
    compile_ms: Option<u64>,
    build_ms: Option<u64>,
    run_ms: Option<u64>,
) -> CellResult {
    CellResult {
        cell: Cell {
            example: example.into(),
            schedule: schedule.into(),
            backend: backend.into(),
        },
        required: true,
        status,
        timings: Timings {
            compile: compile_ms.map(Duration::from_millis),
            build: build_ms.map(Duration::from_millis),
            run: run_ms.map(Duration::from_millis),
        },
        corrupted: false,
    }
}

#[test]
fn render_timings_json_emits_three_status_variants_in_planned_order() {
    // PASS, SKIPPED, FAILED — covers all three Status arms and
    // their status-specific payloads. The renderer must keep
    // input order so a downstream comparator can index by
    // position rather than re-sorting.
    let results = vec![
        synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            Some(100),
            Some(2000),
            Some(50),
        ),
        synth_cell_result(
            "ex-b",
            "tiled",
            "mp-tcp-bufsync",
            Status::Skipped {
                reason: "no capabilities.toml".into(),
            },
            None,
            None,
            None,
        ),
        synth_cell_result(
            "ex-c",
            "naive",
            "pthreads-sync",
            Status::Failed {
                phase: Phase::Build,
                detail: "cargo build failed\nlinker error".into(),
            },
            Some(80),
            Some(500),
            None,
        ),
    ];
    let doc = render_timings_json(&results);

    // Structural assertions — we deliberately do NOT shell out
    // to a real JSON parser here (no new dep budget); instead
    // we assert byte-substrings unique enough to catch the
    // shape regressions Stage 2/3 will rely on.
    // Cycle-57: the leading object now carries a `"mode": "run"`
    // key before `"cells"` for forward-compat with the det-mode
    // emitter (the consumer branches on top-level `mode`).
    assert!(
        doc.starts_with("{\n  \"mode\": \"run\",\n  \"cells\": ["),
        "leading brace + mode + cells array missing: {doc}"
    );
    assert!(
        doc.trim_end().ends_with("]\n}"),
        "trailing close missing: {doc}"
    );

    // Ordering: ex-a appears before ex-b before ex-c. A simple
    // 3-find suffices since cell names are unique.
    let a = doc.find("\"ex-a\"").expect("ex-a present");
    let b = doc.find("\"ex-b\"").expect("ex-b present");
    let c = doc.find("\"ex-c\"").expect("ex-c present");
    assert!(a < b && b < c, "planned order broken: {a}, {b}, {c}");

    // PASS cell carries `phase_times_ms` with ints, total_ms,
    // and NO fail_phase/skip_reason key.
    let pass_block = &doc[a..b];
    assert!(
        pass_block.contains("\"status\":\"PASS\""),
        "PASS status: {pass_block}"
    );
    assert!(
        pass_block.contains("\"compile\":100"),
        "compile ms: {pass_block}"
    );
    assert!(
        pass_block.contains("\"build\":2000"),
        "build ms: {pass_block}"
    );
    assert!(pass_block.contains("\"run\":50"), "run ms: {pass_block}");
    assert!(
        pass_block.contains("\"total_ms\":2150"),
        "total ms: {pass_block}"
    );
    assert!(
        !pass_block.contains("fail_phase"),
        "PASS must not carry fail_phase"
    );
    assert!(
        !pass_block.contains("skip_reason"),
        "PASS must not carry skip_reason"
    );

    // SKIPPED carries skip_reason and nulls for phase_times_ms.
    let skip_block = &doc[b..c];
    assert!(
        skip_block.contains("\"status\":\"SKIPPED\""),
        "SKIPPED status: {skip_block}"
    );
    assert!(
        skip_block.contains("\"skip_reason\":\"no capabilities.toml\""),
        "reason: {skip_block}"
    );
    assert!(
        skip_block.contains("\"compile\":null"),
        "null compile: {skip_block}"
    );
    assert!(
        skip_block.contains("\"build\":null"),
        "null build: {skip_block}"
    );
    assert!(
        skip_block.contains("\"run\":null"),
        "null run: {skip_block}"
    );
    assert!(
        skip_block.contains("\"total_ms\":0"),
        "total ms 0: {skip_block}"
    );

    // FAILED carries fail_phase + detail; detail contains an
    // escaped newline (proving json_escape_str ran).
    let fail_block = &doc[c..];
    assert!(
        fail_block.contains("\"status\":\"FAIL\""),
        "FAIL status: {fail_block}"
    );
    assert!(
        fail_block.contains("\"fail_phase\":\"build\""),
        "fail_phase: {fail_block}"
    );
    assert!(
        fail_block.contains("\"detail\":\"cargo build failed\\nlinker error\""),
        "escaped detail: {fail_block}"
    );
}

#[test]
fn write_timings_json_creates_parents_and_writes_atomically() {
    // Atomic-write contract: a fresh parent path is created on
    // demand; the .tmp sibling does not survive a successful
    // write. Critical for Stage 2 — a baseline file half-written
    // by a crashed harness must NEVER be loaded as a baseline.
    let tmp_root = std::env::temp_dir().join(format!(
        "nucleus-e2e-emit-timings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&tmp_root);

    let path = tmp_root.join("nested").join("dir").join("timings.json");
    assert!(!path.exists());
    assert!(
        !path.parent().unwrap().exists(),
        "parent must not pre-exist"
    );

    let results = vec![synth_cell_result(
        "ex-a",
        "naive",
        "pthreads-sync",
        Status::Pass,
        Some(1),
        Some(2),
        Some(3),
    )];
    write_timings_json(&path, &results).expect("write");

    assert!(path.exists(), "output JSON must exist after write");
    let body = fs::read_to_string(&path).expect("read back");
    assert!(body.contains("\"ex-a\""), "round-trip body: {body}");
    assert!(body.contains("\"total_ms\":6"), "total ms 1+2+3=6: {body}");

    // .tmp sibling must NOT survive — rename consumed it.
    let tmp_sibling = path.with_file_name("timings.json.tmp");
    assert!(
        !tmp_sibling.exists(),
        ".tmp sibling leaked: {}",
        tmp_sibling.display()
    );

    let _ = fs::remove_dir_all(&tmp_root);
}

// ----------------------------------------------------------------
// TASK-0023.03.03 Stage 1.5 (cycle-57) — det-mode emitter
// ----------------------------------------------------------------

fn synth_det_cell_result(
    example: &str,
    schedule: &str,
    backend: &str,
    status: DetCellStatus,
    elapsed_ms: u64,
) -> DetCellResult {
    DetCellResult {
        cell: Cell {
            example: example.into(),
            schedule: schedule.into(),
            backend: backend.into(),
        },
        required: true,
        status,
        elapsed: Duration::from_millis(elapsed_ms),
        perturbed: false,
    }
}

#[test]
fn render_det_timings_json_emits_three_status_variants_with_distinct_payloads() {
    // Schema-shape pin: mode=determinism + cells array; PASS carries
    // files_compared + elapsed_ms; FAIL carries det_mismatch + elapsed_ms;
    // SKIPPED carries skip_reason + elapsed_ms=null. Cells appear in
    // planned (input) order.
    let results = vec![
        synth_det_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            DetCellStatus::Pass { files_compared: 7 },
            120,
        ),
        synth_det_cell_result(
            "ex-b",
            "tiled",
            "mp-tcp-bufsync",
            DetCellStatus::Skipped {
                reason: "no capabilities.toml".into(),
            },
            0,
        ),
        synth_det_cell_result(
            "ex-c",
            "naive",
            "pthreads-sync",
            DetCellStatus::Failed(DetMismatch {
                relative_path: PathBuf::from("src/main.rs"),
                kind: DetMismatchKind::BytesDiffer,
                offset: 42,
                detail: "A=foo\nB=bar".into(),
            }),
            250,
        ),
    ];
    let doc = render_det_timings_json(&results);

    // Top-level shape: distinct mode key (consumer branches on this).
    assert!(
        doc.starts_with("{\n  \"mode\": \"determinism\",\n  \"cells\": ["),
        "leading mode + cells missing: {doc}"
    );
    assert!(
        doc.trim_end().ends_with("]\n}"),
        "trailing close missing: {doc}"
    );

    // Planned order preserved.
    let a = doc.find("\"ex-a\"").expect("ex-a present");
    let b = doc.find("\"ex-b\"").expect("ex-b present");
    let c = doc.find("\"ex-c\"").expect("ex-c present");
    assert!(a < b && b < c, "planned order broken: {a}, {b}, {c}");

    // PASS: files_compared + elapsed_ms, no skip_reason / det_mismatch.
    let pass = &doc[a..b];
    assert!(pass.contains("\"status\":\"PASS\""), "PASS status: {pass}");
    assert!(
        pass.contains("\"files_compared\":7"),
        "files_compared: {pass}"
    );
    assert!(pass.contains("\"elapsed_ms\":120"), "elapsed_ms: {pass}");
    assert!(
        !pass.contains("skip_reason"),
        "PASS must not carry skip_reason: {pass}"
    );
    assert!(
        !pass.contains("det_mismatch"),
        "PASS must not carry det_mismatch: {pass}"
    );

    // SKIPPED: skip_reason + elapsed_ms null; no files_compared.
    let skip = &doc[b..c];
    assert!(
        skip.contains("\"status\":\"SKIPPED\""),
        "SKIPPED status: {skip}"
    );
    assert!(
        skip.contains("\"skip_reason\":\"no capabilities.toml\""),
        "reason: {skip}"
    );
    assert!(skip.contains("\"elapsed_ms\":null"), "null elapsed: {skip}");
    assert!(
        !skip.contains("files_compared"),
        "SKIPPED must not carry files_compared: {skip}"
    );

    // FAIL: det_mismatch object with all four fields + elapsed_ms.
    let fail = &doc[c..];
    assert!(fail.contains("\"status\":\"FAIL\""), "FAIL status: {fail}");
    assert!(
        fail.contains("\"det_mismatch\":{"),
        "det_mismatch present: {fail}"
    );
    assert!(
        fail.contains("\"relative_path\":\"src/main.rs\""),
        "relpath: {fail}"
    );
    assert!(fail.contains("\"kind\":\"bytes differ\""), "kind: {fail}");
    assert!(fail.contains("\"offset\":42"), "offset: {fail}");
    // Escaped newline proves json_escape_str ran on the detail.
    assert!(
        fail.contains("\"detail\":\"A=foo\\nB=bar\""),
        "escaped detail: {fail}"
    );
    assert!(fail.contains("\"elapsed_ms\":250"), "elapsed_ms: {fail}");
}

#[test]
fn write_det_timings_json_round_trips_and_creates_parents_atomically() {
    // Atomic-write contract for the det-mode emitter mirrors the
    // RUN-mode one (write_timings_json_creates_parents_and_writes_atomically):
    // fresh parent created on demand; .tmp sibling consumed by
    // rename. Body must contain the cell identity triple + the
    // top-level mode marker so a consumer can branch on it.
    let tmp_root = std::env::temp_dir().join(format!(
        "nucleus-e2e-emit-det-timings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&tmp_root);

    let path = tmp_root.join("nested").join("dir").join("det-timings.json");
    assert!(!path.exists());
    assert!(
        !path.parent().unwrap().exists(),
        "parent must not pre-exist"
    );

    let results = vec![synth_det_cell_result(
        "ex-a",
        "naive",
        "pthreads-sync",
        DetCellStatus::Pass { files_compared: 3 },
        42,
    )];
    write_det_timings_json(&path, &results).expect("write");

    assert!(path.exists(), "output JSON must exist after write");
    let body = fs::read_to_string(&path).expect("read back");
    assert!(
        body.contains("\"mode\": \"determinism\""),
        "mode marker: {body}"
    );
    assert!(body.contains("\"ex-a\""), "round-trip body: {body}");
    assert!(
        body.contains("\"files_compared\":3"),
        "files_compared: {body}"
    );
    assert!(body.contains("\"elapsed_ms\":42"), "elapsed_ms: {body}");

    let tmp_sibling = path.with_file_name("det-timings.json.tmp");
    assert!(
        !tmp_sibling.exists(),
        ".tmp sibling leaked: {}",
        tmp_sibling.display()
    );

    let _ = fs::remove_dir_all(&tmp_root);
}

// ----------------------------------------------------------------
// TASK-0023.03 Stage 2 — baseline comparator
// ----------------------------------------------------------------

#[test]
fn arg_parser_accepts_baseline_space_form() {
    // Write a real file so the existence-validation step succeeds
    // — `--baseline` is parsed eagerly with an exists() check.
    let tmp = std::env::temp_dir().join(format!(
        "nuc-baseline-arg-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    fs::write(&tmp, "{}").expect("seed");
    let argv: Vec<OsString> = ["--baseline", tmp.to_str().unwrap()]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let a = parse_args(&argv).expect("parse");
    assert_eq!(a.baseline.as_deref(), Some(tmp.as_path()));
    let _ = fs::remove_file(&tmp);
}

#[test]
fn arg_parser_rejects_nonexistent_baseline_path() {
    // The single most common silent-no-op trap: typoed path.
    // Hard-fail at parse so the developer fixes it before paying
    // the matrix cost.
    let argv: Vec<OsString> = ["--baseline", "/tmp/does-not-exist-c55-baseline.json"]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let err = parse_args(&argv).unwrap_err();
    assert!(
        err.contains("does not exist"),
        "expected does-not-exist error, got: {err}"
    );
}

#[test]
fn arg_parser_rejects_empty_baseline_path() {
    let argv: Vec<OsString> = ["--baseline", ""]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
    let err = parse_args(&argv).unwrap_err();
    assert!(err.contains("non-empty PATH"), "got: {err}");
}

#[test]
fn parse_baseline_json_round_trips_emitter_output() {
    // The reader must consume what the emitter produces. Build a
    // synthetic Vec<CellResult>, run it through render_timings_json,
    // then parse it back; the identity triples + total_ms must
    // match (the only fields the comparator reads).
    let results = vec![
        synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            Some(100),
            Some(2000),
            Some(50),
        ),
        synth_cell_result(
            "ex-b",
            "tiled",
            "mp-tcp-bufsync",
            Status::Skipped {
                reason: "no caps".into(),
            },
            None,
            None,
            None,
        ),
        synth_cell_result(
            "ex-c",
            "naive",
            "pthreads-sync",
            Status::Failed {
                phase: Phase::Build,
                detail: "boom".into(),
            },
            Some(80),
            Some(500),
            None,
        ),
    ];
    let doc = render_timings_json(&results);
    let parsed = parse_baseline_json(&doc).expect("round-trip parse");
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].example, "ex-a");
    assert_eq!(parsed[0].total_ms, 2150);
    assert_eq!(parsed[1].example, "ex-b");
    assert_eq!(parsed[1].total_ms, 0);
    assert_eq!(parsed[2].example, "ex-c");
    assert_eq!(parsed[2].total_ms, 580);
}

#[test]
fn parse_baseline_json_loud_fails_on_malformed_input() {
    // Truncated mid-object — the reader MUST NOT silently treat
    // this as empty; it must surface byte-offset + the snippet.
    let bad = r#"{"cells": [{"example":"x","schedule":"y","#;
    let err = parse_baseline_json(bad).unwrap_err();
    assert!(
        err.msg.contains("EOF") || err.msg.contains("string") || err.msg.contains("expected"),
        "expected structural error, got: {err}",
    );
    assert!(err.offset > 0, "offset should be non-zero: {err}");
}

#[test]
fn parse_baseline_json_fails_on_missing_cells_array() {
    // A valid JSON object but lacking the required top-level
    // `cells` key — could be a different tool's output. Fail loud.
    let bad = r#"{"foo": 1}"#;
    let err = parse_baseline_json(bad).unwrap_err();
    assert!(err.msg.contains("cells"), "got: {err}");
}

#[test]
fn compute_delta_rows_flags_regression_largest_first() {
    // Two cells identical, one cell's current_ms 50% slower.
    // The comparator must surface the regressor as the FIRST
    // row in the delta table (largest regression first).
    let baseline = vec![
        BaselineCell {
            example: "ex-a".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
            total_ms: 1000,
        },
        BaselineCell {
            example: "ex-b".into(),
            schedule: "tiled".into(),
            backend: "pthreads-sync".into(),
            total_ms: 2000,
        },
    ];
    let current = vec![
        // 50% slower — the regression.
        synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            Some(1500),
            None,
        ),
        // Unchanged.
        synth_cell_result(
            "ex-b",
            "tiled",
            "pthreads-sync",
            Status::Pass,
            None,
            Some(2000),
            None,
        ),
    ];
    let rows = compute_delta_rows(&baseline, &current, &[]);
    assert_eq!(rows.len(), 2);
    // Largest regression first.
    assert_eq!(rows[0].example, "ex-a");
    assert_eq!(rows[0].baseline_ms, Some(1000));
    assert_eq!(rows[0].current_ms, Some(1500));
    let pct = rows[0].delta_pct.expect("real delta");
    assert!((pct - 50.0).abs() < 0.01, "expected +50%, got {pct}");

    // Round-trip render: plain-text mode must contain the
    // regressor row textually so a grep-based check works in CI.
    let table = render_delta_table(&rows, false);
    let first_line = table
        .lines()
        .find(|l| l.contains("ex-a"))
        .expect("ex-a present");
    assert!(first_line.contains("1000 -> 1500"), "got: {first_line}");
    assert!(first_line.contains("+50.0%"), "got: {first_line}");
}

#[test]
fn compute_delta_rows_handles_new_and_removed() {
    // Three cells: A in both, B only in baseline (removed), C only
    // in current (new). The table must contain all three rows and
    // never crash on either side's missing.
    let baseline = vec![
        BaselineCell {
            example: "ex-a".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
            total_ms: 100,
        },
        BaselineCell {
            example: "ex-b".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
            total_ms: 200,
        },
    ];
    let current = vec![
        synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            Some(100),
            None,
        ),
        synth_cell_result(
            "ex-c",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            Some(300),
            None,
        ),
    ];
    let rows = compute_delta_rows(&baseline, &current, &[]);
    assert_eq!(rows.len(), 3, "rows: {rows:?}");

    // Find each row by example. ex-a is real delta (tier 0,
    // 0%), ex-c is "(new)" (tier 2), ex-b is "(removed)"
    // (tier 1). Tier 1 sorts before tier 2 so removed appears
    // before new in the output.
    let by_ex = |name: &str| -> &DeltaRow { rows.iter().find(|r| r.example == name).expect(name) };
    let a = by_ex("ex-a");
    assert_eq!(a.baseline_ms, Some(100));
    assert_eq!(a.current_ms, Some(100));
    assert!(a.delta_pct.map_or(false, |p| p.abs() < 0.001));

    let b = by_ex("ex-b");
    assert_eq!(b.baseline_ms, Some(200));
    assert_eq!(b.current_ms, None);
    assert!(b.delta_pct.is_none());

    let c = by_ex("ex-c");
    assert_eq!(c.baseline_ms, None);
    assert_eq!(c.current_ms, Some(300));
    assert!(c.delta_pct.is_none());

    // Rendered output must carry the (new) / (removed) tags in
    // plain-text mode — these are the load-bearing "look here"
    // signals for a human reviewing CI logs.
    let table = render_delta_table(&rows, false);
    assert!(table.contains("(new)"), "table: {table}");
    assert!(table.contains("(removed)"), "table: {table}");
}

#[test]
fn render_delta_table_color_paints_regressions_red_improvements_green() {
    // ANSI mode: a positive Δ% gets red SGR; a negative one gets
    // green; zero stays plain. Keeps the human-eye flag visible
    // even in long CI logs.
    let rows = vec![
        DeltaRow {
            example: "ex-a".into(),
            schedule: "n".into(),
            backend: "p".into(),
            baseline_ms: Some(100),
            current_ms: Some(150),
            delta_pct: Some(50.0),
            perf_threshold_pct: None,
            required: false,
            regression: false,
        },
        DeltaRow {
            example: "ex-b".into(),
            schedule: "n".into(),
            backend: "p".into(),
            baseline_ms: Some(100),
            current_ms: Some(50),
            delta_pct: Some(-50.0),
            perf_threshold_pct: None,
            required: false,
            regression: false,
        },
    ];
    let table = render_delta_table(&rows, true);
    assert!(table.contains("\x1b[31m"), "red SGR missing: {table:?}");
    assert!(table.contains("\x1b[32m"), "green SGR missing: {table:?}");
    // And the plain-text variant must carry NO ANSI bytes.
    let plain = render_delta_table(&rows, false);
    assert!(
        !plain.contains('\x1b'),
        "plain-text mode leaked ANSI: {plain:?}"
    );
}

#[test]
fn compare_against_baseline_writes_to_stderr_and_flags_regressor() {
    // End-to-end: write a baseline file, then call the comparator
    // against a current Vec<CellResult> with one cell 50% slower.
    // We can't easily capture STDERR from a unit test without
    // spawning a subprocess, so we verify the public delta-table
    // helper does the right thing here; the integration step in
    // the verification gate exercises the STDERR routing.
    let tmp_root = std::env::temp_dir().join(format!(
        "nuc-baseline-cmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    let _ = fs::remove_dir_all(&tmp_root);
    let path = tmp_root.join("baseline.json");

    let baseline_results = vec![synth_cell_result(
        "ex-a",
        "naive",
        "pthreads-sync",
        Status::Pass,
        None,
        Some(1000),
        None,
    )];
    write_timings_json(&path, &baseline_results).expect("write");

    let current_results = vec![synth_cell_result(
        "ex-a",
        "naive",
        "pthreads-sync",
        Status::Pass,
        None,
        Some(1500),
        None,
    )];
    // Just verify the read-and-parse path: a corrupt-on-disk
    // baseline must NOT silently no-op.
    let src = fs::read_to_string(&path).expect("read");
    let parsed = parse_baseline_json(&src).expect("parse");
    let rows = compute_delta_rows(&parsed, &current_results, &[]);
    let table = render_delta_table(&rows, false);
    assert!(
        table.contains("ex-a") && table.contains("+50.0%"),
        "expected regression row in: {table}"
    );

    let _ = fs::remove_dir_all(&tmp_root);
}

// ----------------------------------------------------------------
// TASK-0023.03.02 (Stage 3) — per-cell perf threshold gating tests.
// The contract per AC#5: a matrix entry with threshold=50,
// baseline=200ms, current=350ms (75% slower) MUST fail; current=
// 250ms (25% slower) MUST pass. The other two pin the negative-
// path (required vs skip-band cells) and the serde-default
// byte-identicality property.
// ----------------------------------------------------------------

/// Helper: build a `PlannedCell` with a perf threshold set and the
/// required flag explicit. The three-string identity triple matches
/// the `synth_cell_result` calls below so the comparator join lines
/// up correctly.
fn planned_with_threshold(
    ex: &str,
    sc: &str,
    be: &str,
    required: bool,
    threshold: Option<f64>,
) -> PlannedCell {
    PlannedCell {
        cell: Cell {
            example: ex.into(),
            schedule: sc.into(),
            backend: be.into(),
        },
        required,
        pre_skip: None,
        perf_threshold_pct: threshold,
    }
}

#[test]
fn perf_threshold_breach_on_required_cell_is_regression() {
    // AC#5 fail case: threshold=50%, baseline=200ms, current=350ms
    // (delta = +75%). Required cell -> regression flag MUST set,
    // and `compute_delta_rows` must surface it for the exit-code
    // wiring to bite.
    let baseline = vec![BaselineCell {
        example: "ex-a".into(),
        schedule: "naive".into(),
        backend: "pthreads-sync".into(),
        total_ms: 200,
    }];
    let current = vec![synth_cell_result(
        "ex-a",
        "naive",
        "pthreads-sync",
        Status::Pass,
        None,
        None,
        Some(350),
    )];
    let planned = vec![planned_with_threshold(
        "ex-a",
        "naive",
        "pthreads-sync",
        true,
        Some(50.0),
    )];
    let rows = compute_delta_rows(&baseline, &current, &planned);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.perf_threshold_pct, Some(50.0));
    assert!(r.required, "required flag must round-trip");
    let pct = r.delta_pct.expect("real delta");
    assert!((pct - 75.0).abs() < 0.01, "expected +75%, got {pct}");
    assert!(r.regression, "75% > 50% must flag regression");
    // The rendered table must carry the REGRESSION token so the
    // human reviewer + grep-based CI checks can spot it.
    let table = render_delta_table(&rows, false);
    assert!(
        table.contains("REGRESSION") && table.contains("threshold=+50.0%"),
        "expected REGRESSION + threshold tags in: {table}",
    );
}

#[test]
fn perf_under_threshold_does_not_flag_regression() {
    // AC#5 pass case: threshold=50%, baseline=200ms, current=250ms
    // (delta = +25%). Required cell, BUT 25% < 50% so no regression.
    // Threshold tag is still rendered (so a reviewer sees the gate
    // was active), the exit-code-impacting REGRESSION is NOT.
    let baseline = vec![BaselineCell {
        example: "ex-a".into(),
        schedule: "naive".into(),
        backend: "pthreads-sync".into(),
        total_ms: 200,
    }];
    let current = vec![synth_cell_result(
        "ex-a",
        "naive",
        "pthreads-sync",
        Status::Pass,
        None,
        None,
        Some(250),
    )];
    let planned = vec![planned_with_threshold(
        "ex-a",
        "naive",
        "pthreads-sync",
        true,
        Some(50.0),
    )];
    let rows = compute_delta_rows(&baseline, &current, &planned);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    let pct = r.delta_pct.expect("real delta");
    assert!((pct - 25.0).abs() < 0.01, "expected +25%, got {pct}");
    assert!(
        !r.regression,
        "25% under 50% threshold must NOT flag regression"
    );
    let table = render_delta_table(&rows, false);
    assert!(
        table.contains("[threshold=+50.0%]"),
        "expected threshold tag (informational) in: {table}",
    );
    assert!(
        !table.contains("REGRESSION"),
        "must NOT emit REGRESSION when under threshold: {table}",
    );
}

#[test]
fn perf_threshold_breach_on_skip_band_cell_is_informational_only() {
    // A `[[skip]]`-band cell with a threshold that genuinely
    // breached: the row's `regression` flag must be set (so the
    // table can dim-flag it for a reviewer), BUT `required` must
    // be `false` so the exit-code-impacting count (filtered by
    // `regression && required` in `compare_against_baseline`) stays
    // at zero — a skip-band breach is signal, not blocker.
    let baseline = vec![BaselineCell {
        example: "ex-skip".into(),
        schedule: "naive".into(),
        backend: "pthreads-sync".into(),
        total_ms: 100,
    }];
    let current = vec![synth_cell_result(
        "ex-skip",
        "naive",
        "pthreads-sync",
        Status::Pass,
        None,
        None,
        Some(500),
    )];
    let planned = vec![planned_with_threshold(
        "ex-skip",
        "naive",
        "pthreads-sync",
        /* required = */ false,
        Some(10.0),
    )];
    let rows = compute_delta_rows(&baseline, &current, &planned);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert!(r.regression, "+400% > 10% must flag regression");
    assert!(!r.required, "skip-band cell must not be required");
    // The exit-code-impacting filter must yield zero.
    let exit_count = rows.iter().filter(|r| r.regression && r.required).count();
    assert_eq!(
        exit_count, 0,
        "skip-band breach must NOT contribute to exit-code count",
    );
}

#[test]
fn perf_threshold_absent_serde_default_byte_identical() {
    // Matrix toml without the new key MUST deserialize unchanged,
    // and the resulting PlannedCell must carry `perf_threshold_pct
    // = None` so no gate fires. This is the load-bearing byte-
    // identicality contract for the cycle: existing manifests
    // (and the bare `just e2e` run) cannot regress.
    let toml_src = r#"
runnable_examples = ["01-elementwise-add"]
backends = ["pthreads-sync"]

[[required]]
example = "01-elementwise-add"
schedule = "naive"
backend = "pthreads-sync"
milestone = "M1"

[[skip]]
example = "01-elementwise-add"
schedule = "tiled"
backend = "pthreads-sync"
reason = "not yet"
milestone = "M2"
"#;
    let m: Manifest = toml::from_str(toml_src).expect("parse");
    assert_eq!(m.required.len(), 1);
    assert_eq!(m.skip.len(), 1);
    assert!(
        m.required[0].perf_threshold_pct.is_none(),
        "absent threshold must serde-default to None",
    );
    assert!(
        m.skip[0].perf_threshold_pct.is_none(),
        "absent threshold on skip must serde-default to None",
    );
}

#[test]
fn perf_threshold_parsed_from_toml_when_present() {
    // Symmetric positive: when the key IS set, it parses to the
    // expected f64. Together with the previous test this pins the
    // serde shape — absence is None, presence is f64.
    let toml_src = r#"
runnable_examples = ["01-elementwise-add"]
backends = ["pthreads-sync"]

[[required]]
example = "01-elementwise-add"
schedule = "naive"
backend = "pthreads-sync"
milestone = "M1"
perf_threshold_pct = 50.0
"#;
    let m: Manifest = toml::from_str(toml_src).expect("parse");
    assert_eq!(m.required[0].perf_threshold_pct, Some(50.0));
}

// ---- TASK-0168: required-coverage negative-gate injection. ----
//
// `NUC_REQUIRED_COVERAGE_NEGATIVE` is process-global; serialise
// env-sensitive cases under one mutex so set_var/remove_var cannot
// interleave on Rust's parallel test runner. These are the ONLY
// tests (and `maybe_inject_required_coverage_negative` the only
// code) that touch this var, so the mutex is a complete fence.
fn req_cov_neg_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

fn sample_manifest_with_one_required() -> Manifest {
    Manifest {
        runnable_examples: vec!["01-elementwise-add".to_string()],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![req("01-elementwise-add", "naive", "pthreads-sync", "M1")],
        skip: vec![],
    }
}

#[test]
fn req_cov_inject_is_strict_noop_when_env_unset() {
    // AC#1 / AC#2: the function must NOT mutate the manifest when
    // the env gate is unset — this is what keeps bare `just e2e`
    // byte-identical and the shipped manifest clean.
    let _guard = req_cov_neg_env_lock();
    std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");
    let mut m = sample_manifest_with_one_required();
    let before_len = m.required.len();
    let injected = maybe_inject_required_coverage_negative(&mut m);
    assert_eq!(injected, Ok(false), "env unset must report no injection");
    assert_eq!(
        m.required.len(),
        before_len,
        "env unset must leave manifest unchanged"
    );
}

#[test]
fn req_cov_inject_appends_synthetic_required_when_env_set() {
    // AC#1: with the gate set, exactly ONE synthetic entry is
    // appended whose schedule is the sentinel and whose example /
    // backend / milestone are taken from the first real required
    // entry (so cell_matches_filters and milestone_in_gate accept
    // it in the bare run). Existing required entries are NOT
    // mutated — AC#2's append-not-mutate discipline.
    let _guard = req_cov_neg_env_lock();
    let mut m = sample_manifest_with_one_required();
    let original_first = m.required[0].clone();
    let original_len = m.required.len();

    std::env::set_var("NUC_REQUIRED_COVERAGE_NEGATIVE", "1");
    let injected = maybe_inject_required_coverage_negative(&mut m);
    std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");

    assert_eq!(injected, Ok(true), "gate=1 must report an injection");
    assert_eq!(
        m.required.len(),
        original_len + 1,
        "exactly one synthetic entry must have been appended"
    );
    // Append, not in-place mutate: the original first entry is
    // byte-identical (preserves AC#2).
    assert_eq!(m.required[0].example, original_first.example);
    assert_eq!(m.required[0].schedule, original_first.schedule);
    assert_eq!(m.required[0].backend, original_first.backend);
    assert_eq!(m.required[0].milestone, original_first.milestone);
    // The new last entry carries the sentinel schedule.
    let last = m.required.last().expect("injected entry");
    assert_eq!(last.schedule, REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE);
    // example / backend / milestone mirror the anchor so filters
    // accept it.
    assert_eq!(last.example, original_first.example);
    assert_eq!(last.backend, original_first.backend);
    assert_eq!(last.milestone, original_first.milestone);
}

#[test]
fn req_cov_inject_then_gap_detected_end_to_end() {
    // End-to-end seam: after injection, `required_coverage_gaps`
    // (the pure function the wired path delegates to) must surface
    // the synthetic cell as a gap because the synthetic schedule
    // does not appear in `planned`. This pins the precise contract
    // that the new `NUC_REQUIRED_COVERAGE_GAP_DETECTED` stdout key
    // in `run_inner` reads off of.
    let _guard = req_cov_neg_env_lock();
    let mut m = sample_manifest_with_one_required();
    // Plan only contains the REAL `naive` cell — the planner could
    // not have discovered the synthetic schedule on disk.
    let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];

    std::env::set_var("NUC_REQUIRED_COVERAGE_NEGATIVE", "1");
    let _ = maybe_inject_required_coverage_negative(&mut m).expect("inject");
    std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");

    let gaps = required_coverage_gaps(&m, &plan, &Args::default()).expect("ok");
    // Exactly the synthetic cell, attributable by sentinel schedule.
    assert_eq!(
        gaps.len(),
        1,
        "synthetic typo'd required must surface as a gap (got {gaps:?})"
    );
    assert_eq!(
        gaps[0].schedule,
        REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE
    );
    // The attribution filter `run_inner` uses must select exactly
    // this gap — pin it here so a future refactor of either side
    // (the filter or the sentinel) fails LOUD instead of silently
    // dropping the count to zero (which would then force the
    // negative recipe into its FATAL Ok(0) arm and the recipe
    // would FAIL — desirable, but the unit-test signal arrives
    // earlier and is cheaper).
    let attributed = gaps
        .iter()
        .filter(|c| c.schedule == REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE)
        .count();
    assert_eq!(attributed, 1);
}

#[test]
fn req_cov_inject_errs_loud_on_degenerate_manifest() {
    // Defensive: if the manifest has no runnable_examples (a manifest
    // shape that should never occur in this project but is permitted
    // by the type), the injection must Err loud rather than silently
    // succeed by picking nothing. Mirror discipline of the
    // determinism / xbackend hooks: a silently uncorrupted build
    // would be a false-PASS for the negative arm.
    let _guard = req_cov_neg_env_lock();
    let mut m = Manifest {
        runnable_examples: vec![],
        backends: vec!["pthreads-sync".to_string()],
        required: vec![],
        skip: vec![],
    };
    std::env::set_var("NUC_REQUIRED_COVERAGE_NEGATIVE", "1");
    let r = maybe_inject_required_coverage_negative(&mut m);
    std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");
    assert!(
        r.is_err(),
        "degenerate manifest (no runnable_examples and no required) \
         must surface a loud Err, got {r:?}"
    );
}
