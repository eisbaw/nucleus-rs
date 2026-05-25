---
id: TASK-0304
title: >-
  Pin transfer_inject behaviour claims on schedule headers (TASK-0299/0303
  sibling-pattern follow-up; different fixture idiom)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 03:09'
updated_date: '2026-05-25 05:05'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT cycle 124.

DESIGN: Option A. The existing sidecar_halo.rs::lower() helper already runs the FULL pipeline including inject_transfers (see lines 46-68); after lower(), acfg.root.collect_xfers() returns every Push/Wait XferPlaceholder. Each XferPlaceholder has tile.bounds: Vec<(IterVar, Range<i64>)>.

ASSERTION SHAPE: per-worker in_arr / img_in Push tile bounds on the partitioned iv:
- Sibling 1 (06-separable-filter/distributed, halo=0): for each Push from host to w_i of in_arr, the hy bound MUST EQUAL the partition band — NO extension. Pins the schedule-header second-conjunct claim 'transfer_inject does NOT extend per-tile transfer ranges'.
- Sibling 2 (05-stencil/distributed, halo=1): for each Push from host to w_i of img_in, the y bound MUST BE STRICTLY WIDER than the partition band on at least one side. Pins the cycle-83 (TASK-0263) cross-worker halo wiring narrative.

Tests live next to task0299_/task0303_ in sidecar_halo.rs (same lower() helper, same narrative-pin pattern, named task0304_*).

STEPS:
1. Read transfer_inject's actual halo-extension logic at compute_partition_bounds_with_dim_prefix (TASK-0301/0302) to understand the extension shape — specifically what bound is emitted for halo=1 at the band edges. Avoid coupling to specific edge clamping.
2. Implement task0304_06_separable_filter_distributed_no_halo_extension_on_in_arr_hy: for each Push of in_arr to a compute worker, hy bound == partition band.
3. Implement task0304_05_stencil_distributed_halo_one_extension_on_img_in_y: for each Push of img_in to a compute worker, y bound is wider than partition band on at least one side.
4. Run cheap gate.
5. Parallel review gate (qa-test-runner + mped-architect).
6. Address findings; re-gate.
7. Commit + close.

GATE: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e' — e2e baseline 108/92/0/16/0 must hold (pure additive test).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR-DIRECT cycle 124 (2026-05-25). TEST-ONLY.

