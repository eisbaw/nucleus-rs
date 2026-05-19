---
id: TASK-0193
title: >-
  SchedLowerError diagnostic-precision: distinguish duplicate-mode from
  sync/async conflict + pin place-worker ordering test
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 15:14'
updated_date: '2026-05-19 18:29'
labels:
  - M0
  - compiler
  - ir
  - diagnostics
dependencies:
  - TASK-0093
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Non-blocking review-gate findings from the TASK-0093/0094/0095 batch (both reviewers GO; both findings explicitly optional/low-priority). (1) ConflictingTransferMode Display says "transfer X is both sync and async" but the same variant also fires on sync,sync / async,async — literally imprecise on the duplicate-mode path. It is DISCLOSED in the variant doc-comment + grammar-sched.md note 7 + commit body (mped-architect adjudicated: recorded, NOT a doc-lie, honest-because-disclosed), so this is precision polish not a correctness/honesty defect. Fix: generalize the Display to be true for both paths (e.g. "transfer X must specify exactly one of sync/async") OR split into a distinct DuplicateTransferMode variant; update grammar-sched.md note 5/7 to document both paths exhaustively (note 5 currently documents only the sync+async case — qa-test-runner P3). (2) Test-hardening: negative_duplicate_place_worker uses declared workers so it does not exercise the duplicate-AND-undeclared ordering path the code comment at lower.rs:234-243 documents (DuplicatePlaceWorker fires before UnknownPlaceWorker). Add a test place k on { ghost, ghost } (ghost undeclared) asserting DuplicatePlaceWorker (not UnknownPlaceWorker) to pin that documented ordering guarantee.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 ConflictingTransferMode diagnostic is literally accurate on BOTH the sync+async and the sync,sync/async,async paths (generalized message or split variant); grammar-sched.md notes 5/7 document both paths
- [x] #2 A negative test feeds 'place k on { ghost, ghost }' (ghost undeclared) and asserts DuplicatePlaceWorker (not UnknownPlaceWorker), pinning the documented dup-before-undeclared ordering
- [x] #3 Full gate green (test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); no SchedLowerError behavioural regression for valid schedules
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Generalize ConflictingTransferMode Display to be true for both sync+async and sync,sync/async,async paths (e.g. "transfer X must specify exactly one of sync/async"). Chosen over splitting a DuplicateTransferMode variant: smaller, no variant churn, no test migration, fully honest for both paths.
2. Update variant doc-comment + grammar-sched.md notes 5 and 7 to document BOTH paths exhaustively (note 5 was non-exhaustive per TASK-0093 review).
3. Add negative test: place k on { ghost, ghost } (ghost undeclared) asserting DuplicatePlaceWorker (not UnknownPlaceWorker), pinning dup-before-undeclared ordering.
4. Full gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED. Decision: GENERALIZED the Display (not split into DuplicateTransferMode) — smaller, no SchedLowerErrorKind shape change, no test migration, fully honest for BOTH paths. New message: "transfer `X` must specify exactly one of `sync` or `async`; they are mutually exclusive and neither may be repeated". Variant doc-comment + grammar-sched.md notes 5 AND 7 (and the stale §5.3 message quote) rewritten to document BOTH the conflict and repeated-mode paths exhaustively (note 5 was non-exhaustive per TASK-0093 review). Existing negative_mutually_exclusive_transfer_sync_async asserts on err.kind (the variant, unchanged) NOT the Display string -> assertion strength preserved, no migration needed. Added negative_duplicate_place_worker_beats_undeclared: `place k on { ghost, ghost }` (ghost undeclared) asserts DuplicatePlaceWorker AND explicitly !matches UnknownPlaceWorker (strength guard) — pins the dup-before-undeclared scan ordering.

AC EVIDENCE (real lower_sched run): sync,async / sync,sync / async,async ALL emit the byte-identical generalized message (single non-branching write!; all three reach ConflictingTransferMode via mode_flags>1 at sched/lower.rs:670).

