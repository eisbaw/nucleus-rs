---
id: TASK-0374
title: >-
  Render-layer negative tests for gather fail-loud guards (TASK-0341.03.01
  rigour)
status: To Do
assignee: []
created_date: '2026-05-30 22:46'
labels:
  - backend
  - gather
  - test
  - rigour
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
RIGOUR follow-up to TASK-0341.03.01. render_gather_index_load (backend-common/src/render/expr.rs) has 4 fail-loud paths NONE of which has a unit test (the 7 e2e cells exercise only the full-rank happy path): empty-indices reject, missing-DataId ContractGap, missing-ResolvedType ContractGap, and the PARTIAL-RANK guard (iref.indices.len() != ty.dims.len() -> UnsupportedFeature). The partial-rank guard is REACHABLE from the surface: x[col_idx[i]] on a 2D col_idx lowers fine (lowering does not rank-check the inner ref), so the render guard is the sole defense. Add backend-common render unit tests (mirror whole_array_classifier.rs RenderCtx setup) asserting each EmitError fires. Architect P2.2 (gather review).
<!-- SECTION:DESCRIPTION:END -->
