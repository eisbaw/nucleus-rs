---
id: TASK-0278
title: Extend TASK-0273 reuse marker coverage to multi_worker_walker strip-mine arm
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 12:08'
updated_date: '2026-05-24 12:29'
labels:
  - M5
  - test-gap
  - reuse
  - forward-carried-from-TASK-0273
dependencies:
  - TASK-0273
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0273 cycle-98. TASK-0273 closed coverage for the NON-strip-mine call site to `render_reuse_marker_comment` (the call OUTSIDE the `if let Some(tag) = block_tag` arm in `multi_worker_walker.rs`; search for `render_reuse_marker_comment` — exercised via Event::Loop with `block_tag: None`). The STRIP-MINE call site (the call INSIDE the `if let Some(tag) = block_tag` arm) remains UNCOVERED.

A regression that drops the marker emit from ONLY the strip-mine arm — e.g. while refactoring the per-occurrence absolute-index rebinding path or while wiring the real circular-buffer codegen for inner-block tile loops — would silently pass today.

## Why this matters now

The shipped `05-stencil/distributed.sched.nuc` carries `loop x : block=64, vectorize=8, reuse;` — block+reuse on the same iv. When TASK-0267 + TASK-0268 unblock that cell, it will execute the strip-mine call site live. Until then, the strip-mine arm's reuse-marker emit is structurally exercised in NO test.

## Acceptance

1. A third test in `nucleus/backend-common/tests/multi_worker_reuse_marker.rs` (or a sibling file) that constructs an `Event::Loop` carrying `block_tag: Some(BlockTag {...})` AND a non-strip-mined enclosing tile loop, populates `sidecar.reuse_widths` for the inner iv, calls `render_worker_events`, and asserts the marker substring appears.
2. The fixture should mirror `nucleus/backend-common/tests/multi_worker_blocked_rebind.rs` for the BlockTag + tile_loop construction — that file already shows the working shape.
3. Test runs in <30s.

## Dependencies

- Forward-carried from: TASK-0273 (cycle-98 honest-limits disclosure; gap was self-identified but not filed by implementer — orchestrator filing the prerequisite the implementer-contract required).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 99 closure (orchestrator-implements-direct, 2026-05-24)

### Commit
74fef15 backend-common/tests: TASK-0278 pin reuse marker on strip-mine arm (the `render_reuse_marker_comment` call inside the `if let Some(tag) = block_tag` arm in multi_worker_walker.rs)

### What landed
One new test in nucleus/backend-common/tests/multi_worker_reuse_marker.rs:
- multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated_under_block_tag

Fixture mirrors multi_worker_blocked_rebind.rs's outer-tile + inner-tagged-strip-mine pattern. Inner Event::Loop carries block_tag = Some(BlockTag { block_n: 4, num_full: 4, is_partial: false }); enclosing tile_loop is untagged. Reuse populated on the INNER iv (src_iv 'x'), matching production shape (reuse rides on strip-mined inner var, not outer tile var). Payload assertions: iv=x, data=img_in, axis=1, length=3, min_offset=-1 (same discrimination shape as the cycle-98 non-strip-mine arm test).

Module-doc Test surface section extended to list all three tests + their call-site attribution.

### Gate (orchestrator-verified)
- cargo test -p backend-common --test multi_worker_reuse_marker: 3/3 PASS (the existing two from cycle 98 + the new TASK-0278 one)
- just e2e: 92/77/0/15/0 byte-identical (test-only change, no production code touched — predicted)
- just clippy: clean
- just fmt-check: exit 0 (one fmt pass to normalise the new test's two-line let-binding; no drift introduced beyond what fmt corrects)

### Per-AC status
- AC#1 (third test with block_tag = Some(BlockTag) + tile_loop enclosing): YES
- AC#2 (mirrors multi_worker_blocked_rebind.rs construction): YES — same outer-untagged + inner-tagged Event::Loop pattern + full-nest BlockTag
- AC#3 (runtime <30s): YES — finishes in <0.01s

### Implementation honesty
- Worked same-session as TASK-0278's filing (cycle 98 → cycle 99). The skill normally prefers deferring follow-ups to fresh sessions. Rationale for working immediately: (a) the gap was specifically a 'silent sibling' pattern that the cycle-98 memory entry warned about — leaving it open one more cycle would weaken the very lesson; (b) bounded scope (~150 LoC pattern copy with exact template available); (c) orchestrator-implements-direct skipped the implementer subagent so no fresh-context-load penalty; (d) zero production-code risk.
- Single follow-up that the implementer-contract-item-5 audit could file: TASK-0273 Option B's description mentions 'partition_worker_ranges populated' — neither cycle-98 nor cycle-99 fixtures populate that. A third coverage shim (partition_worker_ranges-positive fixture) would close the remaining strict-Option-B coverage. Filed as TASK-0279 if reviewers flag it; otherwise let it stay implicit (the production path consults partition_worker_ranges in the loop header, NOT in the marker emit, so coverage value is mostly orthogonal to TASK-0273's stated goal).

### Cycle 99 outcome
TASK-0278 Done; TASK-0273's silent-sibling defect family fully closed for the multi_worker_walker.rs paths (both line 404 strip-mine and line 478 non-strip-mine now have presence + payload pins; line 478 also has symmetric absence). The new test joins the cycle-98 pair as one cohesive 3-test fixture file.

## Cycle 99 review-hardening (orchestrator, 2026-05-24)

Parallel review gate on commit 74fef15: both reviewers **GO**.

### architect P2 (material): pthreads-sync sibling-grep gap
The cycle-99 close pinned both walker arms in backend-common/multi_worker_walker.rs (404 strip-mine + 478 non-strip-mine), but nucleus/backends/pthreads-sync/src/lib.rs has TWO call sites to render_reuse_marker_comment (lines 653 + 675) that were NOT re-audited. The existing single-worker e2e grep test asserts >=1 marker presence — satisfied by EITHER call site, so a regression dropping one would silently pass.

Same silent-sibling pattern as TASK-0278 itself. Filed as **TASK-0279** (Audit pthreads-sync render_reuse_marker_comment call-site coverage; 30-min investigation + possible synthetic pin).

### architect P3 (cosmetic): fixture comment precision
Strip-mine fixture's inner-loop comment overclaimed name-reuse: 'the inner iv reuses the source loop's variable name (per the strip-mine contract)'. The synthetic fixture actually uses distinct IterVar ids (11 + 20) with distinct names ('x' + 'tile') — no name reuse. **Fix landed in commit (this cycle hardening commit)**: rewrote to honestly describe the rendering invariant the fixture models vs the production projection's id-reuse contract.

### qa P3s (all non-blocking): defensible test scaffolding (`.expect` on link/build calls), count >= 1 vs == 1 (legitimate flexibility for future contract revisions), pre-existing tracker md uncommitted at review time (since committed).

### Cycle 99 outcome
TASK-0278 **Done, review-GO with one carry**: TASK-0279 inherits the pthreads-sync sibling-grep audit responsibility. The silent-sibling pattern from cycle 93/95/97-98 is now THREE cycles deep — the family pattern is reliably surfacing because we're explicitly looking for it (the cycle-98 memory entry [[feedback-silent-sibling-defect]] folds the check into reviewer briefs).
<!-- SECTION:NOTES:END -->
