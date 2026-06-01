---
id: TASK-0401
title: >-
  Investigate/pin PartitionBlocks2dError::InnerRepeatNotFound (residual
  white-box arm from TASK-0400 sweep)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 04:36'
updated_date: '2026-06-01 04:59'
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

REVIEW: architect read-only GO (independent). Verified the unreachability claim TRUE by structural induction over all 5 ACFGNode variants (Repeat/Sequence/Operation/Sync/Xfer, exhaustive no-wildcard match) + REPRODUCED the bite mutation (first_repeat_in->None on Sequence fails the test at shape #4, confirming the orchestrator claim exactly; shape #3 bare-Repeat short-circuits before the Sequence arm so does not bite, also as expected). No counterexample input exists. find_outer_of_2d and find_first_inner_repeat have byte-identical outer-descent so they land on the SAME outer body even if an iter_var were reused; block_tag is ..-ignored by both; single production call site, no silent sibling. Doc note precise, no doc-lie, grep-locator per TASK-0391. P3 (architect, backlog-note-only, NOT source): the impl-notes cite the return site as :337 (parent commit line); this commit shifted it to :355 (stamp-twice-when-narrative-shifts-line pattern). CORRECTED CITATIONS for the record: InnerRepeatNotFound return site = partition_blocks2d.rs:355 (post-commit); first_repeat_in defined ~:539, its call from find_first_inner_repeat ~:518. Harmless (in-source note uses the grep-locatable test name, not a line). qa arm: orchestrator self-ran the full deterministic gate transparently (comment+test-only, not flaky-sensitive).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Resolved the TASK-0400 review P2 residual. DETERMINED (cycle-235 discipline: tried to construct a reaching input) that PartitionBlocks2dError::InnerRepeatNotFound is GENUINELY UNREACHABLE by construction: has_inner_repeat = contains_repeat(body) (find_outer_of_2d) and find_first_inner_repeat runs first_repeat_in(body) on the same outer body; contains_repeat and first_repeat_in are the identical predicate, so contains_repeat(body)==first_repeat_in(body).is_some() and no input satisfies contains&&!first_repeat -> a bite test is IMPOSSIBLE (proven). ARTIFACTS (commit 5d506a0): (1) doc note at the return site (partition_blocks2d.rs:355) explaining unreachable-by-construction + naming the pinning test via grep-locator; (2) white-box invariant test inner_repeat_not_found_unreachable_contains_repeat_iff_first_repeat_in pinning the predicate equivalence over 6 body shapes (both polarities). BITE PROVEN by orchestrator AND independently re-verified by architect (diverging first_repeat_in fails shape #4). Architect GO (unreachability claim verified by structural induction, no counterexample). GATE: clippy clean; test 1230/0; test-release 1229/0 (-1 known TASK-0291); e2e 385/328/0/57/0 UNCHANGED (comment+test-only). Completes the partition-pass guard sweep: all variants across partition_workers/rows/blocks2d are now bite-tested (reachable) or invariant-pinned (provably dead).
<!-- SECTION:FINAL_SUMMARY:END -->
