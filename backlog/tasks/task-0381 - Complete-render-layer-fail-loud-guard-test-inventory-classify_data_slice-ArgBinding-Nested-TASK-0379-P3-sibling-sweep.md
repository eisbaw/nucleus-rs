---
id: TASK-0381
title: >-
  Complete render-layer fail-loud guard test inventory: classify_data_slice +
  ArgBinding::Nested (TASK-0379 P3 sibling sweep)
status: Done
assignee:
  - '@me'
created_date: '2026-05-31 01:59'
updated_date: '2026-05-31 03:48'
labels:
  - backend
  - test
  - rigour
  - completeness
  - silent-sibling
dependencies:
  - TASK-0379
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
COMPLETENESS follow-up to TASK-0379 (architect P3, review gate). TASK-0379 + render_gather_negative.rs (TASK-0374) cover render_int_expr, render_const_expr, render_flat_index. FOUR render-layer fail-loud guards remain unit-untested, all in fire.rs and structurally sibling to ones already pinned: (1) fire.rs:376 ArgBinding::Nested -> UnsupportedFeature nested kernel call inside an argument expression (direct sibling of expr.rs:72 Call-in-index, the most glaring omission); (2) fire.rs:426 classify_data_slice missing-ResolvedType -> ContractGap (sibling of fire.rs:529); (3) fire.rs:438 classify_data_slice over-indexed -> UnsupportedFeature (sibling of fire.rs:539 rank-mismatch); (4) fire.rs:446 classify_data_slice scalar-data-indexed -> UnsupportedFeature. (fire.rs:346 no_std fixed-array mismatch is already covered by fire_args_nostd.rs:200 - NOT a gap.) Add unit tests mirroring render_guard_siblings.rs OR document precise unreachability. LOW urgency: all defense-in-depth, none source-reachable. Filing per silent-sibling discipline so the render-layer fail-loud inventory is genuinely complete.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Extend nucleus/backend-common/tests/render_guard_siblings.rs with the 4 remaining render-layer fail-loud guards (TASK-0379 P3 sibling sweep): (1) fire.rs:376 ArgBinding::Nested via pub render_fire_args; (2) fire.rs:427 classify missing-ResolvedType via pub render_fire_output_assign; (3) fire.rs:438 classify over-indexed. (4) fire.rs:446 scalar-data-indexed is STRUCTURALLY MASKED by the over-indexed guard at :435 (any indices>=1 over dims=[] hits :435 first; indices==0 trips the debug_assert at :421) -> document via a masking test, not a flaky/panicking direct test. Update module docstring honestly. Gate: just test + clippy + check-mega-files + check-doc-* fences (test-only, no codegen/e2e impact).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE cycle-221. Added 4 render-layer fail-loud guard tests to render_guard_siblings.rs (commits 1844aeb + a0fc1d0):
- fire.rs:376 ArgBinding::Nested -> UnsupportedFeature (via render_fire_args)
- fire.rs:427 classify_data_slice missing-ResolvedType -> ContractGap (via render_fire_output_assign)
- fire.rs:438 classify_data_slice over-indexed -> UnsupportedFeature
- fire.rs:446 scalar-data-indexed: STRUCTURALLY MASKED by the over-indexed check at fire.rs:435 (any indices>=1 over dims=[] hits :435 first; indices==0 trips the debug_assert at :421 + the caller contract, enforced in BOTH dev and release via the whole-array if-guard at all 4 production callers). Pinned via a masking test (observable routing = over-indexed, not scalar-data), not a flaky direct test.

GOTCHAS / lessons hit this cycle:
1. clippy::doc_lazy_continuation (recurring) fired 11x on the module docstring: a paragraph immediately following a wrapped list item with no blank //! separator. Fix: blank //! line between list and following paragraph. Always re-run just clippy after editing /// or //! blocks.
2. comment-doc-lie (recurring, architect P2 NO-GO->GO): my first draft claimed the Nested guard is NOT source-reachable ("lowering flattens/rejects"). FALSE - acfg/build.rs:276 lowers a nested IrExpr::Call faithfully to ArgBinding::Nested; fire.rs:376 is the SOLE rejection site; ex14 hand-splits denoise(mix2()) to avoid it. The Nested arm is the MOST source-reachable guard in the file. Verify reachability MECHANISM against lowering before asserting it.

Review gate: architect read-only GO (after the P2 doc-lie fix). qa mechanical gate self-run in-thread (test-only additive change; e2e/determinism/xbackend arms invariant - not re-run, honestly noted): test 1165->1169 (+4) dev, clippy clean, check-mega-files/check-doc-citation-staleness/check-doc-links/check-narrative-doc-lie all OK.
<!-- SECTION:NOTES:END -->
