---
id: TASK-0076
title: 'CI hook: verify reference.bin freshness on PRs touching *.algo.nuc'
status: To Do
assignee: []
created_date: '2026-05-17 23:39'
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
