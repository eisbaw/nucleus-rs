---
id: TASK-0422
title: >-
  PRD §8.3 inv(2): full event-contract validator validate_event_lists has NO
  production caller (Push/Wait pair matching unenforced on shipping output)
status: Done
assignee:
  - '@mped'
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 17:54'
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

Forward-carried from TASK-0422.01 (cycle-243, commit 62c1c9e): STEP (1) IS GREEN. PRD §8.3 inv(2) (matched Push/Wait pairs) is empirically proven to hold on the POST-mediation EventList for ALL 4 mp-* backends (mp-tcp-{bufsync,event,poll} + mp-uds-event) across the entire example corpus: 220 (backend, schedule) cells, 0 inv(2) violations, 0 pipeline errors. Regression-pinned + non-vacuity-pinned at nucleus/driver/tests/task0422_01_inv2_post_mediation.rs.

What step (2) (gate-wiring) can now ASSUME:
 - validate_event_lists returns Ok on the post-mediation EventList that the mp-* backends actually consume, for every shipping schedule. Wiring it as a HARD gate at the mp-* backend EventList-consumption entry will NOT crash on any current corpus program. (This was the exact panic-on-valid-input risk that justified the original deferral; it is now discharged for the mediated backends too, not just the pre-mediation pthreads/openmp chain TASK-0428 covered.)
 - The mediation pass set + host election to mirror lives at driver/src/main.rs ~464-553; the test mirrors it via backend_common::elect_host_from_name_workers (shared helper, no skew). If step (2) wires the gate INSIDE the driver after the mediation passes (rather than inside each backend), that single site covers all 4 mp-* backends.

LIMITS step (2) must still respect:
 - This is COMPILE-time contract proof over the current corpus only; it is NOT a proof for arbitrary future schedules. A hard gate is therefore correct (it would catch a future regression), but the gate itself is the enforcement, not this sweep.
 - host_data_relay_inject no-ops for many schedules; the proof covers inv(2) on whatever each pass emits, with non-vacuity demonstrated for 09-producer-consumer/pipelined only.
 - The acfg_to_events debug_assert deliberately stays per-worker-subset because it precedes mediation (see corrected rationale in petri_to_events.rs); step (2) should add the FULL validate_event_lists call at/after the post-mediation projection, not promote the pre-mediation assert.

Implementation Plan (cycle-244, step 2 gate-wiring):
1. Wire validate_event_lists(&per_worker) as a HARD String-error gate in driver cmd_build, sibling of check_accumulator_consistency, right before dispatch_backend (main.rs ~743). Reads the FINAL per_worker (post acfg_to_events -> inject_check_frames -> apply_safe_push_reorder). ONE site = all 7 backends. Add to the nucleus_compiler use-block.
2. VERIFIED inv(2)-preservation through the two post-projection transforms by reading their impls:
   - inject_check_frames (passes/inject_check_frames.rs:185 inject_event): only rewrites Event::Loop nodes (sets check_frame); ALL non-Loop events incl Push/Wait are the `other => other` pass-through arm. Adds/removes NO Push or Wait. CONFIRMED.
   - apply_safe_push_reorder (passes/safe_push_reorder.rs:138 reorder_boundary): partitions boundary event indices into hoistable_idx + others_idx and concatenates -> pure permutation, every event preserved, none added/deleted. validate inv(2) is SET-based (BTreeMap keyed by (src,dst,data,tile,seq)) so reorder cannot break pairing. CONFIRMED.
   Structural argument HOLDS; the ~743 site is equivalent to the ~646 projection for inv(2) purposes.
3. Bite test: a real .nuc violating inv(2) is impossible by design (corpus proven clean), so add a focused negative test in driver/tests/ that calls validate_event_lists (the EXACT fn the driver wires) on a hand-built unmatched-Push map, asserts Err(UnmatchedPush), and documents the driver wiring site. Honest: a true end-to-end driver-reject test is not feasible (no valid source produces a violation).
4. Fix stale docs: event_validate.rs module-doc (63-99) + validate_event_lists_strict_per_worker doc (348-360); petri_to_events.rs module-doc (175-182) + acfg_to_events comment (245-256); tests/petri_to_events.rs TASK-0428 sweep comment (494-519). Do NOT touch the acfg_to_events debug_assert (stays subset; precedes mediation).
5. Gate: just build && clippy && test && test-release && e2e; e2e must hold 385/328/0/57/0.

