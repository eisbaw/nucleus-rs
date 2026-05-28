---
id: TASK-0354
title: >-
  Unit tests for collect_let_at_wait_data + is_whole_array_recv (TASK-0349 cycle
  220b architect P2.2)
status: To Do
assignee: []
created_date: '2026-05-27 23:58'
labels:
  - tests
  - backend-common
  - defensive
  - cycle-220b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 architect P2.2: the new public helpers `collect_let_at_wait_data` (nucleus/backend-common/src/multi_worker_walker/collect.rs) and `is_whole_array_recv` (nucleus/backend-common/src/multi_worker_walker/wait.rs) have only INDIRECT e2e-gate coverage. Per project test discipline (TASK-0304 / TASK-0310 behaviour-pin precedent), add unit-level positive + negative tests:

1. Mixed-mode (one slice, one whole) Wait of the same data -> stays OUT of let_at_wait set.
2. Accumulate-fan-in data -> stays OUT (the wrapping_add identity needs the zero-init).
3. Indexed-Fire-written data -> stays OUT (the indexed assigns need the zero-init).
4. Empty Waits -> empty result.
5. Shape-error on wait_slice for one of the Waits -> .unwrap_or(false) propagates as 'not all whole' -> stays OUT.
6. Pure whole-array Waits inside an Event::Loop body -> classifier descends, data IN let_at_wait.
7. Scalar data (ty.dims empty) -> classifier handles correctly.

## Honest scope

Doc-only / test-only; no Rust code changes. Defensive against future drift of the let-at-wait classifier semantics.
<!-- SECTION:DESCRIPTION:END -->
