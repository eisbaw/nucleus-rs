---
id: TASK-0310
title: >-
  transfer_inject behaviour pin sibling-sweep: 05-stencil/distributed-2d +
  07-matmul/distributed behaviour layer (TASK-0304 cycle-124 architect P2.2 +
  P2.4)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 05:04'
updated_date: '2026-05-25 05:26'
labels:
  - M5
  - test-coverage
  - transfer_inject
  - sibling-sweep
  - forward-carried-from-TASK-0304
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0304 cycle 124 LANDED behaviour-layer pins for 05-stencil/distributed (halo=1 extension by ±1) and 06-separable-filter/distributed (halo=0 no-extension). The cycle-124 architect review-gate flagged two STRUCTURALLY IDENTICAL sibling narratives that remain unpinned at the behaviour layer — the precise pattern `feedback-silent-sibling-defect` warns about.

## What to pin

### Sibling 1: 05-stencil/distributed-2d.sched.nuc (2D-blocks2d shape, P2.2)

The 2D distributed-2d schedule for 05-stencil makes the SAME load-bearing TASK-0263 transfer_inject claim as 05/distributed but for partition=blocks2d. `task0303_05` (cycle 120) pinned the halo_widths VALUE layer for blur3 in this schedule; the BEHAVIOUR layer (per-tile transfer ranges actually extended by ±1 on both y AND x) is unpinned. A regression in transfer_inject's 2D extension path (e.g. an iv↔dim mapping defect, see open TASK-0302) would pass `task0303_05` AND `task0304_05_stencil_distributed_*` and be caught only at e2e bytes.

### Sibling 2: 07-matmul/distributed.sched.nuc (1D shape, halo=0, P2.4)

TASK-0303 (cycle 120) pinned `task0303_07_matmul_distributed_halo_widths_pinned_to_zero` (the halo_widths VALUE side: `madd_i == 0`). The BEHAVIOUR side is the SAME pattern as 06-separable-filter/distributed: halo=0 → transfer_inject does NOT extend per-tile transfer ranges. The schedule header at `nuc-nucleus/examples/07-matmul/schedules/distributed.sched.nuc:25` carries this claim (`no halo, no cross-worker carry`). Unpinned at the behaviour layer.

## Acceptance criteria

1. Add `task0309_05_stencil_distributed_2d_transfer_inject_halo_one_extension_on_img_in_y_AND_x` to `nucleus/nucleus-compiler/tests/sidecar_halo.rs`. For each img_in Push to a compute worker under partition=blocks2d, assert tile.bounds[y] AND tile.bounds[x] are EACH band±1 (or band±halo where applicable). Use the existing `lower()` helper.
2. Add `task0309_07_matmul_distributed_transfer_inject_no_halo_extension_on_a_i` to the same file. For each a Push to a compute worker under partition=workers (or whatever partition shape 07-matmul/distributed uses for i), assert tile.bounds[i] == partition band (no extension because halo_widths[madd][i] = 0).
3. Each test cites the schedule-header line range it defends and names the failure mode in the assert message (matching the task0304_* idiom).
4. e2e baseline 108/92/0/16/0 preserved.

## Implementer hint

- The TASK-0304 cycle-124 lower() helper at sidecar_halo.rs:46-68 runs the full pipeline and returns post-inject_transfers ACFG. Use `acfg.root.collect_xfers()` + filter on `XferRole::Push && data == DataId`.
- For 05/distributed-2d: read `acfg.partition_blocks2d_ranges[(y_iv, x_iv)]` for the per-worker 2D band map. The pre-cycle-124 architect noted that the 2D iv→dim mapping has known limits (TASK-0302 open) — be defensive about whether bounds carry both ivs.
- For 07/distributed: structurally identical to task0304_06_*; copy the idiom.

## Honest scope

LOW priority. The behaviour-layer regression risk for halo-bearing distributed schedules is low (the cycle-83 TASK-0263 + cycle-118 TASK-0301 + cycle-121 TASK-0302 axis-mapping passes all have extensive coverage). This task closes the across-schedule sibling-sweep gap.

## Cross-references

