---
id: TASK-0374
title: >-
  Render-layer negative tests for gather fail-loud guards (TASK-0341.03.01
  rigour)
status: Done
assignee:
  - '@me'
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 01:10'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE 2026-05-31. Added backend-common/tests/render_gather_negative.rs (commit 9f51cc8) with 5 tests covering ALL 4 fail-loud arms of render_gather_index_load (expr.rs:88-138) plus a positive control. All 5 pass (cargo test -p backend-common --test render_gather_negative: 5 passed). Coverage: (1) empty-indices -> UnsupportedFeature, msg names whole-array reference + col (expr.rs:92-98); (2) missing-DataId -> ContractGap, msg "no DataId" (expr.rs:107-112); (3) missing-ResolvedType -> ContractGap, msg "no ResolvedType" (expr.rs:113-118); (4) partial-rank (rank-1 index over rank-2 data) -> UnsupportedFeature, msg "FULL-RANK" + names the 1-vs-rank-2 mismatch (expr.rs:119-128, the SOURCE-REACHABLE arm); (5) positive control: full-rank rank-1 col[k] renders Ok, snapshot-pinned exactly to col[(k) as usize] + asserts contains col[. Each assertion names the expr.rs guard line. Gate green: build/clippy/test/test-release all 0-fail, e2e 329/272/0/57/0 unchanged. GOTCHA: cargo must be invoked --manifest-path nucleus/Cargo.toml (workspace root is nucleus/, not repo root) or via just from the dev shell. No follow-ups filed.
<!-- SECTION:FINAL_SUMMARY:END -->