RESULT (cycle-244): GATE WIRED. validate_event_lists (FULL surface incl PRD §8.3 inv(2) Push/Wait pairing) is now a HARD String-error production gate in driver cmd_build, sibling of check_accumulator_consistency, immediately before dispatch::dispatch_backend, reading the FINAL per_worker. ONE site covers all 7 backends (it is THE backend-consumption entry).

inv(2)-PRESERVATION VERIFICATION (the load-bearing soundness claim) — CONFIRMED by reading both post-projection transform impls:
 - inject_check_frames: inject_event (passes/inject_check_frames.rs:185) matches only Event::Loop (sets check_frame); every non-Loop event, incl Push/Wait, hits the `other => other` pass-through arm. Adds/removes/mutates NO Push or Wait.
 - apply_safe_push_reorder: reorder_boundary (passes/safe_push_reorder.rs:138) classifies each boundary event index into hoistable_idx vs others_idx, then emits hoistable-then-others -> a PURE PERMUTATION within each top-level boundary; Sync events stay in place; no event added/deleted/field-mutated. validate inv(2) is SET-based (BTreeMap keyed (src,dst,data,tile,seq)) => reorder cannot change pairing.
 Conclusion: the ~744 gate site is equivalent to the ~646 projection for inv(2); no transform could change the Push/Wait set. NO finding.

BITE TEST: nucleus/driver/tests/task0422_gate_wired_reject.rs (2 fns):
 - task0422_driver_gate_rejects_unmatched_push: calls validate_event_lists (THE fn the driver wires) on a hand-built unmatched-Push map, asserts Err(UnmatchedPush). Docstring + assertion document the exact driver wiring site.
 - task0422_driver_gate_accepts_matched_pair: matched pair => Ok (non-vacuity guard).
 HONEST LIMIT: a true end-to-end driver-reject test (real .nuc -> cmd_build Err) is NOT feasible: no valid source produces an inv(2) violation (corpus proven clean by TASK-0428 + TASK-0422.01), which is the whole reason wiring is safe. Such a source would itself be a NEW finding, not a fixture. The e2e differential is the complementary POSITIVE-direction corpus-wide proof (gate runs on every codegen build; 385/328/0/57/0 held).

STALE DOCS FIXED (comment-doc-lie hygiene): event_validate.rs module-doc "How this is wired" + validate_event_lists_strict_per_worker docstring; petri_to_events.rs module-doc + acfg_to_events debug_assert rationale comment; tests/petri_to_events.rs TASK-0428 sweep comment. All now cite the driver gate site and do NOT overclaim (enforced on shipping codegen builds; e2e is defense-in-depth; NOT "all programs forever"). The acfg_to_events debug_assert LEFT as strict-per-worker SUBSET (untouched) — it precedes mediation AND is hit by the driver pre-mediation host-election preview projections (main.rs ~484/~537) where inv(2) need not hold.

GATE: just build OK; just clippy clean (-D warnings, independently re-run); just test dev=1260/0/3 (+2); just test-release=1258/0/3 (+2); just e2e=385/328/0/57/0 (baseline HELD). No findings, no new tasks filed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
PRD §8.3 inv(2) (Push/Wait matched pairs) is now ENFORCED on shipping compiler output. validate_event_lists wired as a hard String-error gate in driver cmd_build (the final per_worker, before dispatch_backend; one site, all 7 backends). inv(2)-preservation through inject_check_frames (Loop-only rewrite, Push/Wait pass-through) and apply_safe_push_reorder (pure intra-boundary permutation) verified by reading both impls. Bite test driver/tests/task0422_gate_wired_reject.rs pins that the driver consumes this exact validator (rejects UnmatchedPush, accepts matched pairs); a true end-to-end driver-reject test is infeasible by design (no valid source violates inv(2) — corpus proven clean). Stale "no production caller / not wired" docs across event_validate.rs + petri_to_events.rs + tests corrected, not overclaimed. acfg_to_events debug_assert intentionally left as strict-per-worker subset (precedes mediation + driver preview projections). Gate green: dev 1260/0/3, release 1258/0/3, e2e 385/328/0/57/0 held; clippy clean. No findings, no new tasks.
<!-- SECTION:FINAL_SUMMARY:END -->
