---
id: TASK-0113
title: 'Optimisation pass: drop redundant Syncs after transfer injection'
status: To Do
assignee: []
created_date: '2026-05-18 01:34'
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
