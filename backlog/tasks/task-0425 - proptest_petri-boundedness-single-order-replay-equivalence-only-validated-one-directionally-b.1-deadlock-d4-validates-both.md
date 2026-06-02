---
id: TASK-0425
title: >-
  proptest_petri: boundedness single-order-replay equivalence only validated
  one-directionally (b.1); deadlock (d4) validates both
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 10:03'
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

DONE (commit 7702a90). Gate run locally: build OK; clippy OK (forced fresh test-target recheck for doc_lazy_continuation); test dev 1252 passed/0 failed/3 ignored (baseline 1251 +1); test-release 1250 passed/0 failed/3 ignored (baseline 1249 +1); e2e 385/328/0/57/0 UNCHANGED (tests-only, no codegen).

b5 PASSED on first authoring (no pass disagreement surfaced; detector matched check_bounded on every case). TEETH-VERIFIED: temporary eprintln instrumentation (reverted before commit) showed the overflow=true branch fires ~443/2000 cases — NOT a vacuous false==false tautology. derive_firing_order DOES ship overflowing orders: it only fires non-overflowing transitions greedily, then appends remaining transitions in source order (boundedness.rs `if order.len() < total`); a stuck-leftover that is token-enabled-but-overflows is where check_bounded reports CapacityExceeded on the shipped order.

GOTCHAS / contract facts (feed-forward):
1. check_bounded reports CapacityExceeded ONLY when the transition is token-ENABLED first; a NotEnabled step short-circuits to InvalidFiringOrder (deadlock territory), NOT CapacityExceeded. The detector MUST therefore return None (not overflow) at the first non-enabled step and STOP — mirroring check_bounded returning on first error. Counting a non-enabled step as overflow would have falsely failed the biconditional.
2. Capacity contract = NET delta: would_be = have - consumed + produced, checked would_be > cap.get(). A self-looping buffer (consumed AND produced on same place) is checked at post-firing count, not transient peak. Multiple arcs same place sum weights.
3. Unbounded places (capacity None) never overflow.
4. The detector re-implements overflow DETECTION from net.arcs/net.places/Marking; it does NOT call check_bounded/Net::fire/fire_in_place -> genuine cross-check. But it shares the per-step enabling MODEL meaning with the pass -> a Net::fire enabling bug escapes both. SAME acceptable residual as d.1/d.3/d.4/oracle_first_stall_position; documented in the b5 + helper docstrings. Does NOT close PRD 8.6 general equivalence; b.1 general-equivalence reverse gap stays OPEN (honest residual stated).
REJECTED approach: asserting reverse direction against the BFS oracle (oracle-overflow => pass flags) — that is exactly the direction b.1/b.3 correctly refuse because the chosen order may dodge a BFS-reachable overflow. b5 deliberately sidesteps the BFS and asserts over the FIXED shipped order only.

Cycle-242 orchestrator review gate (independent, read-only):
- qa-test-runner: GO. build OK; clippy clean (forced fresh recheck, no doc_lazy_continuation on +206 lines); test 1252 dev / 1250 release (0 failed, 3 ignored); e2e 385/328/0/57/0 x2; b5 green across 8 runs incl. 3x at PROPTEST_CASES=4000, no seed sensitivity / no flakiness.
- mped-architect: GO. Verified line-by-line that order_first_overflow_position is a GENUINE independent cross-check (reads only initial_marking/arcs/places[].capacity; never calls check_bounded/Net::fire/fire_in_place) and matches check_bounded contract on all 4 points: net-delta capacity (not transient peak), multi-arc weight summation, None-capacity never overflows, non-enabled step short-circuits to non-overflow (the highest-risk point — handled correctly). Biconditional is real prop_assert_eq! with overflow=true cases asserted (NOT prop_assume-d away); ~22% bite rate confirmed plausible from derive_firing_order appending stuck leftovers in source order. Docstrings honest, no overclaim; b.1 general PRD-§8.6 equivalence reverse gap correctly left OPEN.

LATENT residual (architect P3, non-blocking, NOT fixed — dead under current generator): order_first_overflow_position uses plain `have - consumed + produced` while fire_in_place uses checked_sub/checked_add().expect(). Safe under THIS property net_and_derived_order() (weight=1, <=4 places, capacity 1..=3 => single-digit token counts, no u32 overflow possible). FORWARD-CARRIED: if b5 is ever repointed at weighted_net_and_derived_order() (weight in 1..=3, as b.4 uses), mirror the checked_* arithmetic so the detector panics symmetrically with fire_in_place rather than wrapping. Re-check at that widening, not before.

TASK-0425 stays DONE — both directions asserted over the shipped order, gate green, independent review GO.
<!-- SECTION:NOTES:END -->
