---
id: TASK-0060
title: Decide bit-identical strategy for floating-point examples
status: To Do
assignee: []
created_date: '2026-05-17 23:10'
labels:
  - validation
  - docs
  - M0
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §10.1 and §12: bit-identical claim is non-negotiable, but float reductions reorder and break it. Decide policy: integer-only examples, or controlled-FP examples (fixed reduction order), or both. Document the decision and back-fill it into existing examples.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/numeric-determinism.md documents the v2 rule for FP: either integer-only, or FP with fixed evaluation order across schedules.
- [ ] #2 Each existing example is reviewed; any FP example confirms its determinism strategy in its README.
- [ ] #3 Test: a deliberately reordering test (e.g. swap two adds in a parallel reduction) breaks determinism — shows the rule bites.
- [ ] #4 Implementation notes record the design discussion (e.g. why not epsilon comparison).
- [ ] #5 Implementation notes record honest limitations (FP determinism narrows the algorithm class noticeably; this is a model-level trade-off, not a bug).
<!-- AC:END -->
