---
id: TASK-0440
title: PRD §8.3 inv(2)/inv(6) validator durability test - synthetic reject cell
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-04 08:16'
updated_date: '2026-06-04 22:43'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
COMPILE-TIME WITNESS (type-state), supersedes the env-gated runtime seam.

1. gate.rs: add pub(crate) struct GatedPerWorker<'a> { per_worker: &'a BTreeMap<WorkerId, Vec<Event>> } (PRIVATE field) + pub(crate) fn events(&self) -> &'a BTreeMap. Doc: sole constructor is the gate success path => holding the token PROVES the gate ran. Cite TASK-0440, supersedes TASK-0422.02 honest residual.
2. gate.rs: change gate_per_worker_for_dispatch return Result<(), String> -> Result<GatedPerWorker<'a>, String> with 'a tying per_worker; success path Ok(GatedPerWorker { per_worker }). Two checks + error strings BYTE-IDENTICAL.
3. dispatch.rs: dispatch_backend second param &BTreeMap -> gated: crate::gate::GatedPerWorker<'_>; first body line let per_worker = gated.events();
4. main.rs cmd_build: let gated = gate::...(...)?; pass gated to dispatch_backend.
5. Fix now-FALSE docstrings: gate.rs:14-25 module HONEST RESIDUAL, gate.rs ~140 test-mod HONEST RESIDUAL, main.rs ~650 inline comment. Residual now CLOSED by witness; call is compile-time load-bearing.
6. Add ONE positive unit test: gate matched pair -> gated.events() == input map.
7. Bite proof: comment out gate call -> observe compile error -> restore -> compiles. Record verbatim.
8. Gate: build --release, clippy -D warnings, test, test --release, e2e (hold 427/364/0/63/0).
<!-- SECTION:PLAN:END -->

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

APPROACH CHOSEN (2026-06-05, user delegated "do whatever is most correct"): COMPILE-TIME WITNESS, not the env-gated runtime seam I first proposed. Rationale: gate.rs:14-25 documents TASK-0422.02 DELIBERATELY DECLINED a fault-injection hook in the production hot path on maintainability grounds, claiming "the ONLY way to prove the literal invocation is a #[cfg(test)]-gated fault-injection hook." That claim is INCOMPLETE — a type witness is another way that DOMINATES: gate_per_worker_for_dispatch returns a GatedPerWorker<a> token (private constructor in gate.rs, sole source is the gate); dispatch_backend takes the token INSTEAD of &per_worker. Then deleting the cmd_build gate call (main.rs:652) fails to COMPILE (no token for dispatch) — strictly stronger than a runtime test (cannot regress even with all tests deleted) and ZERO production-hot-path test scaffolding (resolves the exact maintainability objection). dispatch_backend has ONE call site (main.rs:654), so contained. Bite proof = comment out the gate call -> observe compile error -> restore -> compiles (fail-then-pass). Validator runtime detection stays proven by the existing gate.rs unit tests (gate_rejects_unmatched_push) + event_validate.rs. MUST update the now-FALSE docstrings (gate.rs HONEST RESIDUAL "deliberately NOT added / only way is a fault hook"; main.rs:650-651 "this call line itself is not test-proven to execute") — they become lies once the witness lands (comment-doc-lie recurring pattern).

IMPLEMENTED (2026-06-05, compile-time witness). Files: nucleus/driver/src/gate.rs (GatedPerWorker witness + return-type change + 2 docstring rewrites + 1 new positive unit test), nucleus/driver/src/dispatch.rs (param &BTreeMap -> GatedPerWorker token + first-line events()), nucleus/driver/src/main.rs (thread let gated = ...; pass gated; inline-comment rewrite).

Driver package name is "nucleus" (NOT nucleus-driver) — cargo -p nucleus.

BITE PROOF (fail-then-pass for the compile-time guarantee): commenting out the cmd_build gate call line gives:
  error[E0425]: cannot find value `gated` in this scope
   --> driver/src/main.rs:659:9
    | 659 |         gated,
    |             ^^^^^ not found in this scope
  error: could not compile `nucleus` (bin "nucleus") due to 1 previous error
Restoring the line -> compiles clean (Finished dev profile). So the literal gate invocation is now compile-time load-bearing; deleting it breaks the build. Residual closed; supersedes TASK-0422.02.

GOTCHA for next agent: Result::expect_err in the existing gate_rejects_unmatched_push test requires the Ok type to be Debug, so GatedPerWorker needs #[derive(Debug)] (added, documented as test-only). dispatch.rs no longer names BTreeMap/Event/WorkerId (per_worker type inferred from events()), so those 3 imports were removed to keep clippy -D warnings clean.

GATE NUMBERS (all green inside nix dev shell): build --release OK; clippy --workspace --all-targets -D warnings OK; cargo test OK (gate_witness_threads_event_list_through + gate_rejects/accepts all pass); cargo test --release OK (0 fail); e2e total 427 / pass 364 / fail 0 / skipped 63 / required-fail 0 (baseline held).
<!-- SECTION:NOTES:END -->
