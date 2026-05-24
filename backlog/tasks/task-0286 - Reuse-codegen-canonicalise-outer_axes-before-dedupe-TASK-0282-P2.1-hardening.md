---
id: TASK-0286
title: >-
  Reuse codegen: canonicalise outer_axes before dedupe (TASK-0282 P2.1
  hardening)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-24 18:19'
updated_date: '2026-05-24 18:49'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implementation:

1. Add private helper canonicalise_outer_axes (Vec<IrExpr>) -> Vec<IrExpr> in render.rs (alongside other reuse-codegen helpers).
   - Recursive fold: Add(e, IntLit(0)) -> canonical(e); Add(IntLit(0), e) -> canonical(e); Sub(e, IntLit(0)) -> canonical(e). Apply once per node, propagating through child positions. Mul(e, IntLit(1)) / Mul(IntLit(1), e) -> canonical(e) — cheap to include for parity.
   - Pass canonicalisation: walk every node bottom-up, return a new expression. Stable on already-canonical input (deterministic + idempotent).
2. Apply at walk_arg_for_reuse:1364-1369 immediately after outer_axes is built and before the dedupe.iter().any(...) check. Both stored outer_axes and the search key need to be canonicalised — otherwise a previously-stored canonical form would NOT match a fresh non-canonical input.
3. Unit test in nucleus/backend-common/tests/multi_worker_reuse_marker.rs or a new module: build a synthetic body with TWO Fire calls — one with DataSlice indices=[Ident(y), Ident(x)+IntLit(-1)], one with DataSlice indices=[Add(Ident(y), IntLit(0)), Ident(x)+IntLit(0)]. Assert discover_reuse_groups returns ONE group (post-canonicalise the outer-axes tuples [y] match).
4. Verify e2e baseline 92/79/0/13/0 preserved (the existing fixtures emit already-canonical outer axes, so canonicalisation is a no-op on every shipped path).

Gate: cargo test (818+1), cargo clippy --workspace --all-targets -- -D warnings clean, just e2e 92/79/0/13/0, just determinism-check 92/79/0/13.

Honest scope: pre-emptive defence. No shipped fixture triggers the over-emission. Test exists to bite if a future upstream affine pass emits non-canonical outer axes.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-111 REVIEW: qa-test-runner GO (820 tests pass / 0 fail; e2e 92/79/0/13/0 across 2 samples; determinism 92/79/0/13; no new fmt drift on touched files). mped-architect GO (idempotence + termination by bottom-up walk + finite IrExpr; slice boundary + UTF-8 safety verified; DataRef/Call pass-through bonafide per ir.rs:180-186; identity folds sound under all integer semantics). P3 observations from architect (advisory, no action): commuted Add(IntLit(neg), e) form not unfolded — passes/common.rs:274 always emits direct Sub form so no production trigger; constant folding + associativity + Add->Sub conversion explicitly out-of-scope per honest-scope discipline. Memory entry project-cross-backend-differential updated to reflect cycle-111 closure of P2.1+P2.2 forward-carries.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 111 LANDED in commit 840ecb3. canonicalise_outer_axis(IrExpr) helper folds e+0 / 0+e / e-0 / e*1 / 1*e to e before the dedupe key in walk_arg_for_reuse takes outer_axes as a key. Bite-verified regression test multi_worker_walker_dedupes_canonical_outer_axes_add_zero in backend-common/tests/multi_worker_reuse_marker.rs: two Fire events with outer Ident(y) vs Add(Ident(y), IntLit(0)) coalesce into one g0 group (pre-fix would produce g0 + g1). Orchestrator manually verified the bite (temporarily reverting the call produces _g1 in the emit). All 3 ACs met. Pre-emptive hardening only; no shipped fixture triggers today, but the AC#4 verbatim-read grep cannot detect silent buffer-bloat so this closes a real silent-failure mode. Gate: 819 tests pass, e2e 92/79/0/13/0, determinism 92/79/0/13, clippy clean. Cycle-111 review (qa + architect) both GO.
<!-- SECTION:FINAL_SUMMARY:END -->
