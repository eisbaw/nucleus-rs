---
id: TASK-0279
title: >-
  Audit pthreads-sync render_reuse_marker_comment call-site coverage
  (sibling-grep follow-up to TASK-0278)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 12:28'
updated_date: '2026-05-24 12:44'
labels:
  - M5
  - test-gap
  - reuse
  - sibling-grep-audit
  - forward-carried-from-TASK-0278
dependencies:
  - TASK-0278
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0278 cycle-99 architect review. The cycle-93/95/97/98 silent-sibling defect family teaches: always grep ALL peer call sites and pin each. TASK-0273 + TASK-0278 closed both walker arms in `backend-common/multi_worker_walker.rs` (lines 404 strip-mine + 478 non-strip-mine), but `nucleus/backends/pthreads-sync/src/lib.rs` has TWO call sites to `render_reuse_marker_comment` (lines 653 + 675) that were NOT re-audited this cycle.

The existing single-worker grep test `nucleus/nucleus-compiler/tests/e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive` asserts `>=1` marker occurrence. That is satisfied by EITHER call site emitting — a regression dropping the marker from ONLY one of the two call sites would silently pass.

## Acceptance

1. Investigate: which call site (line 653 vs line 675) emits the marker for the shipped `05-stencil/reuse.sched.nuc`? `NUC_TRACE=1` or `println!` instrumentation may help — or static reading suffices.
2. If both arms can fire on different schedule shapes (e.g. one for strip-mine, one for non-strip-mine, analogous to multi_worker_walker), add a second e2e test or synthetic fixture pinning the under-covered arm. If one is structurally dead, document it + consider removing.
3. Run `just e2e` + `just test` post-fix — no regressions.

## Honest scope

This is a 30-minute investigation followed by either a small test addition or a documentation update. Most likely the two call sites differentiate between the strip-mine and non-strip-mine arms (mirroring multi_worker_walker.rs's structure) — in which case the existing single-worker e2e test exercises ONE arm (whichever the shipped reuse.sched.nuc routes through) and the other arm needs a synthetic pin.

## Dependencies

- Forward-carried from: TASK-0278 cycle-99 architect review (silent-sibling family closure).
- Related: TASK-0273 (the original walker arm), TASK-0278 (strip-mine arm closure).
- Pattern: same as the cycle-95 `UnknownIterVarInScope` rename sweep — grep ALL peer sites + verify each is covered.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 100 closure (orchestrator-implements-direct, 2026-05-24)

### Investigation result
Confirmed: pthreads-sync/src/lib.rs has structurally identical strip-mine vs non-strip-mine arms as multi_worker_walker.rs:
- Line 653: marker call inside the strip-mine arm (after the rebound tile-loop header at lines 640-648), guarded by Some(block_tag).
- Line 675: marker call inside the non-strip-mine arm (regular loop header at line 667).

Shipped 05-stencil/reuse.sched.nuc carries 'loop x : reuse;' with NO block= on either iv — routes EXCLUSIVELY through line 675. The existing e2e_example_05.rs grep test ('reuse_marker_present_on_reuse_schedule_absent_on_naive') asserts >=1 marker presence, so it covers line 675 but proves nothing about line 653.

The shipped strip-mine-WITH-reuse cell (05-stencil/distributed.sched.nuc with 'loop x : block=64, vectorize=8, reuse;') is [[skip]]ped across every backend pending TASK-0267 + TASK-0268. Until then, line 653 had ZERO coverage.

### Commit
Latest commit on master: pthreads-sync/tests: TASK-0279 pin reuse marker on strip-mine arm (lib.rs:653) — single new test file nucleus/backends/pthreads-sync/tests/reuse_marker.rs (~180 LoC).

### Fixture
Mirrors cycle-99 multi_worker_reuse_marker.rs strip-mine pin exactly: outer untagged tile_loop enclosing inner Event::Loop carrying BlockTag {block_n=4, num_full=4, is_partial=false}; reuse populated on the INNER iv; payload assertions cover iv/data/axis/length/min_offset.

Uses pthreads-sync's public render_single_worker_main entry point (no need to expose render_event privately).

### Gate (orchestrator-verified)
- cargo test -p pthreads-sync --test reuse_marker: 1/1 PASS
- just e2e: 92/77/0/15/0 byte-identical
- just clippy: clean
- just fmt-check: exit 0

### Per-AC status
- AC#1 (investigation): YES — both call sites confirmed structurally distinct; line 675 covered by existing e2e; line 653 was uncovered.
- AC#2 (synthetic pin OR documentation): YES — synthetic pin landed in tests/reuse_marker.rs.
- AC#3 (no regressions): YES — test-only, e2e byte-identical.

### Silent-sibling family CLOSED for render_reuse_marker_comment
After this cycle, ALL FOUR production call sites have presence pins:
- multi_worker_walker.rs strip-mine arm (the `render_reuse_marker_comment` call INSIDE the `if let Some(tag) = block_tag` branch; cycle 99, TASK-0278)
- multi_worker_walker.rs non-strip-mine arm (the `render_reuse_marker_comment` call OUTSIDE the `if let Some(tag) = block_tag` branch; cycle 98, TASK-0273)
- pthreads-sync/src/lib.rs:653 (THIS cycle, TASK-0279)
- pthreads-sync/src/lib.rs:675 (existing e2e_example_05.rs grep test)

### Honest scope / known limits
- The new test, like its cycle-99 sibling, uses synthetic ivs with distinct ids (not actual production name-reuse). Same caveat as cycle 99 — the test models the rendering invariant, not the projection's id-reuse contract.
- The forward-carry warning in the module doc is identical to multi_worker_reuse_marker.rs's: when TASK-0269 lands real circular-buffer codegen, the 'reuse_widths_pending' substring will be renamed/subsumed; update BOTH test files in lockstep.

### Cycle 100 outcome
TASK-0279 Done; the cycle-93/95/97/98/99/100 silent-sibling chain for render_reuse_marker_comment is now end-to-end closed. The chain length itself (6 cycles to fully close 4 call sites) is the durable lesson the [[feedback-silent-sibling-defect]] memory entry was created to surface — the next implementer touching a shared helper across N call sites should grep-and-pin all N in the first cycle, not discover them one at a time across N cycles.
<!-- SECTION:NOTES:END -->