GATE: just test 0 failed (new ordering test + all suites pass); e2e 30/26/0/4/0; determinism byte-identical 30/26/0/4 x2; determinism-check-negative 26 perturbed (bites); xbackend-check-negative 13 corrupted/1 detected (bites); clippy --workspace --all-targets -D warnings clean; ci exit 0. Commit 78f266c.

FEED-FORWARD: generalize-vs-split chosen generalize; no SchedLowerErrorKind shape change so no downstream sched-error task impact. clippy::doc_lazy_continuation gotcha: a markdown `*` list in a rustdoc comment followed by prose triggers "doc list item without indentation" under -D warnings — used plain prose instead of a bullet list.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all 4 batch tasks, no NO-GO, no follow-up required (each independently verifiable, not a faked batch-Done). qa-test-runner: workspace 425/0; 3 new tests pass by name; determinism byte-identical x2 + e2e EXACTLY 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; no new panic on user paths. mped-architect (independently verified, not trusting self-report): TASK-0194 enum-removal AIRTIGHT — Expr::Ident genuinely parser-unreachable (ident_or_call routes bare ident via index_tail .repeated() to Expr::LValue empty-indices, never Expr::Ident; ZERO construction sites repo-wide), only the 4 dead delegating arms removed (the 4 helpers stay LIVE via the surviving LValue arms), NO forced _ wildcard anywhere (clean cargo check = Rust proves exhaustiveness), IrExpr::Ident is a DISTINCT live type untouched; TASK-0193 generalized message literally true for sync+async AND sync,sync AND async,async (no residual overclaim), grammar-sched.md notes 5/7/§5.3 now EXHAUSTIVE + verbatim-matching the shipped message (the recurring comment/doc-lie class NOT repeated; old "both sync and async" §5.3 quote fully removed), variant payload unchanged so no test migration/strength preserved; TASK-0195 genuinely exercises the SYNTHETIC-label NonIntegerShapeExpr path (decl=="<index/loop-bound expression>") asserting Some(span) at the source-recomputed offset (4,14) — closes the TASK-0090 located-vs-position-less boundary both ways; TASK-0197 a genuine control-flow ordering pin (multi-fault dup-worker+unknown-class asserts DuplicateWorker first + !matches UnknownWorkerClass) that WOULD fail if ref-recording moved before the dup guard — constrains the TASK-0196-equivalence invariant, not tautological. decision-0003 upheld (only non-comment src addition is the Display string); scope clean (0193 did NOT widen TASK-0086 option-span; 0194 algo-only no IR/codegen). Per-task Done honest. This cycle converted 4 review-surfaced filed follow-ups into verified Done (good graph hygiene). Task Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Generalized the ConflictingTransferMode diagnostic to be literally accurate on BOTH the sync+async-conflict and the repeated-mode (sync,sync / async,async) paths, and pinned the dup-before-undeclared place-worker scan ordering.

Changes:
- sched/ir.rs: Display generalized to "transfer `X` must specify exactly one of `sync` or `async`; they are mutually exclusive and neither may be repeated"; variant doc-comment rewritten (both paths, non-list prose to satisfy clippy::doc_lazy_continuation).
- docs/grammar-sched.md: notes 5 + 7 rewritten exhaustively; stale §5.3 message quote corrected.
- tests/sched_lower.rs: added negative_duplicate_place_worker_beats_undeclared (asserts DuplicatePlaceWorker + !matches UnknownPlaceWorker).

Decision: generalize over split-variant — smaller, no SchedLowerErrorKind shape change, no test migration, honest for both. Existing sync/async test asserts err.kind (unchanged variant) so strength preserved.

Evidence: real lower_sched run shows byte-identical accurate message for sync,async / sync,sync / async,async.

Gate: just test 0 failed; e2e 30/26/0/4/0; determinism byte-identical 30/26/0/4 x2; both negative gates bite; clippy --all-targets clean; ci exit 0. Commit 78f266c.
<!-- SECTION:FINAL_SUMMARY:END -->
