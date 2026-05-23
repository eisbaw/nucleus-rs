---
id: TASK-0255
title: 'mp-tcp-event: negative-path coverage for multi_worker Plan::build'
status: To Do
assignee: []
created_date: '2026-05-23 23:35'
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
