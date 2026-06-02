---
id: TASK-0422
title: >-
  PRD §8.3 inv(2): full event-contract validator validate_event_lists has NO
  production caller (Push/Wait pair matching unenforced on shipping output)
status: To Do
assignee: []
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 16:21'
labels:
  - compiler
  - event-contract
  - prd-invariant-audit
  - cycle-241
  - principled-deferral
dependencies:
  - TASK-0422.01
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-2, VERIFIED. PRD §8.3 lists the per-worker EventList contract invariants; invariant (2) = Push/Wait events form matched pairs. The full validator validate_event_lists (event_validate.rs) is the ONLY checker of inv(2), and it has ZERO production call sites (VERIFIED: grep finds only docstring refs + the lib.rs re-export; no backend or pass calls it). The shipping enforcement is only validate_event_lists_strict_per_worker, a release-stripped debug_assert at petri_to_events.rs:239 that by construction EXCLUDES inv(2). So validate_event_lists is effectively dead production code presented as a gate.

PRINCIPLED DEFERRAL (documented at event_validate.rs:73-84): transfer_inject has a known cross-scope splicing gap that leaves LEGITIMATE unmatched Wait events for currently-shipping programs (e.g. 02-split-add), so hard-asserting inv(2) today would crash debug builds on valid input (the exact panic-on-valid-input class this project rejects). Hence the deliberate strict-subset-only debug_assert.

SCOPE (gated on the transfer_inject cross-scope splice landing — file/find that as the prerequisite): once unmatched Waits no longer occur for valid programs, wire validate_event_lists as the EventList-consuming backends entry gate (its docstring already nominates backend codegen as the opt-in caller, but NO backend opts in — verified). Until then this stays a tracked gap so the dead validator is not mistaken for a live gate. Mitigation today: the e2e bit-identical differential would surface a genuinely-wrong Push/Wait pairing as a red cell, so this is defense-in-depth, not the only line. All 6 EventValidationError variants ARE individually bite-tested in tests/event_validate.rs.

Pointers: src/event_validate.rs (validate_event_lists + the :48-84 deferral rationale); src/passes/petri_to_events.rs:225-241 (the strict-subset debug_assert + why inv(2) is excluded).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
UNBLOCKED by TASK-0428 (cycle-242, commit e2a4ecd). Forward-carried from TASK-0428: the deferral premise this task documented ("transfer_inject leaves legitimate unmatched Waits, hard-asserting inv(2) crashes debug builds") is STALE. Empirically verified inv(2) holds on the projected EventList for the ENTIRE example corpus (55/55 schedules, 0 violations) via the real pipeline; 02-split-add (the cited reproducer) is Ok(()). Regression-pinned (tests/petri_to_events.rs::task0428_inv2_holds_for_entire_example_corpus). Stale docstrings at event_validate.rs:71-84 and petri_to_events.rs module-doc/debug_assert are corrected.

REMAINING SCOPE for this task (now actionable): wire validate_event_lists as the EventList-consuming-backend entry gate. TWO sub-steps, in order:
 (1) CONFIRM inv(2) over the POST-MEDIATION EventList for mp-tcp-{bufsync,event,poll} + mp-uds-event. TASK-0428 sweep covered only the pthreads-{sync,async} backend-agnostic chain; those mp-* backends run host_mediation_inject + host_data_relay_inject AFTER inject_transfers, re-routing Push/Wait through host. Extend the sweep to apply those two passes (driver main.rs ~464-end gates them on backend name + elects host) before validate_event_lists. If any post-mediation EventList violates inv(2), that is a NEW finding (mediation-pass bug or a genuine inv(2) reshape need) — do NOT just relax the check.
 (2) Only after (1) is green: add the validate_event_lists call at the backend EventList-consumption entry (or promote the acfg_to_events debug_assert to the full validator IF (1) proves it safe pre-mediation too). The acfg_to_events assert site currently stays per-worker-subset deliberately because it precedes mediation (see corrected rationale comment there).

The "principled deferral" framing in this task description is now historical — keep for provenance but the live blocker is (1), not the transfer_inject splice gap.
<!-- SECTION:NOTES:END -->
