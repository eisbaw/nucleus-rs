---
id: TASK-0354
title: >-
  Unit tests for collect_let_at_wait_data + is_whole_array_recv (TASK-0349 cycle
  220b architect P2.2)
status: In Progress
assignee:
  - '@claude'
created_date: '2026-05-27 23:58'
updated_date: '2026-05-28 00:24'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implementation plan (cycle 221): create one new integration test file nucleus/backend-common/tests/collect_let_at_wait_data.rs with a private make_minimal_tables helper (DataId -> name + ResolvedType{ScalarType::I32, dims}) and 7 #[test] fns mapping 1:1 to the description's 7 cases. All tests drive the PUBLIC collect_let_at_wait_data entry point (exercising the pub(super) is_whole_array_recv indirectly via the documented call site at collect.rs:392). Test names: mixed_whole_and_slice_waits_excludes_data, accumulate_fan_in_data_excluded, indexed_fire_written_data_excluded, empty_waits_yields_empty_set, shape_error_on_wait_slice_excludes_data (out-of-bounds leading range trips wait_slice:269-278 guard -> Err -> unwrap_or(false) -> excluded), whole_array_wait_inside_event_loop_body_included (Event::Loop with whole-array Wait in body -> descent + included), scalar_data_no_dims_treated_as_whole_array. Gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e' must pass; e2e baseline 120/110/0/10/0 preserved (test-only additive); cargo test +7 delta.
<!-- SECTION:PLAN:END -->
