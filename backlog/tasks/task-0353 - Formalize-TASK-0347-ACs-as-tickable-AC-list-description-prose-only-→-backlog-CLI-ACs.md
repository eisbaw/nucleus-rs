---
id: TASK-0353
title: >-
  Formalize TASK-0347 ACs as tickable AC list (description-prose-only → backlog
  CLI ACs)
status: To Do
assignee: []
created_date: '2026-05-27 22:51'
labels:
  - tracker-hygiene
  - backlog-debt
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0347 (ACFG + link: handle identity-copy dataflow statements; cycle-77 DEFERRED trigger fired by 15-transpose) was filed cycle 204 with 'Acceptance criteria' as prose in the description block (lines 31-44 of the description). The formal Acceptance Criteria section reads 'No acceptance criteria defined'.

The cycle-219 review-gate caught that TASK-0341.01's cycle-219 closure note referenced 'TASK-0347 AC#3 regression-pin' but no such tickable AC exists. Cycle-219 reworded TASK-0341.01's reference to be vaguer (regression-pin candidate, not AC#3); this task formalizes TASK-0347's ACs so future references can be precise.

## Acceptance criteria

1. Read TASK-0347's description prose 'Acceptance criteria' block.
2. Re-file the 4 prose ACs as formal --ac entries via 'backlog task edit TASK-0347 --ac "..." --ac "..." ...' so they appear in the Acceptance Criteria section and can be ticked individually.
3. Verify via 'backlog task view TASK-0347 --plain' that the ACs are now tickable.
4. No code changes; tracker-only hygiene.

## Honest scope LIMITS

- Doc-only. No ACFG / link work happens here.
- Low priority because TASK-0347 itself is To Do, not in progress. File only when TASK-0347 is about to be picked up (the formalization will be the first step of that cycle anyway).
<!-- SECTION:DESCRIPTION:END -->
