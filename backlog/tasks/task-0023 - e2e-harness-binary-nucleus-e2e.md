---
id: TASK-0023
title: e2e harness binary (nucleus-e2e)
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 03:20'
labels:
  - M1
  - tooling
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The single CLI entry point that runs the differential test matrix. Takes flags for example, schedule, backend; or runs full matrix when invoked bare. Justfile's e2e recipe calls this.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Crate 'e2e' produces a 'nucleus-e2e' binary.
- [ ] #2 Flags: --example <name>, --schedule <name>, --backend <name>, --milestone <id>; bare invocation runs the full matrix.
- [ ] #3 For each triple: compile via nucleus, run, diff against reference.bin, report pass/fail with timing.
- [ ] #4 Exit non-zero on any failure; print a final matrix summary.
- [ ] #5 Test: 'just e2e' runs and produces a green matrix at M1 (examples 1-3, naive only, pthreads-sync only).
- [ ] #6 Implementation notes record design questions (e.g. parallel vs sequential matrix execution; default for v2).
- [ ] #7 Implementation notes record honest limitations (e.g. timing reports only; no perf regressions tracked yet).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions explored

**Matrix-manifest format: single top-level vs per-example sidecars.** Chose a single `nuc-nucleus/e2e-matrix.toml` at the repo root. Two reasons:

1. The matrix is small (one screen at M1) and reviewing it is one-file work. Splitting into N sidecars triples grep-load with no decoupling benefit while the matrix fits a screen.
2. The harness already walks every example to enumerate schedules; the manifest needs to know which examples are runnable at all (05/13/14 still lack input.bin/reference.bin/kernels.rs) and which schedule-backend cells are required for exit-gating. Both decisions co-locate naturally.

The manifest carries three lists: `runnable_examples`, `backends`, `[[required]]` cells (those gating the exit code), and `[[skip]]` cells (those reporting SKIPPED with a manifest-supplied reason). Cells discovered in the cross-product that match neither list run as *informational* — they execute and report, but do not gate exit.

If the matrix outgrows one screen (post-M5/M6), the right split is by example: `examples/NN-name/required_schedules.txt` for the required set, with the cross-cutting `[[skip]]` table staying central. Filed mentally; not a task yet.

**Parallel vs sequential cell execution.** Sequential. `cargo build --release` for the emitted project already saturates available cores on a single cell; running multiple cells in parallel mostly contends for the build cache. At 5 cells x ~600ms each the wall-clock cost is small. As the matrix grows (M3+ adds mp-tcp-bufsync; M6 adds the remaining tier-1 backends), parallelism matters more. Filed as **TASK-0023.01**.

**Should per-example tests in compiler/tests/ be retained or subsumed?** Retained per the task brief. They are cheap and serve as the first regression catcher during development (single `cargo test -p compiler` runs the M1 trio without invoking the harness's manifest discovery). The harness is the *matrix* runner; the per-example tests are the *focused* runners. The two layer cleanly; deleting them would cost ~1 second of CI in exchange for losing the focused-test ergonomic.

**Arg parser: clap or hand-rolled?** Hand-rolled. The driver crate hand-rolls its own args for the same MSRV-friction reason (clap's MSRV creeps faster than nucleus's pin at 1.83). For five flags, the hand roll is shorter than the clap plumbing would be.

**Color handling without a crate.** Inspect `NO_COLOR` and `CARGO_TERM_COLOR` env vars. Defaults ON. No isatty crate; the alternative was pulling in `is-terminal` for one detection site. The user can pipe through `tee` and lose colour — acceptable trade-off; `NO_COLOR=1` is the documented opt-out.

## Honest limitations

- **Sequential execution.** TASK-0023.01.
- **No JUnit/JSON output.** Human-readable table only. CI integration (matrix surface in GitLab pipelines) wants structured output. TASK-0023.02.
- **No perf regression tracking.** Timings are reported but not retained. TASK-0023.03.
- **--milestone flag is parsed but ignored.** The manifest is the milestone gate today. Wired in for forward-compatibility with PRD §11 milestone subsets.
- **Cell-level capability gate is delegated to the driver.** The harness sniffs that `capabilities.toml` exists and parses but does NOT duplicate `check_schedule_compat`. A schedule incompatible with a backend appears as a `FAILED (compile)` row with the driver's error in the detail column — clear enough but not a clean SKIPPED. Acceptable; turning every capability mismatch into a SKIPPED would force the harness to load the SchedIR, which means depending on the compiler crate. The harness deliberately depends only on serde + toml.
- **Color codes embedded in stdout.** When the user pipes `just e2e` through a file or another non-TTY consumer, ANSI codes leak through. Honour `NO_COLOR` is documented.
- **Manifest `backends` list is the registry, not auto-discovered.** Adding a backend requires editing both the workspace Cargo.toml and the manifest. PRD §12.2 cites "three concrete things" to add a backend (crate, capabilities, workspace member); the manifest now makes it four. Trade-off: the harness needs an explicit list to know what to test against; alphabetical `ls nucleus/backends/` would couple the test matrix to an unstable filesystem order.

## AC verification

- [x] #1 Crate 'e2e' produces a 'nucleus-e2e' binary. (nucleus/e2e/Cargo.toml [[bin]])
- [x] #2 Flags: --example NAME, --schedule NAME, --backend NAME, --milestone ID; bare invocation runs full matrix. (parse_args + manual smoke-test with --example 03-reduction --schedule naive returned one cell.)
- [x] #3 For each triple: compile via nucleus, run, diff against reference.bin, report pass/fail with timing. (run_cell drives 4 phases; per-phase timings retained.)
- [x] #4 Exit non-zero on any failure; print a final matrix summary. (Exit 0 on green; required-fail flips to non-zero. Summary table printed by print_summary.)
- [x] #5 'just e2e' runs and produces a green matrix at M1. Output captured: 4 required cells PASS, 1 informational SKIPPED (03/distributed with TASK-0117/0126 reason). Exit 0.
- [x] #6 Implementation notes record design questions. (Above.)
- [x] #7 Implementation notes record honest limitations. (Above.)

## Verification commands

```
nix develop --command just e2e            # full matrix; exit 0
nix develop --command bash -c 'cd nucleus && cargo test -p e2e'  # 8 unit tests, all pass
nix develop --command bash -c 'cd nucleus && cargo clippy --workspace -- -D warnings'  # clean
```

## Follow-ups filed

- TASK-0023.01: parallel cell execution.
- TASK-0023.02: JUnit/JSON structured output.
- TASK-0023.03: perf regression tracking.
<!-- SECTION:NOTES:END -->
