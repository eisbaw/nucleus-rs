---
id: TASK-0441
title: >-
  PRD §8.4(a) free-choice / conflict-free structural per-build check (depth
  keystone)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-04 08:16'
updated_date: '2026-06-04 22:13'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per WIP review (2026-06-04). PRD §8.4(a) names the free-choice / conflict-free restriction as the KEYSTONE tractability assumption of the Petri-net soundness model. TASK-0421 exists as a stub but there is NO per-build check enforcing this. This is the only PRD-named tractability restriction without a per-build check.

Scope: implement 'check_free_choice(&PetriNet) -> Result<(), NetSoundnessError>' in nucleus-compiler/src/passes/net_soundness.rs. Wire into the existing per-build check_net_sound aggregator. Add ~3 bite tests including a synthetic conflict net (two transitions sharing a single input place, both enabled). ~200-400 LoC.

Why: Cleanest available DEPTH win - closes a named PRD obligation with concrete code. A reviewer asking 'how do you enforce §8.4(a)?' currently gets prose, not code.

Dependencies: Reuse the existing PetriNet types + CONFLICT_BFS_TRANSITION_LIMIT precedent. Reference TASK-0421 + TASK-0427.01.

Estimated effort: MEDIUM priority, 1 cycle if shallow OR 2 cycles if structurally non-trivial.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TRIAGE-CLOSE as DUPLICATE / SUBSUMED by TASK-0421 (Done 2026-06-02, cycle-241). TASK-0441 premise is factually STALE: it says "TASK-0421 exists as a stub but there is NO per-build check enforcing this" — FALSE. TASK-0421 fully implemented the §8.4(a) per-build check. Verified empirically this cycle (orchestrator-direct, cheap-empirical-verification > trusting the narrative; same class as memory feedback-precursor-filed-without-empirical-verification):

EVIDENCE (greps + test run, 2026-06-05):
- check_conflict_free(net) -> Result<(),ConflictError> at passes/net_soundness.rs:379, with PetriAnalysisError::ConflictingChoice(ConflictError::FreeChoice{place,place_name,transitions,position}) variant (net_soundness.rs:140/174).
- Wired as the FIRST check inside check_net_sound (net_soundness.rs:792, .map_err(PetriAnalysisError::ConflictingChoice)?), conflict-freedom being the precondition the single-order bounded/deadlock replay rests on.
- check_net_sound runs on EVERY build: driver/src/main.rs:562 (cmd_build, stringified "petri-net soundness check failed: {e}").
- REACHABILITY-AWARE predicate + coverability BFS completeness (TASK-0427) under CONFLICT_BFS_TRANSITION_LIMIT=64; large-net BFS-completeness residual already filed as TASK-0427.01 (Low).
- TESTS (cargo test -p nucleus-compiler --test net_soundness => 9 passed / 0 failed THIS CYCLE):
  * REJECTS unsound: free_choice_conflict_net_rejected (place "shared" marking1 -> 2 co-enabled drains -> ConflictingChoice/FreeChoice at position 0); free_choice_net_is_flagged_by_conflict_pass_directly; off_order_free_choice_conflict_now_detected (coverability BFS catches an off-order conflict the single-order replay would miss).
  * ACCEPTS sound: sound_matched_producer_consumer_passes; shipping_shaped_unrolled_loop_buffer_passes_no_false_reject (2 waits on one buffer place, serialised by per-worker control places — the corpus shape AC#1 of TASK-0421 proved must NOT false-reject).

TASK-0441 scope (check_free_choice + wire into check_net_sound + ~3 bite tests + ~200-400 LoC) is 100% delivered by TASK-0421. The only difference is naming (check_conflict_free vs check_free_choice); renaming the established symbol would be churn-for-churn breaking existing tests/docs — NOT done. No new code; no implementer cycle; no review gate (no code change). The user-requested verification ("rejects an unsound net, accepts the sound ones") is SATISFIED by the existing gate, proven above.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DUPLICATE/SUBSUMED by TASK-0421 (Done 2026-06-02). PRD §8.4(a) free-choice/conflict-free already has a per-build structural gate: check_conflict_free (passes/net_soundness.rs:379) wired FIRST into check_net_sound (net_soundness.rs:792), which the driver runs on every build (driver/main.rs:562). Reachability-aware predicate + coverability BFS (TASK-0427); large-net BFS completeness residual is TASK-0427.01 (Low). Verified empirically this cycle: 9/9 net_soundness tests pass — REJECTS an unsound free-choice net (shared place -> 2 co-enabled drains -> ConflictingChoice/FreeChoice) and ACCEPTS sound nets (matched producer/consumer; shipping-shaped unrolled-loop serialised by control places). No new code written: TASK-0441 premise ("TASK-0421 is a stub") was stale/incorrect. Genuine remaining depth in this area = TASK-0427.01 only.
<!-- SECTION:FINAL_SUMMARY:END -->
