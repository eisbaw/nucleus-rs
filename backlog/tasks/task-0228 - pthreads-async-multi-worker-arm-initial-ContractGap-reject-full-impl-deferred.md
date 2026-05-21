---
id: TASK-0228
title: >-
  pthreads-async multi-worker arm (initial: ContractGap-reject; full impl
  deferred)
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
labels:
  - M4
  - backend
  - multi-worker
dependencies:
  - TASK-0226
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After TASK-0226 lands single-worker, the multi-worker path needs a defined behaviour even before its full implementation. Mirror the pattern pthreads-sync established (multi_worker.rs Plan::emit): if used_workers.len() >= 2 the arm runs.

INITIAL behaviour (this task): reject with EmitError::ContractGap('pthreads-async: multi-worker pipelined arm not yet implemented (see TASK-0228.01)'). This makes the single-worker arm shippable + the multi-worker shape decidable + the failure mode HONEST.

FULL implementation deferred to TASK-0228.01 (filed once TASK-0226 + this task land): per-fan-out-pair (DataId, SeqTag) ring sized N=buffer; the same SHARED static + Drop guard pattern from pthreads-sync multi_worker.rs for check_frame; partition=workers + pipeline=D projects per-pair rings — see TASK-0216 forward-carry.

Read the TASK-0052.05 forward-carry on TASK-0042.01 for the multi-worker check_frame contract; the same panic=abort SIGABRT gotcha applies (Cargo.toml profile.release panic="abort" -> worker thread panic -> whole-process SIGABRT not exit-101).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 pthreads-async emit() rejects multi-worker (used_workers >= 2) with ContractGap pointing at TASK-0228.01.
- [ ] #2 Tests exist for the multi-worker rejection path (no false-pass risk).
- [ ] #3 TASK-0228.01 is filed (the actual multi-worker arm) and references the forward-carries listed above.
<!-- AC:END -->
