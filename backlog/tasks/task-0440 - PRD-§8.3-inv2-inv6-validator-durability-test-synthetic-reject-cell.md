---
id: TASK-0440
title: PRD §8.3 inv(2)/inv(6) validator durability test - synthetic reject cell
status: To Do
assignee: []
created_date: '2026-06-04 08:16'
updated_date: '2026-06-04 22:16'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per WIP review (2026-06-04). The PRD §8.3 invariants validator (validate_event_lists in driver/src/main.rs) has its REJECT arm exercised by no test. A refactor could silently delete the validator call without any test biting. Per memory project-e2e-gate-trust-caveats.md point 6.

Scope: One synthetic test that constructs an EventList with a duplicated Push (inv-2 violation) or empty Sync (inv-3 violation) and asserts the validator rejects it with typed error. Bypasses the normal building cells (which always produce valid EventLists by construction). ~50 LoC. Defends durability of the TASK-0422 gate work.

Why: Without a biting reject test, the validator's existence is asymptotically guaranteed to rot.

Estimated effort: LOW priority, ~50 LoC, single cycle. No design risk.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TRIAGE (2026-06-05, orchestrator-direct grep-witness, applying the now-HARD rule [[feedback-precursor-filed-without-empirical-verification]] — verify "no test exists" before working). FINDING: TASK-0440 needs RE-SCOPE, not implementation-as-written.

The LITERAL scope ("one synthetic test constructs an EventList with an inv-2 violation and asserts the validator rejects it with typed error, ~50 LoC") is ALREADY SHIPPED, multiple times:
- nucleus/driver/tests/task0422_gate_wired_reject.rs::task0422_driver_gate_rejects_unmatched_push — synthetic unmatched-Push EventList -> asserts UnmatchedPush typed reject (+ _accepts_matched_pair positive).
- nucleus/driver/src/gate.rs unit tests gate_rejects_unmatched_push (172) / gate_accepts_matched_pair (205) — call gate_per_worker_for_dispatch, the EXACT fn cmd_build invokes.
- nucleus/nucleus-compiler/tests/event_validate.rs (all 6 EventValidationError variants), proptest_event_validate.rs (TASK-0429, all arms non-vacuous), task0422_01_inv2_post_mediation.rs (corpus-clean proof).

So writing another synthetic function-reject test would be pure duplication (no-op vs the stated GOAL).

GENUINE RESIDUAL (matches TASK-0440 GOAL "a refactor could silently delete the validator CALL without any test biting", and the gate.rs:140 self-admitted "HONEST RESIDUAL"): NO test bites if the production CALL `gate::gate_per_worker_for_dispatch(...)` at driver/src/main.rs:652 is deleted/downgraded. All existing tests exercise the gate FUNCTION, not the cmd_build CALL SITE. Driving cmd_build to the reject arm via a real .nuc is infeasible BY DESIGN (corpus-clean: no valid source violates inv2 — task0422_gate_wired_reject.rs docstring).

RECOMMENDED RE-SCOPE: add a strict-no-op fault-injection SEAM in cmd_build (env-gated, e.g. NUC_EVENT_GATE_NEGATIVE=1 perturbs the final per_worker EventList to violate inv2 just before the gate call) + a driver integration test that runs a real cmd_build under the env and asserts it returns Err at the gate — so deleting the main.rs:652 call makes the test FAIL loud. This is the SAME wired-path-bite pattern as TASK-0168 (required-coverage-negative) and TASK-0446 (--with-mpi coverage guard). Genuine value, ~tractable single cycle, production-driver change (needs the review gate). Leaving To Do with this re-scope, NOT closing (goal unmet).
<!-- SECTION:NOTES:END -->
