---
id: TASK-0094
title: 'SchedIR: detect place_set with duplicate worker names'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:33'
updated_date: '2026-05-19 15:14'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
place k on { w0, w0 } is currently accepted by lower_sched (TASK-0010). Either reject as a hard error or fold to a unique set. PRD sec 6.3.2 doesn't speak to the duplicate case; pick a rule and enforce it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A rule for 'place k on { w0, w0 }' is decided (reject-as-hard-error recommended over silent fold, per fail-fast + decision-0003) and recorded (commit message + a code comment and/or grammar-sched.md/PRD note)
- [x] #2 SchedIR lowering enforces the rule with a typed error (decision-0003: NOT a panic, NOT silent acceptance unless the decision is explicitly fold-to-unique)
- [x] #3 Negative test pins the duplicate-worker rejection; valid place sets still lower; no e2e/determinism regression
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. DECISION: reject-as-hard-error (fail-fast + decision-0003). A duplicate worker in a place set is a user typo; silent fold hides the mistake and the user never learns the set was not what they wrote. Record in commit body + code comment + grammar-sched.md note.
2. Add SchedLowerError::DuplicatePlaceWorker{kernel,worker} + Display.
3. In lower_place PlaceTarget::Many: track seen set, reject on repeat (before the existing undeclared-worker check so the message is specific).
4. Negative test: place k on { w0, w0 }; positive: place k on { w0, w1 } still lowers.
5. Full gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented. DECISION: reject-as-hard-error (NOT silent fold-to-unique). Rationale: a repeated worker in a distributed placement set is a user mistake; folding silently would change the placement the user wrote without telling them — violates fail-fast and decision-0003 (user-diagnosable input -> typed Result). PRD 6.3.2 was silent; rule now documented in: (a) code comment at the check site in lower.rs, (b) grammar-sched.md sec.2 note 10, (c) this commit body. Not tribal anymore.
SchedLowerError::DuplicatePlaceWorker{kernel,worker} + Display. Check in lower_place PlaceTarget::Many, BEFORE the undeclared-worker check so the duplicate gets its specific message even if the repeated name is also undeclared.
Gate: just test 399/0. e2e 30/26/0/4 req-fail 0. determinism byte-identical 30/26/0/4. det-neg + xbackend-neg bite. clippy clean. ci exit 0.
E2E driver evidence: `place add on { w0, w0 }` -> nucleus: error: schedule lower error: `place add` lists worker `w0` more than once in its placement set. No panic.
Regression-grep: all place-set forms in nuc-nucleus/examples/ are { w0,w1,w2,w3 } distinct — none trip the rule. Commit 07af8fc.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all three batch tasks genuinely Done & gate-substantiated. qa-test-runner: workspace 399/0 (sched_lower 43/43, 4 neg + 5 pos by name); e2e EXACTLY 30/26/0/4/0; determinism byte-identical + both negatives bite; clippy --all-targets clean; ci exit 0; 4 rejections proven end-to-end via the real driver (clean nucleus: error: lines, no panic/backtrace); regression INDEPENDENTLY grep-verified across all 20 example schedules (none trip a new rule; 14-hearing-aid accessible_by names confirmed declared worker_class). mped-architect: rules grounded EXACTLY in grammar EBNF/notes (single-valued set = AST LoopOption/TransferOption exhaustively, not over-broad); silent-grammar interpretation (bare reuse idempotent) documented in note 7 + code + positive test, not tribal; zero panic/unwrap/expect on any user-reachable check path (decision-0003 compliant); 0094 reject-as-hard-error sound + recorded x3 + ordering verified by inspection; grammar-sched.md notes 4/7/10 independently verified to match shipped code (no doc-lie); per-task Done honest, ACs map 1:1 to committed code, not retrofitted-loose; disclosed test-bug fix was correctly to the test (production code right); pre-existing limitations (no sched-AST spans; first-violation-only) correctly attributed as out-of-scope not regressions. Two reviewer findings BOTH explicitly optional/non-blocking/low-priority (ConflictingTransferMode message imprecise-but-disclosed on sync,sync path — adjudicated NOT a doc-lie; optional place-worker-ordering test) filed as TASK-0193 (dep TASK-0093) rather than scope-crept into this deep-context cycle. Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
`place k on { w0, w0 }` is now a hard error (SchedLowerError::DuplicatePlaceWorker), not a silent fold.

Decision & rationale: reject-as-hard-error over silent fold-to-unique. A duplicate worker in a distributed placement is a user typo; a silent fold changes the placement without informing the user (fail-fast; decision-0003). PRD 6.3.2 was silent on the duplicate case — the chosen rule is now recorded in a code comment at the check site, in grammar-sched.md sec.2 note 10, and in commit 07af8fc.

Changes:
- ir.rs: + DuplicatePlaceWorker{kernel,worker} + Display.
- lower.rs: duplicate-detection in lower_place PlaceTarget::Many, ordered before the undeclared-worker check for a specific message.
- sched_lower.rs: negative test (place k on { w0, w0 }) + positive test (place k on { w0, w1 } lowers).
- grammar-sched.md: sec.2 note 10 documents the rule.

Tests: just test 399/0; e2e 30/26/0/4; determinism byte-identical; clippy/ci clean. All existing example place-sets are distinct (regression-grepped). Driver evidence captured.
<!-- SECTION:FINAL_SUMMARY:END -->
