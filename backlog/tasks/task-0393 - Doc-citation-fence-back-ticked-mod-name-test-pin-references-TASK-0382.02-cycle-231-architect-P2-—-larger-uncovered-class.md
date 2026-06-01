---
id: TASK-0393
title: >-
  Doc-citation fence: back-ticked mod::name test-pin references (TASK-0382.02
  cycle-231 architect P2 — larger uncovered class)
status: To Do
assignee: []
created_date: '2026-06-01 00:53'
updated_date: '2026-06-01 00:55'
labels:
  - tooling
  - ci
  - doc-lie
  - cycle-221-followup
dependencies:
  - TASK-0382.02
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-231 architect-review (a89db02) finding on the check-doc-test-name-staleness fence. That fence covers back-ticked task<NNNN> unit-test cites (39 in-tree). The architect noted the cycle-197 stale test-pin citation that motivated the fence-family was actually a module::descriptive_name shape (multi_worker_emit::host_excluding_barrier_is_typed_contract_gap), NOT a task<NNNN> name. There are ~643 back-ticked tokens of shape mod::name (or deeper ::-paths) in .rs docstrings/comments -- a LARGER uncovered class than the task<NNNN> arm. Many are stale-prone test-pins (e.g. runtime_src::tests::header_len_matches_wire_runtime). Design a zero-FP existence check: a back-ticked path-shaped tokens tail (last :: segment) should resolve to a defined fn/type/const, OR the full path resolve as a module path. HARD/zero-FP: ::-paths also name TYPES, methods, trait items, std paths (Vec new, BTreeMap insert) and external-crate items the workspace grep cannot see -- huge FP surface. Likely must restrict to a whitelist of in-crate roots or to the ::tests:: infix, SKIP on anything resolvable to std/extern. Only build if zero-FP achievable; else keep deferred. LOW.
<!-- SECTION:DESCRIPTION:END -->
