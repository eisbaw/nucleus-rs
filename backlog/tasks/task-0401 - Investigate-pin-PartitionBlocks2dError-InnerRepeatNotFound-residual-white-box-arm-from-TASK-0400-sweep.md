---
id: TASK-0401
title: >-
  Investigate/pin PartitionBlocks2dError::InnerRepeatNotFound (residual
  white-box arm from TASK-0400 sweep)
status: To Do
assignee: []
created_date: '2026-06-01 04:36'
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
- [ ] #1 Empirically determine whether InnerRepeatNotFound (:337) is reachable: attempt to construct a LinkedIR/ACFG where has_inner_repeat==true but find_first_inner_repeat==None; record the result
- [ ] #2 If reachable: white-box bite test asserting Err(InnerRepeatNotFound). If genuinely unreachable: doc note at the return site citing this task + the identical-descend-rule reasoning (and dead-code annotation if warranted)
- [ ] #3 Gate: just clippy + just test green; e2e baseline 385/328/0/57/0 unchanged (test/doc-only)
<!-- AC:END -->
