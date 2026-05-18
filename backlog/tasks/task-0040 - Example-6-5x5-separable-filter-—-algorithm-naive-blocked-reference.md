---
id: TASK-0040
title: 'Example 6: 5x5 separable filter — algorithm + naive + blocked + reference'
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
labels:
  - M3
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two-pass stencil with intermediate buffer (horizontal blur, then vertical). Stresses intermediate-data lifetime across passes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/06-separable-filter/ has algo + schedules + kernels.rs + reference + binaries.
- [ ] #2 Algorithm has two sequential loops; intermediate array is single-assignment within scope.
- [ ] #3 Test: passes M3 differential matrix on both M3 backends.
- [ ] #4 Implementation notes record design questions (e.g. should the intermediate buffer be hinted to live in fast memory; deferred to schedule's place_data).
- [ ] #5 Implementation notes record honest limitations (clamp boundaries; no reuse-with-shift; integer-typed).
<!-- AC:END -->
