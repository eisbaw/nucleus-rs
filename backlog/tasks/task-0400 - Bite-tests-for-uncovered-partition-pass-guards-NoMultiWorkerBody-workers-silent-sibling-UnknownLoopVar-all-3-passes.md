---
id: TASK-0400
title: >-
  Bite tests for uncovered partition-pass guards: NoMultiWorkerBody (workers
  silent-sibling) + UnknownLoopVar (all 3 passes)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-01 04:14'
updated_date: '2026-06-01 04:25'
labels:
  - tests
  - partition
  - silent-sibling
  - prove-the-check-bites
  - hardening
  - panic-not-diagnostic
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
ENDGAME HARDENING (prove-the-check-bites sweep). Empirically-verified gaps (a read-only audit subagent over-reported ~11/13 false positives by missing in-file mod tests; these survived per-claim verification of the return-Err sites + test references). TWO real silent-sibling clusters across the three partition passes: (1) PartitionError::NoMultiWorkerBody (partition_workers.rs:239) has NO bite test, while the structurally-identical sibling guards partition_rows::NoMultiWorkerBody (tests/partition_rows.rs:250) and partition_blocks2d::NoMultiWorkerBody (tests/partition_blocks2d.rs:376) ARE tested -- inverted silent-sibling (the original pass lacks what its siblings have). SURFACE-REACHABLE: a partition=workers directive on a loop whose body has <2 workers is a valid-but-meaningless schedule the linker passes through. (2) UnknownLoopVar is defined + returned in ALL THREE passes (workers:229/235, rows:259/264, blocks2d:315/321) and is UNCOVERED in all three (only block_transform::UnknownLoopVar tests/block_transform.rs:428 and reuse_inference::UnknownLoopVar sidecar_reuse.rs:381 have bite tests). WHITE-BOX defensive guard (the linker pre-rejects an unknown loop var; reachable only via a hand-built inconsistent LinkedIR/ACFG pair -- mirrors the TASK-0397 white-box-pin pattern + sidecar_reuse.rs:368). FIX: add pass-level bite tests mirroring insufficient_work_range_is_rejected (tests/partition_workers.rs:390) -- build a LinkedIR with a partition directive + an ACFG that trips the guard, assert the typed variant. Scope: partition_workers NoMultiWorkerBody (1) + UnknownLoopVar both return sites name-miss@229 and find_loop-miss@235 (2); partition_rows + partition_blocks2d UnknownLoopVar name-miss (1 each). Test-only, e2e-inert (baseline 385/328/0/57/0).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 tests/partition_workers.rs gains a bite test for PartitionError::NoMultiWorkerBody (partition=workers directive + single-worker-body Repeat -> Err(NoMultiWorkerBody{var,workers:1})), closing the inverted silent-sibling gap vs partition_rows/blocks2d
- [ ] #2 tests/partition_workers.rs gains bite tests for BOTH UnknownLoopVar return sites: name_iter_vars-miss (:229) and find_loop-miss (:235); documented as white-box invariant pins (linker pre-rejects on the surface path)
- [ ] #3 tests/partition_rows.rs and tests/partition_blocks2d.rs each gain an UnknownLoopVar bite test (name-miss path), completing the 3-pass sibling sweep so no structurally-identical guard is left untested
- [ ] #4 Gate green inside nix develop: build && clippy (no doc_lazy_continuation) && test && test-release && e2e; e2e baseline 385/328/0/57/0 UNCHANGED (test-only); all new tests pass
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED in-thread (orchestrator; test-only, per feedback-spawned-agents-refuse-code-edits). 5 bite tests added across the 3 partition-pass integration test files, mirroring insufficient_work_range_is_rejected (build LinkedIR with a partition directive + an ACFG that trips the guard, assert the typed variant via expect_err + match). NEW HELPER linked_with_workers_directive in tests/partition_workers.rs (mirrors partition_rows linked_with_rows_directive). Tests: partition_workers single_worker_body_is_rejected_no_multi_worker_body (NoMultiWorkerBody, surface-reachable, closes inverted silent-sibling); unknown_loop_var_when_directive_var_absent_from_name_iter_vars (UnknownLoopVar site-1 name-miss, white-box); unknown_loop_var_when_no_repeat_carries_the_resolved_iter_var (UnknownLoopVar site-2 find_loop-miss, white-box, distinct guard); partition_rows + partition_blocks2d negative_unknown_loop_var_when_directive_var_absent (UnknownLoopVar name-miss each, white-box). All inherently BITE (expect_err: remove the guard -> no Err -> test panics). META-FINDING (durable, recorded to memory): the read-only audit subagent that surfaced these over-reported ~11/13 false positives by failing to inspect in-file mod tests despite explicit instruction -- every claim was per-verified at the return-Err site + test-reference level before filing; the reuse_inference/halo_inference/partition_rows variants it flagged were ALREADY covered in-file. GATE (orchestrator-run): clippy -D warnings clean (no doc_lazy_continuation); test dev 1229/0 (was 1224, +5); test-release 1228/0 (-1 known TASK-0291); e2e 385/328/0/57/0 UNCHANGED (test-only). Holding for parallel read-only review gate before Done.
<!-- SECTION:NOTES:END -->
