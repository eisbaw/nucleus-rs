---
id: TASK-0459
title: >-
  build_sidecar: cumulative_data silently drops on name-to-id desync while its
  sibling panics loud — make it loud
status: To Do
assignee: []
created_date: '2026-06-09 22:00'
labels:
  - silent-sibling
  - compiler
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P3.11). build_sidecar treats the SAME name<->id desync invariant two different ways: data_decl_order fails loud (sidecar.rs:858-866) while cumulative_data filter_maps the desync away silently (sidecar.rs:837-840). A silently-dropped cumulative symbol would skip the COPY-not-accumulate exclusion — the xN-double-count protection that is value-correctness-load-bearing (see the 16-jacobi cumulative-array memory: whole-array accumulate was xN-wrong for cumulative cross-iteration state until the discriminator landed).

Recurring classes: feedback-silent-sibling-defect + feedback-option-none-skip-arm-silent-drop. Work: make the cumulative_data path fail loud identically to its sibling; add a unit test constructing the desync; then audit the rest of build_sidecar for further filter_map / skip-arm siblings of the same invariant and fix or justify each.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 cumulative_data desync is loud (typed error or panic matching the data_decl_order treatment), never a silent drop
- [ ] #2 Unit test constructs the desync and pins the loud outcome
- [ ] #3 build_sidecar audited for sibling skip-arms; each fixed or justified in notes
<!-- AC:END -->