- TASK-0304 cycle 124 architect P2.2 + P2.4 — the gap-discovery review-gate.
- TASK-0303 cycle 120 — VALUE-layer sibling pins for 05-distributed-2d + 07-matmul/distributed.
- TASK-0302 — open 2D iv↔dim mapping limit; may bear on the 05/distributed-2d test fixture shape.
- Memory: `feedback-silent-sibling-defect` — the recurrence pattern this task closes.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 125 implementation plan (orchestrator in-thread, mirror cycle-124 task0304_* idiom):
- Add task0310_05_stencil_distributed_2d_transfer_inject_halo_one_extension_on_img_in_y_AND_x to nucleus/nucleus-compiler/tests/sidecar_halo.rs. Lower 05/distributed-2d; filter acfg.root.collect_xfers() for XferRole::Push && data == img_in_id; for each per-worker Push, lookup partition_worker_ranges[y_iv][x.dst] AND partition_worker_ranges[x_iv][x.dst]; assert both bounds present in x.tile.bounds; assert each EQUAL (band.start - 1)..(band.end + 1). Source range for y and x is 1..15; bands are 7-wide → expansion stays inside source clamp; strong band±1 holds. Assert seen_workers.len() == 4 (w0..w3).
- Add task0310_07_matmul_distributed_transfer_inject_no_halo_extension_on_a_i. Lower 07/distributed; filter for XferRole::Push && data == a_id; for each per-worker Push, lookup partition_worker_ranges[i_iv][x.dst]; assert i bound present + EQUAL the band (no extension because halo_widths[madd][i] = 0). Assert seen_workers.len() == 4.
- Each test cites the schedule-header line range; failure message names the conjunct + the precise tile.bounds[iv] vs expected.
- Run nix develop -c just check + clippy + test + test-release + e2e to confirm 108/92/0/16/0 preserved and tests pass dev+release.
- Spawn parallel read-only review gate (qa-test-runner + mped-architect) on the cycle commit.
- Commit per the project convention (no AI co-author), append-notes + final-summary to TASK-0310, mark Done iff gate is GO.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 125 (2026-05-25) LANDED at commit e26cfb1. 2 new behaviour-pin tests appended at the bottom of nucleus/nucleus-compiler/tests/sidecar_halo.rs (lines 1066-1365):

- task0310_05_stencil_distributed_2d_transfer_inject_halo_one_extension_on_img_in_y_and_x (305 LoC incl. doc paragraph): lowers 05/distributed-2d via the shared lower() helper, filters acfg.root.collect_xfers() to host-broadcast Pushes of img_in (excludes cross-worker halo-strip Pushes from inject_halo_strip_xfers TASK-0289 cycle-114a by checking x.src ∉ partition_worker_ranges[y_iv].keys()), asserts BOTH y and x bounds present in tile.bounds AND each == (band.start - 1)..(band.end + 1). seen_workers.len() == 4 closes the vacuous-pass arm (2x2 grid → 4 workers). Cross-references the open TASK-0306 latent defensive panic.
- task0310_07_matmul_distributed_transfer_inject_no_halo_extension_on_a_i: lowers 07/distributed, filters Pushes of a (which is indexed [i][k] so dim-prefix yields non-empty bounds — narrative paragraph documents why a and not b or c), asserts i bound EQUAL partition band (no extension, halo_widths[madd][i] = 0). Same vacuous-pass defence.

AC#1 (sibling 1 / P2.2): SATISFIED (task0310_05 above).
AC#2 (sibling 2 / P2.4): SATISFIED (task0310_07 above).
AC#3 (cite schedule-header line range + name failure mode): SATISFIED (each test cites the schedule narrative paragraph + names the failure mode in the assert message; mirrors task0304_* idiom).
AC#4 (e2e baseline 108/92/0/16/0 preserved): SATISFIED, verified by 2 e2e samples (qa-test-runner re-run + orchestrator-pre-run), non-flake.

Cycle gate (independently reproduced by qa-test-runner sub-agent on commit e26cfb1):
- just check + just clippy: clean.
- just test (dev): 854/0/3 (+4 since cycle-123 850/0/3 baseline: cycle-124 added task0304_05 + task0304_06; cycle-125 added task0310_05 + task0310_07).
- just test-release: 854/0/3 (no dev/release divergence; TASK-0291 discipline).
- just check-textual-replace-on-codegen + just check-include-str-coverage: OK.
- just e2e × 2 samples: 108/92/0/16/0 bit-identical, non-flake.

