---
id: TASK-0239
title: >-
  Extract shared multi-worker event-walker between pthreads-sync and
  pthreads-async (TASK-0228 Wave B-2 de-dup follow-up)
status: To Do
assignee: []
created_date: '2026-05-22 07:35'
labels:
  - tech-debt
  - M4
  - backend
dependencies:
  - TASK-0228
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 26 (TASK-0228 Wave B-2) duplicated pthreads-sync's render_worker_events + render_wait_assign + leading_axis_slice + collect_pre_init_sets into pthreads-async/src/multi_worker.rs (~400 LoC duplicate). The two implementations differ ONLY in 'slot' vs 'ring' variable prefix and the file-scope Slot<T> vs Ring<T> struct emission; everything else (Fire/Loop/Sync/Wait/check_frame, barrier identity, pre-init computation, slice-paste gather, partition_worker_ranges per-worker bounds) is byte-for-byte the same emit-string shape.\n\nThe right architectural move is to lift the shared walker into a parameterized helper (in pthreads-sync's pub surface, OR in a new backend-common crate). The parameter is the rendezvous-primitive shape: (struct-emit fn, instance-emit fn, callsite var-prefix). Then pthreads-async's Plan::emit becomes ~80 LoC of orchestration plus the substrate calls.\n\nThis was NOT done in cycle 26 because the precedent in this codebase is 'duplicate first, then extract once N>=3 sites exist' (cf TASK-0222 which did exactly that for the four check_frame emit-string templates after pthreads-sync + mp-tcp-bufsync both exhibited them). pthreads-async is now the second site for the worker-events walker; the extraction is justified, but it is its own cycle.\n\nA drift test (codegen-output-equal-modulo-substitution between the two backends on a real fixture) would be the right defense, OR the extraction itself which eliminates the duplication structurally.
<!-- SECTION:DESCRIPTION:END -->
