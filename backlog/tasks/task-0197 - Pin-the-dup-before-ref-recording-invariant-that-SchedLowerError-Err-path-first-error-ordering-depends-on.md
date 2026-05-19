---
id: TASK-0197
title: >-
  Pin the dup-before-ref-recording invariant that SchedLowerError Err-path
  first-error ordering depends on
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 17:16'
updated_date: '2026-05-19 18:29'
labels:
  - M0
  - compiler
  - diagnostics
  - tech-debt
  - test
dependencies:
  - TASK-0196
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Non-blocking latent-fragility surfaced by the TASK-0196 mped-architect review (minor obs #2; both reviewers GO, explicitly no follow-up REQUIRED — this tracks it per backlog-as-working-memory so it cannot silently rot). TASK-0196 option (b) relocated UnknownWorkerClass/UnknownAccessibleByName into pass-1 AST-walk side-tables (worker_class_refs/accessible_by_refs) and proved first-error ordering byte-equivalent to the old name-sorted BTreeMap iteration via a stable sort. That equivalence holds ONLY because worker/region name uniqueness holds, which in turn holds ONLY because dup-detection (DuplicateWorker/DuplicateMemoryRegion) early-returns BEFORE any ref tuple is recorded. This ordering invariant is comment-documented but IMPLICIT: a future refactor moving dup-detection after ref-recording would silently change which error a user sees first on a multi-fault schedule — and the determinism gate would NOT catch it (it only covers valid input; this is an Err-path property). Add an explicit guard so the invariant cannot silently break.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 An explicit mechanism pins the invariant: either a debug_assert/structural guard that ref-recording happens only after the dup guards, OR a regression test feeding a multi-fault schedule (a duplicate worker AND an unknown worker-class) asserting the SAME error fires first as the documented behaviour, so a refactor reordering dup-detection vs ref-recording fails loudly
- [x] #2 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); no behaviour change
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add regression test: multi-fault schedule with a duplicate worker AND an unknown worker-class. Assert DuplicateWorker fires first (documented dup-before-ref-recording ordering). Chosen over debug_assert: a runtime assert cannot meaningfully express "ref-recording happens only after dup guards" (it is a control-flow ordering, not a state predicate); a regression test fails loudly if a refactor reorders.
2. Full gate; no behaviour change.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED. Decision: REGRESSION TEST (chosen over debug_assert). Rationale: the invariant is a control-flow ORDERING between two passes (pass-1 dup-detection early-return vs post-collection ref-validation), not a single-point state predicate — a runtime assert cannot phrase "ref-recording happens only after the dup guards" without re-encoding the very ordering it would guard. A regression test fails loudly on a reorder; the determinism gate cannot (Err-path only).

Added dup_worker_beats_unknown_class_pins_ref_recording_ordering (tests/sched_lower.rs). Multi-fault schedule (all typed-form — typed/simple cannot mix in one {}; first probe used mixed form and hit a ParseError, fixed to all-typed with a declared `core` class): `workers = { fe : missing_class, host : core, host : core }`. The FIRST entry references undeclared class missing_class (would be UnknownWorkerClass if ref-recording/validation ran); a LATER entry duplicates host. Asserts DuplicateWorker("host") fires first AND explicitly !matches UnknownWorkerClass (strength guard). Pins: pass-1 DuplicateWorker early-returns (sched/lower.rs ~178/186) BEFORE worker_class_refs.push (~217) and before the post-collection unknown-class validation ever runs — the exact invariant TASK-0196 option-(b) ordering-equivalence proof depends on.

AC#2 no-behaviour-change: pure test addition; determinism byte-identical 30/26/0/4 x2.

GATE: dup_worker_beats_unknown_class_pins_ref_recording_ordering PASSES; just test 0 failed; e2e 30/26/0/4/0; clippy --all-targets clean; ci exit 0. Commit 78f266c.

FEED-FORWARD: guard-vs-test -> chose test (ordering property, not state predicate). Exact invariant pinned: DuplicateWorker early-return precedes ref-recording+validation, so worker-name uniqueness holds at validation time (the precondition of TASK-0196 first-error ordering equivalence). GOTCHA: typed and simple worker-entry forms cannot mix in one { } — multi-fault worker tests must be all-typed.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all 4 batch tasks, no NO-GO, no follow-up required (each independently verifiable, not a faked batch-Done). qa-test-runner: workspace 425/0; 3 new tests pass by name; determinism byte-identical x2 + e2e EXACTLY 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; no new panic on user paths. mped-architect (independently verified, not trusting self-report): TASK-0194 enum-removal AIRTIGHT — Expr::Ident genuinely parser-unreachable (ident_or_call routes bare ident via index_tail .repeated() to Expr::LValue empty-indices, never Expr::Ident; ZERO construction sites repo-wide), only the 4 dead delegating arms removed (the 4 helpers stay LIVE via the surviving LValue arms), NO forced _ wildcard anywhere (clean cargo check = Rust proves exhaustiveness), IrExpr::Ident is a DISTINCT live type untouched; TASK-0193 generalized message literally true for sync+async AND sync,sync AND async,async (no residual overclaim), grammar-sched.md notes 5/7/§5.3 now EXHAUSTIVE + verbatim-matching the shipped message (the recurring comment/doc-lie class NOT repeated; old "both sync and async" §5.3 quote fully removed), variant payload unchanged so no test migration/strength preserved; TASK-0195 genuinely exercises the SYNTHETIC-label NonIntegerShapeExpr path (decl=="<index/loop-bound expression>") asserting Some(span) at the source-recomputed offset (4,14) — closes the TASK-0090 located-vs-position-less boundary both ways; TASK-0197 a genuine control-flow ordering pin (multi-fault dup-worker+unknown-class asserts DuplicateWorker first + !matches UnknownWorkerClass) that WOULD fail if ref-recording moved before the dup guard — constrains the TASK-0196-equivalence invariant, not tautological. decision-0003 upheld (only non-comment src addition is the Display string); scope clean (0193 did NOT widen TASK-0086 option-span; 0194 algo-only no IR/codegen). Per-task Done honest. This cycle converted 4 review-surfaced filed follow-ups into verified Done (good graph hygiene). Task Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a regression test pinning the dup-before-ref-recording ordering invariant that SchedLowerError Err-path first-error ordering (TASK-0196 option (b)) depends on.

Changes:
- tests/sched_lower.rs: dup_worker_beats_unknown_class_pins_ref_recording_ordering. Multi-fault schedule (unknown worker-class on the first typed entry + duplicate worker later) asserts DuplicateWorker fires first and explicitly !matches UnknownWorkerClass.

Decision: regression test over debug_assert — the property is a control-flow ordering between two passes, not a single-point state predicate; a runtime assert would have to re-encode the ordering it guards.

No behaviour change (pure test addition); determinism byte-identical x2.

Gate: just test 0 failed; e2e 30/26/0/4/0; determinism byte-identical 30/26/0/4 x2; clippy --all-targets clean; ci exit 0. Commit 78f266c.
<!-- SECTION:FINAL_SUMMARY:END -->