SHIPPED:
- nucleus/nucleus-compiler/tests/sidecar_halo.rs imports: +WorkerId, +XferRole (both via crate-root re-exports per cycle-124 architect P2.5 symmetry nit), +BTreeSet.
- nucleus/nucleus-compiler/tests/sidecar_halo.rs +2 tests (~200 LOC total) at end-of-file:
  * task0304_06_separable_filter_distributed_transfer_inject_no_halo_extension_on_in_arr_hy — for each in_arr Push to a compute worker, asserts tile.bounds[hy] EQUALS partition band (no extension because halo_widths[hblur_acc][hy] = 0). Pins the SECOND CONJUNCT of 06-separable-filter/schedules/distributed.sched.nuc:19-21 (the BEHAVIOUR-layer claim that task0299_06's VALUE-layer pin does NOT cover).
  * task0304_05_stencil_distributed_transfer_inject_halo_one_extension_on_img_in_y — for each img_in Push to a compute worker, asserts tile.bounds[y] EQUALS (band.start - 1)..(band.end + 1) (exact extension by halo=1 on both sides). Pins the cycle-83 TASK-0263 narrative at 05-stencil/schedules/distributed.sched.nuc:30-34.

DESIGN: Option A from TASK-0304's spec. The existing sidecar_halo.rs::lower() helper already runs the FULL pipeline including inject_transfers, so post-lower() acfg.root.collect_xfers() returns every XferPlaceholder. Reading partition_worker_ranges for the band source-of-truth keeps the test robust to TASK-0262 remainder-policy changes.

GATE (cycle 124 close):
- just build: clean.
- just clippy -D warnings: clean.
- just test: 852/0/3 across 76 binaries (up from 850 cycle 123).
- just test-release: 852/0/3 (matches dev).
- just e2e: 108/92/0/16/0 UNCHANGED from cycle-123 baseline (purely additive test, no code change).
- Targeted: cargo test task0304 → 2/2 pass; ran 4× independent (dev + release) — stable, no flake.

REVIEW GATE (cycle 124 parallel read-only):
- qa-test-runner: GO. Verified counts dev+release+e2e; ran flake check 4×. Findings: NO P0/P1/P2 issues. Noted minor observation: closes one feedback-silent-sibling-defect arm at the behaviour layer for 05+06.
- mped-architect: GO. Verified docstring line citations (06:19-21, 05:30-34, transfer_inject.rs:~2079/~2188/~2197-2217 all confirmed); verified band math (H=16, y 1..15, 4 numpy.array_split bands all inside clamp [0, 16]); CONFIRMED counterfactual that a tightened-clamp-to-data-extent ([1,15]) WOULD fail w0 (0→1) + w3 (16→15) — the docstring's claim is empirically correct. Empirical verification via target/e2e-matrix/05-stencil/distributed/pthreads-async/src/main.rs lines 74+108 (img_in[0..96] for w0 + img_in[64..160] for w1) — matches the asserted band±1 extension.
- Architect surfaced 4 follow-ups:
  * P1.1 (plan-vs-implementation strength drift): plan said weaker existential ('wider on at least one side'), implementation used exact equality (band.start-1)..(band.end+1). Architect explicitly accepts because the test docstring HONESTLY discloses the cost — a future clamp-policy change must update this test, which the docstring at sidecar_halo.rs:~988-998 names. Architect call: accept as-is.
  * P2.1 (lower vs lower_partition_aware): test pipeline uses STRICT-A but driver uses partition-aware-B. Pre-existing in lower(); inherited by new tests. Filed as TASK-0309.
  * P2.2 + P2.4 (sibling-sweep): 05/distributed-2d (2D shape) + 07-matmul/distributed (halo=0 behaviour) unpinned at the behaviour layer despite VALUE-layer pins (task0303_05 + task0303_07). Filed combined as TASK-0310.
  * P2.5 (import symmetry): cosmetic — WorkerId via deeper event:: path while XferRole used crate-root. Architect recommended fixing in-commit. APPLIED in-commit: both now via root re-export.
  * P2.3 (gather direction): tmp/img_out Pushes uncovered. Architect explicitly says do-not-block (narrative is specifically about reads). Noted only.

GOTCHAS + FORWARD-CARRY:
- Plan-implementation strength drift: I unilaterally upgraded the assertion from existential ('wider') to exact-equality ('band±1'). The architect's framing helps: if the implementation supports the stronger pin AND the docstring is honest about the cost, the strong pin is preferred. General principle for future cycles: when upgrading assertion strength beyond the plan, the test's own docstring must declare the upgrade + the cost (which counterfactuals would now fail).
- The lower()/lower_partition_aware() divergence (P2.1) is a multi-cycle hygiene gap that has been silently inherited by every halo-sidecar test in this file. Filing TASK-0309 surfaces it for a future fresh-context implementer.
- The schedule-header narrative at 06-separable-filter/distributed.sched.nuc:19-21 is a TWO-PART CONJUNCTION. Cycle 119 pinned the first half (task0299_06); cycle 124 pinned the second half (task0304_06_*). General principle: schedule-header narrative claims with conjunctions ALWAYS need each conjunct pinned separately — pinning one and assuming the other follows is the silent-sibling pattern at the narrative-clause level.
- Imports note: nucleus_compiler::WorkerId is re-exported at the crate root via lib.rs:38. Always prefer the root re-export over the deeper module path (matching project convention for symmetric/clean imports).

FILES SHIPPED (cycle 124):
- nucleus/nucleus-compiler/tests/sidecar_halo.rs (+200 / -2): 2 new tests + 3-line import.
- backlog/tasks/task-0304 - ... (status: To Do → Done; plan + notes + final summary).
- backlog/tasks/task-0309 - ... (new, P2.1 lower vs lower_partition_aware follow-up).
- backlog/tasks/task-0310 - ... (new, P2.2 + P2.4 distributed-2d + 07-matmul behaviour sibling-sweep).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0304 cycle-124 (2026-05-25) LANDED. Two behaviour-layer narrative pins (sibling-pattern to task0299_06 + task0303_07 at the value layer) close the schedule-header second-conjunct gap on 06-separable-filter/distributed (halo=0 → no-extension) and the cycle-83 TASK-0263 wired-extension narrative on 05-stencil/distributed (halo=1 → band±1). AC#1 (06 test) + AC#2 (05 test) + AC#3 (docstrings cite the defended schedule line range + name failure mode) all satisfied. Implementer chose Option A from the AC (use the existing lower() helper, query acfg.root.collect_xfers()) — simplest of the 3 options + lowest coupling. Strength: exact-equality pin (upgraded from plan's existential 'wider on at least one side'); the upgrade IS disclosed in the test docstring with the counterfactual cost (tightened clamp would fail w0+w3). Review gate: BOTH reviewers GO with 4 follow-ups — P1.1 (strength drift) accepted via docstring honesty; P2.5 (import symmetry) APPLIED in-commit; P2.1 (lower vs lower_partition_aware divergence) filed as TASK-0309; P2.2 + P2.4 (distributed-2d + 07-matmul behaviour sibling-sweep) filed combined as TASK-0310; P2.3 (gather direction) noted, do-not-block per architect. Gate: 852/0/3 tests dev+release, e2e 108/92/0/16/0 unchanged.
<!-- SECTION:FINAL_SUMMARY:END -->
