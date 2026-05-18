---
id: TASK-0163
title: >-
  e2e harness: unknown schedule in [[required]] silently ignored, not a CI
  failure
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 22:15'
updated_date: '2026-05-18 22:34'
labels:
  - infra
  - tooling
  - e2e
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Found during TASK-0057 CI work. Adding a [[required]] entry to nuc-nucleus/e2e-matrix.toml whose schedule does not match a discovered *.sched.nuc file does NOT fail the e2e run — the harness only walks discovered schedule files, so a typo'd or stale required cell vanishes silently instead of FAILing. This is a CI blind spot: a required cell can be lost without anyone noticing. Harness should error if a [[required]] (example,schedule,backend) triple is not discovered/executed. Forward-carried from TASK-0057.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Harness exits non-zero if any [[required]] matrix entry is not matched to an executed cell
- [x] #2 Error message names the unmatched required triple
- [x] #3 e2e-matrix.toml typo of a required schedule is caught by just e2e
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a coverage check after plan_cells(): for every [[required]] triple that passes the active CLI filters, assert it is either present in the planned set OR present in [[skip]]. Unmatched (not planned AND not skipped) -> hard non-zero exit naming the exact (example, schedule, backend) triple.
2. Implement as a pure function required_coverage_gaps(manifest, planned, args) -> Vec<Cell> so it is unit-testable without cargo/filesystem. Wire it into run() before execution; error path returns Err naming all gaps.
3. Respect [[skip]] exemption: a required triple also in [[skip]] is SATISFIED. Respect CLI filters so narrowed runs (--example/--schedule/--backend) do not falsely fail.
4. Regression tests: (a) typo'd required schedule -> gap reported with triple named; (b) required also in skip -> no gap; (c) current real manifest -> zero gaps (pins 8/0/2 unchanged); (d) CLI filter narrows coverage scope.
5. Gate: nix develop -c just test / e2e / determinism-check / determinism-check-negative / clippy. Commit (no push).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Raised to HIGH (mped-architect review of TASK-0057, P2): the entire project + CI gate trusts the e2e harness `required-fail: 0` line. If a [[required]] cell with a typo/stale schedule silently vanishes instead of failing, a required cell can be deleted by a one-char typo with GREEN CI — a false-negative in the falsifier itself, the exact class determinism-check-negative exists to prevent but unguarded for the required matrix. Foundational, not deferrable. Should be treated as gating trust in TASK-0057 / TASK-0167. Forward-carried into TASK-0167 (genuine milestone matrix must not reintroduce this).

Implemented required-matrix coverage guard in nucleus/e2e/src/main.rs.

- New pure fn required_coverage_gaps(manifest, planned, args) -> Vec<Cell>: a [[required]] triple is a GAP iff it is neither in the planned set NOR in [[skip]], evaluated only within the active CLI filter scope (cell_matches_filters mirrors plan_cells filters).
- Wired into run() right after plan_cells/is_empty, BEFORE any cell builds, for BOTH run-mode and determinism-mode (both trust the required matrix). Returns Err naming every unaccounted triple as (example=, schedule=, backend=).
- Skip exemption honoured: required-also-in-skip is SATISFIED (carried-context gotcha) -> current 8/0/2 unchanged, the two informational distributed skips are not required so unaffected.
- 5 regression tests added (e2e crate unittests 13 -> 18, all green): typo->gap+triple-named, required-in-skip->no-gap, planned->no-gap, CLI-filter scoping (out-of-scope exempt but in-scope typo still caught), real shipped manifest->zero gaps (durable pin).

Gate (nix develop -c): clippy --workspace -D warnings clean; just test 0 failed (incl 5 new); just e2e total 10/pass 8/fail 0/skipped 2/required-fail 0 (UNCHANGED); just determinism-check 8/0 byte-identical; just determinism-check-negative correctly bites.

Proof it bites: transiently typo'd first required schedule naive->naiv in real manifest; just e2e exited 1 with error naming (example=01-elementwise-add, schedule=naiv, backend=pthreads-sync); manifest reverted, git diff/status clean. Durable guard is the unit test, not the manifest mutation.

ORCHESTRATOR REVIEW GATE (phase3-ralph): qa-test-runner GO + mped-architect GO, both read-only, run by orchestrator. Numbers RE-RUN by reviewers this cycle (not transcribed from implementer): just test all green (e2e crate unit 18/0/0, +5 incl. all 5 named coverage tests); just e2e total 10/pass 8/fail 0/skip 2/required-fail 0 (UNCHANGED — no happy-path regression); just determinism-check 8/0 byte-identical; determinism-check-negative bites 2/2 (non-flaky); clippy --workspace -D warnings clean. Bite reproduced END TO END by QA: naive->naiv typo in real e2e-matrix.toml -> just e2e exit 1 naming (example=01-elementwise-add, schedule=naiv, backend=pthreads-sync) before any build; file restored bit-identical (sha256 match, git clean). Architect confirmed skip-exemption correct, CLI-filter scoping does NOT re-hide the bug (bare just e2e/CI uses unfiltered Args::default), determinism-mode coverage is desirable not over-reach, tests pin the bite. One follow-up filed: TASK-0168 (standing wired-path negative gate). No AI credit in 5195ea9. TASK-0163 Done is honest: all 3 ACs genuinely met + independently verified + both reviews GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed a false-negative in the e2e falsifier: a [[required]] matrix cell whose schedule did not match any discovered *.sched.nuc file was never planned and never FAILed, so a one-char manifest typo (or a stale entry after a schedule rename) silently deleted a gating cell while just e2e / CI stayed green.

Changes (nucleus/e2e/src/main.rs):
- required_coverage_gaps(manifest, planned, args) -> Vec<Cell>: pure, unit-testable. A [[required]] triple is a coverage GAP iff it is neither in the planned set NOR declared [[skip]], evaluated only within the active CLI filter scope.
- cell_matches_filters(): mirrors plan_cells' per-axis --example/--schedule/--backend filters so narrowed runs are not falsely failed for out-of-scope required cells.
- Wired into run() immediately after plan_cells (before any cell builds), for BOTH run-mode and determinism-mode. On any gap: non-zero exit with an error naming every unaccounted (example=, schedule=, backend=) triple plus remediation guidance.

Skip exemption honoured (carried-context gotcha): a required triple also in [[skip]] is SATISFIED, so the two informational distributed skips and the 8/0/2 outcome are unchanged. The fix only ADDS a new failure path.

Tests: 5 regression tests (e2e crate unittests 13 -> 18, all green): typo->gap+triple-named, required-in-skip->exempt, planned->ok, CLI-filter scoping (out-of-scope exempt, in-scope typo still caught), and real shipped manifest->zero gaps (durable pin, the spirit of determinism-check-negative).

Gate (nix develop -c, all green): clippy --workspace -D warnings clean; just test 0 failed; just e2e total 10/pass 8/fail 0/skipped 2/required-fail 0 (UNCHANGED); just determinism-check 8/0 byte-identical; just determinism-check-negative correctly bites. End-to-end bite proven by a transient manifest typo (just e2e exited 1 naming the triple) then reverted clean; durable guard is the unit test, not the manifest mutation.

Forward-carried to TASK-0167 (milestone narrowing must extend cell_matches_filters in lockstep) and TASK-0057 (CI required-fail line now trustworthy).

Limitations: the guard validates a required schedule names a discoverable file; it does not validate the file is well-formed (that surfaces as a normal compile-phase FAIL when the cell runs). No remote; committed only (5195ea9).
<!-- SECTION:FINAL_SUMMARY:END -->
