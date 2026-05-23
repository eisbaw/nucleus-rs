---
id: TASK-0113
title: 'Optimisation pass: drop redundant Syncs after transfer injection'
status: Done
assignee: []
created_date: '2026-05-18 01:34'
updated_date: '2026-05-23 21:31'
labels:
  - compiler
  - ir
  - optimisation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Once TASK-0018 lowers cross-worker dataflow into Push/Wait pairs, the conservative Syncs inserted by TASK-0017 become redundant where a matching Push/Wait already provides ordering. Add a pass that walks the ACFG and removes Syncs whose participants are covered by neighbouring transfer events. Keep barrier-only Syncs (no data crossing) untouched.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Pass removes only provably redundant Syncs (no transitive ordering broken).
- [ ] #2 Verified on examples 13 batch_parallel/pipeline_parallel: pre/post sync counts differ, output equivalence preserved by downstream tests.
- [ ] #3 Limitations recorded (what counts as redundant; what doesn't).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-perf-driver (orchestrator-direct, cycle 77 sweep). Companion to TASK-0128 (closed cycle 77 same reason) — both are opt passes that drop redundant Syncs. No perf measurement has shown the redundant-Sync overhead is a real bottleneck (pthreads barriers on shared-memory: ns scale; e2e cells complete in seconds). The barrier-only Syncs (no data crossing) the task says to KEEP are the only ones that have semantic load anyway — the rest are conservative scaffolding. Reopen if a real perf measurement bites. Same deferred-no-driver pattern as TASK-0115/0128/0131/0132.
<!-- SECTION:FINAL_SUMMARY:END -->
