---
id: TASK-0255
title: 'mp-tcp-event: negative-path coverage for multi_worker Plan::build'
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:35'
updated_date: '2026-05-24 09:38'
labels:
  - M4
  - backend
  - test-gap
dependencies:
  - TASK-0042.05
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect-review F10 of TASK-0042.05 (cycle 79): only one negative test exists for the multi-worker Plan::build ContractGap surface (host_excluding_barrier). Missing tests:
1. missing transfer_buffer_for_seq sidecar entry on a multi-worker push/wait (multi_worker.rs:174-186). pthreads-async has the parallel test build_fails_on_missing_sidecar_buffer_entry — port that shape.
2. worker-to-worker Push rejection (multi_worker.rs:220-228). Currently only exercised by e2e [[skip]] cells; needs a direct Plan::build test asserting the typed ContractGap.
3. Wait without matching Push (multi_worker.rs:154-161): defensive ContractGap untested.
4. malformed/empty EventList on the multi-worker dispatch path (multi_worker.rs:114-119): currently reachable only via the single-worker arm.

These are typed EmitError::ContractGap branches landed in cycle 79; without tests they will rot silently as the codegen evolves. Acceptance: 4 new tests in nucleus/backends/mp-tcp-event/tests/multi_worker_emit.rs, each asserting the precise ContractGap message string (or a stable substring). All branches must trigger via a hand-built Plan + EventList fixture, NOT via running an e2e cell.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add 3 integration tests to nucleus/backends/mp-tcp-event/tests/multi_worker_emit.rs (Branches B, C, D — reachable via emit()):
- wait_without_matching_push_is_typed_contract_gap (Branch B)
- missing_sidecar_buffer_for_seq_is_typed_contract_gap (Branch C, TASK-0233 forward-link)
- worker_to_worker_push_is_typed_contract_gap (Branch D, TASK-0175 forward-link)
2. Add 1 cfg-test unit test inside src/multi_worker.rs (Branch A — emit() dispatch routes <=1 workers to single-worker arm BEFORE Plan::build, so Branch A is unreachable from emit() and must be tested by calling Plan::build directly from inside the crate):
- single_worker_input_is_typed_contract_gap (Branch A)
3. Hand-build per_worker BTreeMap, NameTables, NameSidecar fixtures. Mirror the existing host_excluding_barrier_is_typed_contract_gap pattern: scratch dir, kernels.rs path from 02-split-add, match Err(ContractGap), assert stable substrings + TASK-NNNN forward-links where present.
4. Gate: cargo test, just e2e (92/77/0/15/0), just determinism-check, clippy clean.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 90 (commit eba3f7d): landed 5 new tests covering all 4 brief branches:
- Branch A: single_worker_input_is_typed_contract_gap (lib unit, src/multi_worker.rs); + bonus zero_worker_input_is_typed_contract_gap edge.
- Branch B: wait_without_matching_push_is_typed_contract_gap (integration, tests/multi_worker_emit.rs).
- Branch C: missing_sidecar_buffer_for_seq_is_typed_contract_gap (integration; TASK-0233 forward-link substring asserted).
- Branch D: worker_to_worker_push_is_typed_contract_gap (integration; TASK-0175 forward-link substring asserted).
Gotcha: Branch A is unreachable from emit() because lib.rs:290 dispatch routes used_workers.len() <= 1 to the single-worker arm BEFORE Plan::build. The branch is in effect a dispatch-regression detector; tested by calling pub(crate) Plan::build directly from a #[cfg(test)] mod inside src/multi_worker.rs (the same pattern pthreads-async uses for the same constraint). Branches B/C/D are all reachable from emit() on 2+ worker fixtures and follow the existing host_excluding_barrier_is_typed_contract_gap pattern.
Gate: cargo test -p mp-tcp-event = 5 lib + 7 integration (was 3 + 4) all pass; just e2e = 92/77/0/15/0; just determinism-check = 92/77/0/15; cargo clippy --workspace --all-targets -D warnings = clean.
<!-- SECTION:NOTES:END -->
