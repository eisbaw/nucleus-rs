---
id: TASK-0422
title: >-
  PRD §8.3 inv(2): full event-contract validator validate_event_lists has NO
  production caller (Push/Wait pair matching unenforced on shipping output)
status: To Do
assignee: []
created_date: '2026-06-02 02:27'
labels:
  - compiler
  - event-contract
  - prd-invariant-audit
  - cycle-241
  - principled-deferral
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-2, VERIFIED. PRD §8.3 lists the per-worker EventList contract invariants; invariant (2) = Push/Wait events form matched pairs. The full validator validate_event_lists (event_validate.rs) is the ONLY checker of inv(2), and it has ZERO production call sites (VERIFIED: grep finds only docstring refs + the lib.rs re-export; no backend or pass calls it). The shipping enforcement is only validate_event_lists_strict_per_worker, a release-stripped debug_assert at petri_to_events.rs:239 that by construction EXCLUDES inv(2). So validate_event_lists is effectively dead production code presented as a gate.

PRINCIPLED DEFERRAL (documented at event_validate.rs:73-84): transfer_inject has a known cross-scope splicing gap that leaves LEGITIMATE unmatched Wait events for currently-shipping programs (e.g. 02-split-add), so hard-asserting inv(2) today would crash debug builds on valid input (the exact panic-on-valid-input class this project rejects). Hence the deliberate strict-subset-only debug_assert.

SCOPE (gated on the transfer_inject cross-scope splice landing — file/find that as the prerequisite): once unmatched Waits no longer occur for valid programs, wire validate_event_lists as the EventList-consuming backends entry gate (its docstring already nominates backend codegen as the opt-in caller, but NO backend opts in — verified). Until then this stays a tracked gap so the dead validator is not mistaken for a live gate. Mitigation today: the e2e bit-identical differential would surface a genuinely-wrong Push/Wait pairing as a red cell, so this is defense-in-depth, not the only line. All 6 EventValidationError variants ARE individually bite-tested in tests/event_validate.rs.

Pointers: src/event_validate.rs (validate_event_lists + the :48-84 deferral rationale); src/passes/petri_to_events.rs:225-241 (the strict-subset debug_assert + why inv(2) is excluded).
<!-- SECTION:DESCRIPTION:END -->
