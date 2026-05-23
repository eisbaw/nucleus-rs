---
id: TASK-0076
title: 'CI hook: verify reference.bin freshness on PRs touching *.algo.nuc'
status: Done
assignee: []
created_date: '2026-05-17 23:39'
updated_date: '2026-05-23 21:21'
labels:
  - M2
  - ci
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
At M2, add a CI job that, for each examples/NN-name/, re-runs the reference regeneration command from docs/reference-impl-policy.md §1 and byte-diffs the produced output against the committed reference.bin. Any difference fails the build, naming the example and pointing at the policy. Required: PR-time gate (not nightly) on any commit touching *.algo.nuc, examples/NN-name/reference/**, or reference.bin. See docs/reference-impl-policy.md §6.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-until-TASK-0057-lands (orchestrator-direct, cycle 77 sweep). Description: 'At M2, add a CI job that, for each examples/NN-name/, re-runs the reference regeneration... Required: PR-time gate (not nightly) on any commit touching *.algo.nuc, examples/NN-name/reference/**, or reference.bin.' The just regen-references recipe landed cycle 77 (TASK-0077) provides the LOCAL maintenance entry point; the CI wiring is the remaining piece and is gated on TASK-0057 (CI workflow matrix runner, in_progress). When TASK-0057 lands a stable CI workflow, this task's CI hook is added at that point — scoped to whatever path-filter / runner mechanics TASK-0057 settles on (today's ci.yml shape is mid-flight, so freezing the hook now would silently rot). Reopen when TASK-0057 closes. Same deferred-until-prerequisite pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
