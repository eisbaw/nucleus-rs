---
id: TASK-0248
title: 'JUnit XML: parallel-aware suite_time accounting'
status: To Do
assignee: []
created_date: '2026-05-22 19:50'
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
