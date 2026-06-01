---
id: TASK-0401
title: >-
  Investigate/pin PartitionBlocks2dError::InnerRepeatNotFound (residual
  white-box arm from TASK-0400 sweep)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-01 04:36'
updated_date: '2026-06-01 04:56'
labels:
  - tests
  - partition
  - blocks2d
  - silent-sibling
  - defensively-unreachable
  - prove-the-check-bites
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0400 review P2 (architect, commit 9a4727d gate). The 3-pass partition guard sibling sweep in TASK-0400 left ONE variant unpinned: PartitionBlocks2dError::InnerRepeatNotFound (defined partition_blocks2d.rs:194, returned at :337). It is the blocks2d analog of the white-box defensive arms TASK-0400 pinned in the other passes, so the 3-pass-sweep framing slightly over-claimed completeness. ARCHITECT STATIC ANALYSIS (verify empirically per feedback cheap-empirical-verification): the arm appears GENUINELY UNREACHABLE even white-box -- has_inner_repeat = contains_repeat(body) (the NotOuterOf2DNest gate at rows.rs:382-equivalent) and find_first_inner_repeat -> first_repeat_in(body) (:337) use the IDENTICAL descend rule (descend through Sequence, stop at any Repeat), so whenever has_inner_repeat==true (NotOuterOf2DNest passes) find_first_inner_repeat necessarily returns Some -> InnerRepeatNotFound cannot be reached. If confirmed, a bite test is IMPOSSIBLE to construct; the correct action is to mark/document it dead (mirror the audit-flagged BlockTransformError::NotDivisible #[allow(dead_code)] + TASK-0397 defensively-unreachable-guard precedent) rather than force a test. If a subtle reachable path EXISTS (the two rules diverge on some shape), add a white-box bite test mirroring partition_workers unknown_loop_var_when_no_repeat_carries_the_resolved_iter_var. DELIVERABLE: either (a) a white-box bite test if reachable, or (b) a doc note + (if truly dead) an #[allow(dead_code)]-equivalent annotation citing this task, with the reachability reasoning recorded. LOW; test/doc-only; mirrors TASK-0397.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Empirically determine whether InnerRepeatNotFound (:337) is reachable: attempt to construct a LinkedIR/ACFG where has_inner_repeat==true but find_first_inner_repeat==None; record the result
- [x] #2 If reachable: white-box bite test asserting Err(InnerRepeatNotFound). If genuinely unreachable: doc note at the return site citing this task + the identical-descend-rule reasoning (and dead-code annotation if warranted)
- [x] #3 Gate: just clippy + just test green; e2e baseline 385/328/0/57/0 unchanged (test/doc-only)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED in-thread (orchestrator; comment+test-only). REACHABILITY DETERMINED (per cycle-235 discipline: tried to CONSTRUCT a reaching input): InnerRepeatNotFound (:337) is GENUINELY UNREACHABLE by construction, NOT just unhit. Proof: the pass reaches the None branch only if has_inner_repeat==true (NotOuterOf2DNest gate passed) AND first_repeat_in(body)==None; but has_inner_repeat IS contains_repeat(body) (find_outer_of_2d, partition_rows.rs:382), and contains_repeat (partition_rows.rs:437-443) and first_repeat_in (partition_blocks2d.rs:521+) are the SAME predicate over the SAME outer body (Repeat=>yes; Sequence=>any/first child; Operation/Sync/Xfer=>no). So contains_repeat(body) == first_repeat_in(body).is_some() => no input satisfies contains&&!first_repeat => a bite test is IMPOSSIBLE (proven, not failure-to-find). ARTIFACTS (AC#2 path b, genuinely-unreachable): (1) doc note at the :337 return site explaining unreachable-by-construction + naming the pinning test (grep-locator, no line numbers per TASK-0391); (2) white-box invariant test inner_repeat_not_found_unreachable_contains_repeat_iff_first_repeat_in pinning contains_repeat(n)==first_repeat_in(n).is_some() over 6 body shapes (both polarities exercised) -- if a future edit diverges the two descents, InnerRepeatNotFound may become live and the test BITES. BITE PROVEN: temporarily made first_repeat_in return None on Sequence -> test FAILED on shape #4 (Sequence-wrapped Repeat: contains_repeat=true, first_repeat_in=None); reverted clean. GATE (orchestrator-run, transparent): clippy clean; test dev 1230/0 (+1); test-release 1229/0 (-1 known TASK-0291); e2e 385/328/0/57/0 UNCHANGED (comment+test-only). Architect read-only review for independent check on the unreachability reasoning (the highest-risk comment-doc-lie kind) pending before Done.
<!-- SECTION:NOTES:END -->
