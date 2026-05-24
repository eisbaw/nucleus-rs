---
id: TASK-0286
title: >-
  Reuse codegen: canonicalise outer_axes before dedupe (TASK-0282 P2.1
  hardening)
status: To Do
assignee: []
created_date: '2026-05-24 18:19'
labels:
  - M5
  - reuse
  - codegen
  - hardening
  - forward-carried-from-TASK-0282
dependencies:
  - TASK-0282
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0282 (cycle 110, commit 6984c64) generalised the reuse-codegen rewrite to emit one circular buffer per UNIQUE (data_id, axis, outer_axes_tuple). Dedupe at `walk_arg_for_reuse` (`nucleus/backend-common/src/render.rs:1370-1375`) uses `IrExpr` `PartialEq` (structural equality) on raw `DataSlice.indices` clones — no normalisation pass between source and dedupe.

## Risk
Distinct-but-equivalent outer-axes ASTs would emit redundant buffers:
- `y - 1 + 0` vs `y - 1` — two buffers, both with the same source-array slice.
- `y` vs `y + 0` — same.
- An upstream affine pass that emits `Add(Ident("y"), IntLit(0))` in one DataSlice and `Ident("y")` in another would silently bloat the buffer count.

Direction of risk is CONSERVATIVE-SAFE (over-emission, never coalesce-distinct) — the structural-equality dedupe cannot fold semantically-distinct shapes incorrectly. But the AC#4 `<= 3` grep at `nucleus/nucleus-compiler/tests/e2e_example_05.rs` would NOT catch a silent over-emission (it bounds verbatim reads, not buffer count).

## Today's mitigation
Upstream `affine_decompose` / link-time passes canonicalise iv-axis indices. Outer indices in shipped fixtures are user-source `y` / `y+1` / `y-1` literals only, so this is benign in practice TODAY.

## Acceptance Criteria
1. Add a `canonicalise_outer_axes(&[IrExpr]) -> Vec<IrExpr>` helper at `render.rs`, applied at the dedupe insertion site BEFORE `found.iter().any(...)`. Fold trivial `Add(_, IntLit(0))` / `Sub(_, IntLit(0))` and similar identity cases.
2. New unit test: `task0282_dedupe_canonicalises_add_zero` — build two DataSlices with semantically-equal outer axes (one with `Add(y, IntLit(0))`, one with `Ident(y)`) and assert `discover_reuse_groups` returns ONE group, not two.
3. e2e baseline 92/79/0/13/0 preserved.

## Dependencies
- TASK-0282 (Done).

## Honest scope
- Cheap defence — single helper + one regression test. NOT a load-bearing fix today (no shipped fixture triggers the over-emission); files exists to keep future cycles structurally clean before a new affine pass lands that might emit non-canonical outer axes.

## Forward-carried from TASK-0282 architect P2.1
<!-- SECTION:DESCRIPTION:END -->
