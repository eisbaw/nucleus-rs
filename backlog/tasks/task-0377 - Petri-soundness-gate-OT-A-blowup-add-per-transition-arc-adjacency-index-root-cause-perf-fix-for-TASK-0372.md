---
id: TASK-0377
title: >-
  Petri soundness gate O(T*A) blowup: add per-transition arc adjacency index
  (root-cause perf fix for TASK-0372)
status: To Do
assignee: []
created_date: '2026-05-30 23:06'
labels:
  - perf
  - petri
  - gate
  - root-cause
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
ROOT-CAUSE fix superseding TASK-0372's flag-it-off workaround. Orchestrator measurement (cycle 218) of the cycle-217 Petri gate (passes/net_soundness.rs, check_net_sound = derive_firing_order + check_bounded + check_deadlock_free):

EVIDENCE (prebuilt release binary, direct invocation, EPOCHREALTIME, LC_ALL=C, 30 reps):
- 07-matmul/distributed8 (T=4149 transitions, P=32870 places, A=65722 arcs): GATE-ON 473 ms/build vs GATE-OFF (gate stubbed, rebuilt) 34 ms/build -> gate adds ~439 ms = 93% of build, a 14x slowdown.
- 16-jacobi/distributed (T=402, A=2170): gate ~5 ms total (small net).
- Instrumented per-component on dist8: derive_firing_order=187ms, check_bounded=179ms, check_deadlock_free=172ms (each ~T fire() calls).

ROOT CAUSE: petri.rs Net::fire(t) is O(A) -- it does self.arcs.iter().filter(|a| a.transition==t) TWICE (PtoT at line ~349, TtoP at line ~380) scanning ALL arcs to find those incident to t. No per-transition adjacency index. Each of the 3 analyses fires ~T transitions => O(T*A) per analysis; ~800M arc comparisons total on dist8. Secondary: fire() returns Ok(self.current_marking.clone()) (line ~429) cloned on every success but DISCARDED by all 3 analysis hot-paths; check_bounded/check_deadlock_free clone marking_before EVERY step (lines ~188/~236) though it is only used on the (rare) failure arm and fire() does not mutate on failure. Tertiary: derive_firing_order rescans from index 0 each outer iter -> O(T^2) cheap bool-skips (~17M on dist8, minor).

FIX (near-linear, MUST stay bit-identical -- gate must accept/reject exactly the same nets): (1) precompute per-transition incident-arc index once (Vec<Vec<arc-idx>> or in/out arc lists keyed by TransitionId) and have fire (or a fire_in_place variant) consult it in O(deg(t)) instead of O(A); (2) add a non-cloning fire path for the analyses so the discarded marking clone is gone; (3) capture marking_before lazily only on the failure arm (fire leaves marking unmutated on failure); (4) optional: advance derive_firing_order scan cursor past the contiguous-fired prefix. Behavior is preserved because needs/produces are BTreeMap-summed per place (order-independent) and determinism comes from arc-insertion order which the index preserves.

ACCEPTANCE: gate cost on dist8 drops from ~440ms to a small fraction (target <40ms, ideally <15ms); existing boundedness.rs/deadlock.rs/net_soundness.rs unit tests still pass; e2e bit-identity unchanged across all 7 tier-1 backends; just test + test-release + e2e all green. Once landed, TASK-0372's CLI flag + e2e split is very likely UNNECESSARY (gate stays always-on AND cheap) -- re-evaluate and close 0372 as superseded if the perf target is met.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Net::fire (or a new fire_in_place) consults a per-transition arc adjacency index; no O(A) all-arcs scan remains on the analysis hot path
- [ ] #2 Per-build Petri gate cost on 07-matmul/distributed8 drops from ~440ms to <40ms (measured, report before/after numbers)
- [ ] #3 All existing boundedness/deadlock/net_soundness unit tests pass; behavior bit-identical (gate accepts/rejects the same nets)
- [ ] #4 just test + just test-release + just e2e green; e2e totals + bit-identity unchanged across 7 tier-1 backends
- [ ] #5 A perf regression pin (test or documented benchmark cmd) records the near-linear expectation so a future O(A)-reintroduction is catchable
<!-- AC:END -->
