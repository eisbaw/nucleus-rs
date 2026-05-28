---
id: TASK-0353
title: >-
  Formalize TASK-0347 ACs as tickable AC list (description-prose-only → backlog
  CLI ACs)
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-27 22:51'
updated_date: '2026-05-28 01:02'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 222: backfill landed in-thread (orchestrator-self, no implementer spawn).

Actions taken on TASK-0347:
1. 4 formal --ac entries added (matching the description's 'Acceptance criteria' prose block at lines 31-44):
   #1 ACFG: build_dataflow accepts bare-LValue RHS
   #2 Link / codegen: cross-worker data-move lowers via Xfer pair
   #3 15-transpose simplification + bit-identity regression-pin
   #4 Coordinate followup: ALSO close TASK-0097's identity-copy gap

Verified via 'backlog task 0347 --plain': all 4 ACs now appear as tickable - [ ] entries.

The description-prose ACs at TASK-0347:31-44 are preserved as the authoritative narrative (now mirrored in the tickable section).

Future cycles that close TASK-0347 ACs can tick them precisely via --check-ac N.

Orchestrator self-audit (cycle 222b, pre-review-gate self-discovered): formal AC #4 simplified 'Renumber/coordinate followup' to 'Coordinate followup' — the 'Renumber/' prefix was awkward wording in the original prose with no clear meaning. The simplification is honest but is a wording change from the source. No semantic change.
<!-- SECTION:NOTES:END -->
