---
id: TASK-0429
title: >-
  Hardening: property tests for validate_event_lists — panic-freedom +
  deterministic emission under arbitrary/adversarial EventLists
status: To Do
assignee: []
created_date: '2026-06-02 20:17'
labels:
  - compiler
  - event-contract
  - hardening
  - property-test
  - cycle-246
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Backlog-maturity hardening wave (cycle-246). The full validate_event_lists (event_validate.rs, the production gate wired at driver/src/gate.rs via TASK-0422/0423) has only 10 hand-built #[test] bite cases (tests/event_validate.rs) and NO property test. Its two LOAD-BEARING contract claims are unproven under fuzzing: (1) the docstring 'Pure function; never panics on user-reachable input' — the gate at gate.rs relies on this (a panic there crashes the compiler on the very input it should DIAGNOSE, the exact panic-not-diagnostic class this project rejects); (2) deterministic, sorted error emission (the module doc at ~278-283 + the gate error message depend on it; cited repeatedly across cycles 242-245 but only positively tested). proptest is already a dev-dep and proptest_serde.rs:417-479 ALREADY has event_leaf()/event_strategy() generating arbitrary Event values (incl. nested Loop bodies) — reuse or model on it. SCOPE: add property tests (new tests/proptest_event_validate.rs, or a proptest! block in tests/event_validate.rs) generating arbitrary BTreeMap<WorkerId, Vec<Event>> (including malformed: self-push, unmatched pairs, empty/disagreeing Syncs, deeply-nested Loops) and assert: (P1 panic-freedom) validate_event_lists returns Ok/Err without panicking for ANY generated input; (P2 determinism) validating the same input twice yields an identical error Vec, AND validating a WORKER-ORDER-permuted input yields the identical error SET (BTreeMap iteration makes worker order irrelevant — pins cross-worker determinism); (P3 reorder-invariance, optional) permuting events WITHIN a worker does not change the set of UnmatchedPush/Wait or SyncParticipantDisagreement errors (set-keyed by (src,dst,data,tile,seq)/SyncTag — pins the safe_push_reorder soundness argument from TASK-0422.02 as a property). Keep it honest: if a property reveals a real non-determinism or panic, that is a REAL finding to root-cause, NOT a test to weaken. Pointers: nucleus-compiler/src/event_validate.rs (validate_event_lists:293, the emission-order doc, EventValidationError); nucleus-compiler/tests/proptest_serde.rs:417-479 (event strategy); nucleus-compiler/tests/proptest_petri.rs (proptest! house style + shrink config).
<!-- SECTION:DESCRIPTION:END -->
