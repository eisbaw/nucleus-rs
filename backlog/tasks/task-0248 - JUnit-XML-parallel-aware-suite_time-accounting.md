---
id: TASK-0248
title: 'JUnit XML: parallel-aware suite_time accounting'
status: Done
assignee: []
created_date: '2026-05-22 19:50'
updated_date: '2026-05-22 20:54'
labels:
  - tooling
  - e2e
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 53 (TASK-0023.02) added --format=junit. The suite-level `<testsuite time=...>` attribute sums each cell's elapsed_ms — accurate under sequential default, but OVERSTATES under --jobs N>=2 because parallel cells run concurrently (4 cells of 1s each on --jobs 4 take ~1s wall-clock, but suite_time would report ~4s).

JUnit consumers (Jenkins, Surefire, GitHub Actions) generally tolerate this — `time` is informational, not load-bearing for pass/fail. Schema-legal. But it's misleading for CI dashboards comparing parallel runs.

Fix: track a separate wall-clock total (from execute_cells_parallel start to end) + emit it as the <testsuite time=...>. Per-cell <testcase time=...> stays per-cell elapsed.

Also cosmetic: <failure type="phase" message="phase"> repeats the same string in both attrs. Should make `type` the structural phase tag (Build|Run|Diff) and `message` human-readable, OR drop `message` entirely (it's optional in JUnit).

Acceptance:
- <testsuite time=N> reflects WALL-clock time under --jobs >=2.
- <failure type=phase> and <failure message=...> are distinct (or message removed).
- Sequential default behavior unchanged.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 58 (2026-05-22) — closed. JUnit XML <testsuite time=> now wall-clock-honest under --jobs N; <failure type=phase message=phase> duplicate attribute dropped.

Implementation: nucleus/e2e/src/main.rs only. execute_cells_parallel signature returns (Vec<R>, Duration) — wall_start anchored at entry, elapsed() captured at exit. print_summary_junit + print_determinism_summary_junit take wall_clock: Option<Duration>; Some-branch uses it; None-branch falls back to the cycle-53 per-cell sum (no regression for any future caller without an executor wall-clock).

<failure> element now emits as <failure type="phase"><![CDATA[detail]]></failure> — dropped the redundant message="phase" attr (was a verbatim duplicate of type, cosmetic doc-lie). doc-comment cites TASK-0248 as the source.

Acceptance:
- <testsuite time=N> wall-clock under --jobs >=2: VERIFIED (29.080s sequential vs 13.600s --jobs 4 = 2.14x speedup correctly reflected). Sequential default behavior preserved — wall_start anchored BEFORE the jobs==1 branch so sequential runs are also timed honestly (not skipped).
- <failure> type + message distinct: dropped message= entirely (architect option A — simplest, schema-legal). type=phase + CDATA(detail) carries the load.
- Sequential default behavior unchanged: byte-for-byte tally unchanged 88/70/0/18.

Gate (cycle 58): just e2e 88/70/0/18 UNCHANGED; --format=junit sequential time=29.080; --format=junit --jobs 4 time=13.600; just test 0 FAILED; just clippy clean.

Honest limit (architect LOW): Option<Duration> fallback is paid-for code but currently unused (both in-tree callers pass Some(...)). Justifiable as future-proofing (a synthetic test that calls the emitter on a hand-built Vec<CellResult> without running the executor). Cheap insurance.

Review-gate: both qa-test-runner + mped-architect GO. QA verified the 29s→14s wall-clock delta as the structural observable. Architect: HIGH confidence on wall_start placement, Option fallback, message-attr dedup. 1 LOW (dead None-branch) accepted as future-proofing.
<!-- SECTION:FINAL_SUMMARY:END -->
