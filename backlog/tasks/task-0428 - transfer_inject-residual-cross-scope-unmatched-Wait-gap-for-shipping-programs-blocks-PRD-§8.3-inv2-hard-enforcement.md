---
id: TASK-0428
title: >-
  transfer_inject: residual cross-scope unmatched-Wait gap for shipping programs
  blocks PRD §8.3 inv(2) hard-enforcement
status: To Do
assignee: []
created_date: '2026-06-02 10:04'
labels:
  - compiler
  - event-contract
  - transfer_inject
  - prd-invariant-audit
  - cycle-242
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Encodes the prerequisite that TASK-0422 currently only NARRATES in prose (cycle-241 GAP-2 deferral). PRD §8.3 inv(2) (Push/Wait events form matched pairs) cannot be hard-asserted on shipping output, and the full validator validate_event_lists cannot be wired to a production caller, because transfer_inject still leaves LEGITIMATE unmatched Wait events for currently-shipping programs (reproducer: 02-split-add). Hard-asserting inv(2) today would crash debug builds on VALID input — the panic-on-valid-input class this project rejects (see event_validate.rs deferral rationale + petri_to_events.rs strict-subset debug_assert that EXCLUDES inv(2)).

PRIOR ART that did NOT fully close this: TASK-0136 (splice Push across Sequence/Repeat boundaries), TASK-0149 (splice across nested sequences for hoisted Waits), TASK-0151 (cross-scope finalisation gate is whole-program coarse), TASK-0364 (scope-aware let-at-wait classification / typed EmitError) are ALL Done, yet TASK-0422 (filed AFTER them, 2026-06-02) verified the residual unmatched-Wait still occurs. So this is a residual structural gap beyond those, not a duplicate.

SCOPE: (1) characterise WHY 02-split-add (and any sibling shipping program) still produces an unmatched Wait after all splice work landed — static trace, do not assume; (2) close it at root so unmatched Waits no longer occur for valid programs, OR prove it is a fundamental property of the event model and TASK-0422 must instead reshape inv(2) enforcement (e.g. a participant-scoped matched-pair check). On closure, TASK-0422 becomes actionable (wire validate_event_lists as the EventList-consuming backends entry gate) and TASK-0423 (SyncTag participant-set agreement) follows. Pointers: src/passes/transfer_inject/, src/event_validate.rs (validate_event_lists + :48-84 deferral), src/passes/petri_to_events.rs:225-241 (strict-subset debug_assert).
<!-- SECTION:DESCRIPTION:END -->