Architect review-gate findings (read-only): GO. Verified all 4 spot-check citations against code (partition_pairs single-insert site at partition_blocks2d.rs:443; halo==0 early-out at transfer_inject.rs:2188; inject_halo_strip_xfers AC#3 short-circuit at transfer_inject.rs:2538; 05-stencil source ranges 1..H-1 with H=16). Zero new P1/P2/P3 introduced. 2 mention-only P3 observations (precedent-consistent with cycle-124, not blockers): (P3-1) output-Push c for 07-matmul not pinned (cycle-124 task0304_06 set the single-data-pin precedent); (P3-2) the 'Why a and not b or c' multi-claim paragraph kept open for the cycle-NN+1 comment-doc-lie audit lane (verified at review-time: b's dim-prefix → empty bounds → whole-array broadcast claim holds).

Gotchas / subtleties surfaced this cycle (forward-carry):

1. **2D-blocks2d second Push class** (NEW project-level gotcha): under partition=blocks2d, transfer_inject emits TWO Push classes for each halo-bearing data: (a) host-broadcast main Pushes carrying band±halo on each axis, (b) inject_halo_strip_xfers cross-worker halo strips carrying 1-row / 1-column slices. A naive filter (XferRole::Push && data == X_id) catches BOTH. Future behaviour-pin tests under partition=blocks2d MUST disambiguate by checking x.src — if x.src is a key of partition_worker_ranges[outer_iv] it's a strip, not a main Push. Discovered empirically by a real test failure (5-stencil w1 strip tile.bounds=[(IterVar(1), 8..9), (IterVar(0), 1..8)] — a 1-column W↔E strip from w0 to w1's east edge), not by narrative-only reasoning.

2. **snake_case clippy bite on test names**: clippy  rejected the TASK-0310 brief's literal  test-name suffix. Renamed to . Future test-name proposals with uppercase letters need the same translation.

3. **partition=rows vs partition=blocks2d strip behaviour asymmetry**: partition_pairs sidecar (the input to inject_halo_strip_xfers) is populated ONLY by partition_blocks2d (sole insert site partition_blocks2d.rs:443). partition_rows.rs:312 forwards partition_pairs verbatim. So 05/distributed (partition=rows, cycle-124 task0304_05) is SOUND under the naive filter — no strips emitted. Only 05/distributed-2d needs the strip filter. This asymmetry is load-bearing for the cycle-124 task0304_* code path NOT carrying the filter (and being correct on its own narrow scope).

Correction to Implementation Notes item #2 (backtick-quoted segments lost to shell command substitution on the first edit; re-recording correctly): clippy "-D non_snake_case" (under just clippy) rejected the TASK-0310 brief literal test-name suffix "_y_AND_x". Renamed to "_y_and_x". Future test-name proposals with uppercase letters need the same translation.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0310 cycle-125 LANDED behaviour-layer narrative pins on 05-stencil/distributed-2d (halo=1 → band±1 on y AND x, host-broadcast Pushes filtered from inject_halo_strip_xfers strips) + 07-matmul/distributed (halo=0 → band equality, no extension). Closes cycle-124 architect P2.2 + P2.4 with no new P1/P2 follow-ons. Gate: 854/0/3 dev + release; e2e 108/92/0/16/0 × 2 samples non-flake. Forward-carry gotcha for future partition=blocks2d behaviour-pin work: 2D-blocks2d emits TWO Push classes (host-broadcast main + worker-to-worker halo strip); filter on x.src ∉ partition_worker_ranges[outer_iv].keys() for the main subset. Discovered empirically via a real test failure on the first run — narrative alone would have missed it. Mirrors the cycle-124 task0304_* idiom on the 1D shape sibling-sweep but adds the 2D filter (cycle-124 was sound on its narrow scope because partition_pairs is empty under partition=rows; partition_blocks2d is the sole partition_pairs insert site).
<!-- SECTION:FINAL_SUMMARY:END -->
