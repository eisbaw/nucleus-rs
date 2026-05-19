---
id: TASK-0040
title: 'Example 6: 5x5 separable filter — algorithm + naive + blocked + reference'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 01:32'
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
- [x] #1 examples/06-separable-filter/ has algo + schedules + kernels.rs + reference + binaries.
- [x] #2 Algorithm has two sequential loops; intermediate array is single-assignment within scope.
- [x] #3 Test: passes M3 differential matrix on both M3 backends.
- [x] #4 Implementation notes record design questions (e.g. should the intermediate buffer be hinted to live in fast memory; deferred to schedule's place_data).
- [x] #5 Implementation notes record honest limitations (clamp boundaries; no reuse-with-shift; integer-typed).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Mirror 04/05 structure. 5x5 separable = horizontal 1x5 blur -> single-assignment intermediate tmp -> vertical 5x1 blur -> out.
2. Encode under v2 constraints (carried-context from 0039): clamp/boundary in the Rust KERNEL (algorithm cannot index out of range, no conditionals); use the proven rectangular-accumulate pattern (tmp[y][x] <-- hblur_acc over k:0..W; out[y][x] <-- vblur_acc over m:0..H). DISTINCT loop-var names per pass (reused name w/ different bounds is rejected, TASK-0171; reused name even w/ same bounds risks the TASK-0180 blocked double-count).
3. tmp is single-assignment within scope (one statement assigns tmp, one assigns out) — satisfies AC#2.
4. std-only reference/: computes it a DIFFERENT way (explicit clamped two-pass with real tap loops) + generates input.bin (no python).
5. naive + blocked schedules. Blocked: per TASK-0180, blocking a loop-var reused across passes double-counts; here tmp/out are NOT accumulators across the blocked axis IF I block an axis used in only ONE pass — but hy/vy differ per pass so a single `loop : block=` only tiles one pass. Probe blocked; if it double-counts (accumulator over k/m), ship+skip honestly w/ TASK-0180 ref like 04. Determine empirically, do not assume.
6. e2e_example_06.rs; add to runnable_examples + required naive both backends + blocked per probe result.
7. Full gate (test/clippy/e2e x3/determinism); no regression to 24 cells.
8. AC#4 (fast-memory hint deferred to schedule place_data) + AC#5 (clamp, no reuse-with-shift, integer) in notes. Commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
GATE (all green, inside nix develop):
- cargo test --workspace: 0 failed; e2e_example_06 naive+blocked BOTH PASS (active, not ignored).
- cargo clippy --workspace -D warnings: clean.
- just e2e: 28 total / 22 pass / 0 fail / 6 skipped / 0 required-fail. 3x non-flaky (incl. mp-tcp). Matrix grew 24->28.
- determinism-check: byte-identical 28/28. negative: correctly bites. Pre-existing 24 cells (incl. example 04) unchanged.
NEW DIFFERENTIALLY-GREEN CELLS (byte-identical to independent reference.bin under BOTH backends): 06-separable-filter/{naive,blocked} x {pthreads-sync, mp-tcp-bufsync} = 4 cells.
AC#2: tmp is single-assignment within scope (one statement Pass1 produces it, one statement Pass2 consumes it).
AC#4 (design Q — fast memory for tmp): answered by DEFERRAL to schedule place_data (PRD 6.3); algorithm says only WHAT. Recorded as the deliberate algorithm/schedule split, not an omission.
AC#5 (honest limitations): clamp-to-edge boundary only (no mirror/wrap); NO reuse-with-shift (rectangular accumulator recomputes all W/H taps per pixel, O(W)/O(H) not O(1)); box SUM not average (no divide, avoids rounding drift); integer-only wrapping_add.
KEY CROSS-EXAMPLE FINDING: 06/blocked is a POSITIVE CONTROL for TASK-0180. By giving each pass a DISTINCT outer-row var (hy vs vy), each tiled inner loop occurs exactly once in the EventList, the divisible_inner count==1 guard is satisfied, absolute-index rebinding IS applied ((0+hy__tile*4+hy)), and blocked is bit-identical to naive/reference. This confirms TASK-0180 root cause = reused loop-var NAME, not blocking-an-accumulator per se. Also hit & noted TASK-0171: a reused loop-var name with DIFFERENT bounds is rejected outright (drove the distinct-name design).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added example 06-separable-filter (5x5 separable box sum) extending the cross-backend differential.

WHAT: New driving example nuc-nucleus/examples/06-separable-filter/ — algo, kernels, naive+blocked schedules, a std-only independent reference oracle that also generates the fixtures (no python), input.bin/reference.bin, README; plus in-crate e2e_example_06.rs (both schedules active) and e2e-matrix wiring (runnable_examples + 4 required cells for naive AND blocked on both backends).

DESIGN: horizontal 1x5 pass writes a single-assignment intermediate tmp; vertical 5x1 pass consumes tmp into out — the producer→consumer intermediate-lifetime stress (AC#2). As in example 04, the shifted-tap boundary is not expressible in v2 (usize underflow, no conditional — TASK-0179), so the algorithm uses only the proven rectangular reduction-accumulator and the clamp-to-edge tap selection lives in the Rust kernels (the intended PRD 6.2.2 division of labour). Box sum, not average (no divide — avoids rounding drift). Integer-typed, wrapping_add.

DIFFERENTIAL: naive AND blocked are both byte-identical to the independent reference.bin (a deliberately different control structure: explicit clamped tap loops) under BOTH pthreads-sync and mp-tcp-bufsync. Matrix grew from 24 to 28 cells; 22 pass / 0 fail / 0 required-fail; verified 3x non-flaky; determinism byte-identical and the negative arm bites.

KEY CROSS-EXAMPLE FINDING: 06/blocked is a POSITIVE CONTROL for TASK-0180. Giving each pass a distinct outer-row loop variable (hy vs vy) makes each tiled inner loop occur exactly once in the EventList, so the backend divisible_inner count==1 guard is satisfied, absolute-index rebinding IS applied, and blocked is bit-identical to naive — confirming TASK-0180's root cause is the reused loop-var NAME (04-prefix-sum), not blocking-an-accumulator per se. Also hit TASK-0171 (a reused loop-var name with different bounds is rejected outright), which drove the distinct-name design.

AC#4 (should tmp live in fast memory): answered by deferral — that is a schedule place_data concern (PRD 6.3), not an algorithm one; recorded as the deliberate algorithm/schedule split.

AC#5 honest limitations: clamp-to-edge boundary only (no mirror/wrap/zero); no reuse-with-shift (O(W)/O(H) per pixel, not O(1)); box sum not average; integer-only.

TESTS: cargo test --workspace 0 failed; clippy -D warnings clean; just e2e 28/22-pass/0-required-fail (3x); determinism-check byte-identical; negative bites. No regression to the pre-existing 24 cells.
<!-- SECTION:FINAL_SUMMARY:END -->
