---
id: TASK-0304
title: >-
  Pin transfer_inject behaviour claims on schedule headers (TASK-0299/0303
  sibling-pattern follow-up; different fixture idiom)
status: To Do
assignee: []
created_date: '2026-05-25 03:09'
labels:
  - M5
  - compiler
  - test-coverage
  - transfer_inject
  - comment-doc-lie
  - forward-carried-from-TASK-0299
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0299 (cycle 119) and TASK-0303 (cycle 120) closed the halo_widths-VALUE narrative-pin sweep. Cycle 120's architect noted that two further unpinned narrative claims live in schedule headers but are about transfer_inject BEHAVIOUR, not halo_widths values — they need a different fixture idiom (assert on the injected per-tile transfer ranges, not on halo widths).

## What to pin

### Sibling 1: 06-separable-filter/distributed.sched.nuc:19-21 (second conjunct)

Header at lines 19-21 is a TWO-PART conjunction. TASK-0299 cycle 119 pinned the first half ('halo_widths[hblur_acc][hy] = 0'). The second half ('transfer_inject does NOT extend per-tile transfer ranges') is unpinned. A regression that broke this conjunct without touching the first (e.g. a future transfer_inject that unconditionally extends tile ranges even when halo=0) would not trip task0299_*; only e2e bytes would catch it.

### Sibling 2: 05-stencil/distributed.sched.nuc:30-34

Schedule comment claims 'TASK-0263 (cycle 83) wired halo widths into transfer_inject so each per-worker tile carries the halo strips its blur3 reads from neighbouring bands.' This is a transfer_inject BEHAVIOUR claim (per-tile transfer ranges ARE extended by halo). Equally unpinned today; e2e bytes catch regressions silently.

## Acceptance criteria

1. Add a test in nucleus/nucleus-compiler/tests/transfer_inject.rs (or sibling) that loads 06-separable-filter/prog.algo.nuc + schedules/distributed.sched.nuc and asserts: for the in_arr tile passed to each worker, the tile bounds on hy do NOT have a halo extension (halo=0 → no extension). The exact fixture shape depends on how transfer_inject exposes per-tile ranges — either via the ACFG sidecar (tile bounds queryable from a post-pass ACFG) or via inspecting the emitted IterTile in the Operation graph.

2. Add a second test that loads 05-stencil/prog.algo.nuc + schedules/distributed.sched.nuc and asserts: for the img_in tile passed to each worker, the tile bounds on y ARE extended by halo=1 in both directions. Pins the positive-extension behaviour the schedule comment narrates.

3. Test docstrings name the specific schedule-header line they pin and explain the failure mode.

## Honest scope

LOW priority. Pure narrative-pinning hygiene at the transfer_inject layer. The e2e bytes already bite on wrong output; this is narrative-coverage parity with the TASK-0299/0303 halo_widths-value pins.

## Fixture-idiom delta vs TASK-0299/0303

TASK-0299/0303 used the existing lower() helper which returns (LinkedIR, ACFG) post-halo_inference. This task may need to access per-tile transfer ranges that live in the ACFG's per-Operation sidecars or per-edge data. Implementer should pick the cleanest assertion shape:
- Option A: extend lower() to return post-inject_transfers ACFG; grep the resulting per-Operation transfers for the tile-bound shape.
- Option B: hand-build a synthetic ACFG mirroring transfer_inject_hoist.rs's test pattern.
- Option C: query the NameSidecar's per-transfer fields.

## Cross-references

- TASK-0299 (cycle 119, Done) — first-half pin precedent.
- TASK-0303 (cycle 120, Done) — sibling-sweep predecessor.
- cycle-120 architect review-gate Recommendation #1.
<!-- SECTION:DESCRIPTION:END -->
