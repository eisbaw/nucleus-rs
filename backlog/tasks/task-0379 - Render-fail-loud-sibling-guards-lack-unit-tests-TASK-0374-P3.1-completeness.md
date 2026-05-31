---
id: TASK-0379
title: Render fail-loud sibling guards lack unit tests (TASK-0374 P3.1 completeness)
status: Done
assignee:
  - Mark Ruvald Pedersen
created_date: '2026-05-31 01:30'
updated_date: '2026-05-31 02:00'
labels:
  - backend
  - gather
  - test
  - rigour
  - completeness
dependencies:
  - TASK-0374
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
COMPLETENESS follow-up to TASK-0374 (architect P3.1, gather review gate). TASK-0374 unit-pinned the 4 fail-loud arms of render_gather_index_load. Three OTHER render fail-loud guards remain unit-untested defense-in-depth: (1) render_int_expr Call-in-index arm, expr.rs:72 (UnsupportedFeature kernel call inside an integer index); (2) render_const_expr DataRef/Call-in-loop-bound arm, expr.rs:201-203; (3) render_flat_index own three guards, fire.rs:520/530/539. All three are DEEPER than the lowering reject (lower_index_expr rejects Call at lower.rs:1179; allow_gather=false rejects DataRef/Call in loop-bound position), so they are hard to reach from valid source today and are legitimate defense-in-depth, NOT source-reachable like the partial-rank arm. Add unit tests asserting each EmitError fires, mirroring render_gather_negative.rs, OR document precisely why each is structurally unreachable. Lower urgency than TASK-0374 since none is currently surface-reachable.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle-219 hardening): add nucleus/backend-common/tests/render_guard_siblings.rs mirroring render_gather_negative.rs. 5 tests, one per defense-in-depth guard:
(1) render_int_expr IrExpr::Call -> UnsupportedFeature "kernel call inside an integer index" (expr.rs:72-74).
(2) render_const_expr IrExpr::DataRef -> UnsupportedFeature "data-ref / call inside a const expression (loop bound)" (expr.rs:201-203); bonus Call variant.
(3) render_flat_index empty-indices -> UnsupportedFeature "non-indexed reference" (fire.rs:519-522).
(4) render_flat_index rank>=2 missing-ResolvedType -> ContractGap "has no ResolvedType" (fire.rs:529-536). REACHABILITY NUANCE: must use >=2 indices (len==1 returns Ok early at :524), AND data NAME must be present in NameTables.data (data_name at :528 fires ContractGap "has no name" first if absent) while ResolvedType absent from sidecar.
(5) render_flat_index rank/shape mismatch -> UnsupportedFeature "rank/shape mismatch with index list" (fire.rs:538-544); e.g. 2 indices over rank-3 dims.
Bonus: positive control rank-2 row-major stride render. Honest docstring: these are SIBLINGS of the 4 TASK-0374 arms, mostly NOT source-reachable (lowering rejects Call-in-index + DataRef/Call in loop-bound; flat-index guards sit behind shape-valid callers) — UNLIKE the partial-rank arm which IS source-reachable. Tests pin guard BEHAVIOR against silent refactor-removal. Pure hardening; e2e 329/272/0/57/0 invariant.

ORCHESTRATOR REVIEW GATE (2026-05-31, independent, read-only): GO x2. qa-test-runner re-ran the gate: build clean; clippy exit 0 (forced non-cached re-lint, doc_lazy_continuation fix confirmed); test dev 1165/0/3 (render_guard_siblings = 8 passed); test-release 1164/0/3 (-1 = known TASK-0291 divergence); e2e 329/272/0/57/0 on two byte-identical runs. mped-architect empirically confirmed all 5 guard reachabilities (distinct error messages prove each test hits its INTENDED guard not a sibling; flat-index len==1 early-return + data_name-before-ResolvedType ordering verified) and confirmed the unreachability claims hold against lowering. P3 sibling-sweep gap (4 more fire.rs guards: :376 ArgBinding::Nested + :426/:438/:446 classify_data_slice) filed as TASK-0381.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-219. Added nucleus/backend-common/tests/render_guard_siblings.rs (8 tests, all pass dev+release) pinning the 5 defense-in-depth render guards that are SIBLINGS of the 4 TASK-0374 gather arms:
(1) render_int_expr IrExpr::Call -> UnsupportedFeature "kernel call ... index" (expr.rs:72-74).
(2) render_const_expr DataRef in loop-bound -> UnsupportedFeature "const expression ... loop bound" (expr.rs:201-203); + bonus Call half of the same arm.
(3) render_flat_index empty-indices -> UnsupportedFeature "non-indexed reference" (fire.rs:519-522).
(4) render_flat_index rank>=2 missing-ResolvedType -> ContractGap "no ResolvedType" (fire.rs:529-536).
(5) render_flat_index rank/shape mismatch (2 idx over rank-3) -> UnsupportedFeature "rank/shape mismatch" (fire.rs:538-544).
Bonus: positive control (rank-2 row-major stride render == "((y) * 4 + (x)) as usize") + missing-name ContractGap (fire.rs:31).
REACHABILITY (honest): all 5 are mostly NOT source-reachable (lowering rejects Call-in-index and DataRef/Call-in-loop-bound; flat-index guards sit behind shape-valid callers) — UNLIKE TASK-0374 partial-rank arm which IS source-reachable. Value = pin guard BEHAVIOUR against silent refactor-removal; docstring states this honestly.
GOTCHA confirmed empirically (matches carried-context B): to reach the missing-ResolvedType (fire.rs:529) and rank-mismatch (fire.rs:538) guards you MUST use >=2 indices (len==1 returns Ok early at :524) AND the data NAME must be present in NameTables (data_name at :528 ContractGaps first if absent). Got the ordering right; verified by the separate missing-name test firing the :31 guard.
RECURRING-TRAP HIT: clippy doc_lazy_continuation fired on the //! docstring (list immediately followed by prose at col 5); fixed by inserting a blank //! line to break the paragraph. Now clean under -D warnings.
GATE: build OK, clippy clean, just test 1165 passed/3 ignored (was 1157, +8), just test-release 1164 passed/3 ignored (was 1156, +8; dev/release delta=1 is the pre-existing debug_assert #[should_panic], TASK-0291), e2e 329/272/0/57/0 (unchanged HARD invariant). Pure hardening, no production behaviour change.
<!-- SECTION:FINAL_SUMMARY:END -->
