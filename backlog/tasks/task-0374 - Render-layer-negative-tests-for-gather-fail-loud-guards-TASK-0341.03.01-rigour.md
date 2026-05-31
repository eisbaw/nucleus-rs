---
id: TASK-0374
title: >-
  Render-layer negative tests for gather fail-loud guards (TASK-0341.03.01
  rigour)
status: In Progress
assignee:
  - '@me'
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 00:52'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add backend-common/tests/render_gather_negative.rs mirroring fire_args_nostd.rs + whole_array_classifier.rs RenderCtx setup. 4 negative tests (one per fail-loud path in render_gather_index_load, expr.rs:88-138) + 1 positive control: (1) empty-indices -> UnsupportedFeature substring "whole-array reference"/"fully-indexed scalar"; (2) missing-DataId -> ContractGap substring "no DataId"; (3) missing-ResolvedType -> ContractGap substring "no ResolvedType"; (4) partial-rank (indices.len != ty.dims.len) -> UnsupportedFeature substring "FULL-RANK"; (5) POSITIVE: rank-1 col[k] full-rank renders Ok, pinned exactly "col[(k) as usize]" + contains "col[". Each assertion msg names the expr.rs guard line being pinned. Top-of-file docstring: these 4 guards had ZERO unit coverage (e2e hits only full-rank happy path) and partial-rank is SOURCE-REACHABLE (x[col[i]] on 2D col lowers fine). Gate: nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"; e2e MUST stay 329/272/0/57/0.
<!-- SECTION:PLAN:END -->
