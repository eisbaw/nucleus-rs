---
id: TASK-0425
title: >-
  proptest_petri: boundedness single-order-replay equivalence only validated
  one-directionally (b.1); deadlock (d4) validates both
status: In Progress
assignee:
  - '@me'
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 08:41'
labels:
  - testing
  - proptest
  - prd-invariant-audit
  - cycle-241
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-3. PRD §8.6 claims the single deterministic firing orders replay is equivalent to checking all reachable markings. For DEADLOCK this is fully validated both directions by proptest d4 (proptest_petri.rs:1416, independent exhaustive BFS oracle_can_reach_all_fired). For BOUNDEDNESS, proptest b.1 (proptest_petri.rs:1062) asserts ONLY oracle-clean => pass-accepts; the reverse is deliberately NOT asserted (:1055-1061) because derive_firing_orders single chosen order may legitimately dodge an overflow path the BFS finds. This is HONEST and arguably correct (the reverse genuinely does not hold for an arbitrary single order). FILEABLE IMPROVEMENT (low value, test-completeness): a b5-style property over the SPECIFIC derive_firing_order linearisation: if the BFS finds an overflow reachable AND the derived orders prefix reaches the overflowing marking, the pass MUST flag — i.e. close the gap on the order actually shipped rather than over-claiming the general equivalence. Pointer: tests/proptest_petri.rs b.1 at :1062 + the :1055-1061 one-direction rationale; d4 at :1416 as the both-directions template.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle-241 GAP-3 / b5):
- Add proptest b5_check_bounded_overflow_iff_independent_replay_overflows over net_and_derived_order().
- New INDEPENDENT detector fn order_overflows_independently(net, order): walks order step by step; per step computes needs (PtoT arc weights summed per place) + produces (TtoP), checks enabled (have>=need) first; if a step is NOT enabled it is InvalidFiringOrder territory (NotEnabled) -> the order is malformed, NOT a capacity overflow, so STOP and return false (mirrors check_bounded which returns InvalidFiringOrder, not CapacityExceeded). If enabled: for each touched place would_be = have - consumed + produced; if place.capacity Some(c) and would_be>c -> overflow at this step -> return true. Else commit (mutate local marking) and continue. Re-implemented from net.arcs/places, NOT calling Net::fire/check_bounded -> genuine cross-check of overflow-DETECTION logic over the SAME shipped order.
- Assert BOTH directions: check_bounded(net,order) is Err(CapacityExceeded) IFF detector says overflow.
- Honest-limit docstring: refactor-regression guard over SHIPPED order only (shares per-step enabling primitive concept with pass; a Net::fire enabling bug escapes both — same residual as d.1/d.3/oracle_first_stall_position); does NOT validate general PRD 8.6 equivalence; b.1 general-equivalence reverse gap stays open.
GOTCHA: check_bounded only reports CapacityExceeded when the transition is ENABLED-then-overflows; a NotEnabled step short-circuits to InvalidFiringOrder. derive_firing_order may append stuck leftovers (source order) so a malformed-order suffix is possible -> detector must NOT count NotEnabled as overflow and must STOP at first NotEnabled (check_bounded returns on first error). Capacity check is on NET delta (consume then produce), would_be>cap.get().
<!-- SECTION:NOTES:END -->
