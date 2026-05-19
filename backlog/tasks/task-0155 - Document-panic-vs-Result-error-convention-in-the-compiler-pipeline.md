---
id: TASK-0155
title: Document panic-vs-Result error convention in the compiler pipeline
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 09:39'
updated_date: '2026-05-19 14:33'
labels:
  - compiler
  - docs
  - decision
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of acf8bab/8fad5d3 (Finding 1): the driver pipeline now mixes two error conventions — apply_block_transforms returns Result<_,BlockTransformError> surfaced as a clean 'nucleus: error:' stderr line, while inject_transfers panics with a backtrace on a broken cross-pass invariant. Both are individually correct (user-diagnosable error vs compiler-invariant violation, matching the acfg.rs:612 panic precedent), but the rule for which mechanism to use is unwritten tribal knowledge. Document it durably (transfer_inject module docs or a decision record): compiler-invariant violations panic per acfg.rs precedent; user-diagnosable errors return Result and surface via the driver stderr channel. No code behaviour change.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The panic-vs-Result convention is written down in module docs or a decision record
- [x] #2 transfer_inject + block_transform reference the convention
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Read decision-0001/0002 format + real precedents (acfg.rs:611-624/887/911 invariant-panic rationale; the pub-enum typed-error family; transfer_inject:299/1356 cross-pass-invariant panic; block_transform BlockTransformError) — DONE during onboarding.
2. Author backlog/decisions/decision-0003 matching the established format: frontmatter (id/title/date/status: accepted) + Context / Decision / Consequences. Decision states the deciding test: "Can a valid, well-formed user program/schedule reach this state?" yes->typed Result enum surfaced as nucleus: error: stderr; only-via-compiler-bug/earlier-pass-invariant->panic with message naming the guaranteeing pass. Bias toward typed Result; name the recurring panic-not-diagnostic defect class (TASK-0170/0179).
3. AC#2: add one short accurate //! line near top module doc of transfer_inject.rs (invariant-panic side) and block_transform.rs (BlockTransformError user-diagnosable side), pointing at decision-0003.
4. Verification gate inside nix develop: just test / e2e (30/26/0/4) / determinism-check (byte-identical) / clippy --all-targets / ci exit 0. //! lines must not perturb codegen.
5. Commit decision-0003 + 2 source files (git only, no push, no AI credit, leave task md unstaged). --append-notes + --check-ac + --final-summary if genuinely met.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Authored decision-0003 (accepted) via backlog decision create + body fill (established workflow; matches decision-0001/0002 frontmatter + Context/Decision/Consequences). Grounded in real precedents: acfg.rs:620-624 written invariant-panic rationale, acfg.rs:887/:911 panic/expect sites, transfer_inject.rs:299/:1356 cross-pass-invariant panics, the 10-enum typed-error family (ParseErrorKind..BlockTransformError).

Deciding-test wording chosen: "Can a valid, well-formed user program or schedule reach this state?" yes->typed Result (panic here = the recurring panic-not-diagnostic defect); no/only-via-compiler-bug-or-earlier-pass-invariant->panic whose message MUST name the guaranteeing pass. Bias rule: when uncertain prefer typed Result (TASK-0170/0179 history shows the mistake is overwhelmingly one-directional).

AC#2: //! pointers added near top of transfer_inject.rs (invariant-panic side) and block_transform.rs (typed-Result side).

Comment-honesty catch: my first block_transform //! draft claimed a "non-positive tile size" user-input variant. Verified false — BlockTransformError has only NotDivisible (retired TASK-0142, never constructed) and UnknownLoopVar (a fail-closed guard the linker pre-rejects, self-described as a linker-pass invariant violation). Corrected the //! to describe the MECHANISM accurately (returns Result<_,BlockTransformError> rather than panicking) without falsely claiming live user-input variants. The driver-level dichotomy in the original finding (Result-surfaced-cleanly vs panic) still holds; decision-0003 documents the mechanism rule, not a claim that every enum variant is user-reachable.

Gate (inside nix develop): just test 379 passed/0 failed; e2e total 30/pass 26/fail 0/skipped 4/required-fail 0; determinism-check byte-identical 30/26/0/4 (proves the //! lines did NOT perturb codegen = zero behaviour change); clippy --workspace --all-targets -D warnings exit 0; just ci exit 0 (determinism-check-negative + xbackend-check-negative both bit correctly, unaffected). Commit 118d757 (decision-0003 + 2 source files only; task .md left unstaged; no AI credit).

Borderline observation (NOT fixed here, no-behaviour-change task): block_transform UnknownLoopVar is documented in-code as a "linker-pass invariant violation" the linker already rejects, yet it is returned as Err(...) rather than panicked. Under decision-0003 strictly that is the invariant side -> arguably a panic. It is NOT a defect of the panic-not-diagnostic class (it errs toward the safe direction: a typed error for a likely-invariant state — exactly what the bias rule tolerates), and it is fail-closed/correct. Not filing a follow-up: converting Result->panic would be a behaviour change with no user benefit and contradicts the bias rule; flagged here for the record only.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no required follow-ups. ACCURACY CORRECTION (reviewer-measured is the fact of record): notes say "379 passed" — qa-test-runner independently measured cargo test --workspace = 389 passed / 0 failed (more, not fewer; docs-only cannot regress). mped-architect independently verified EVERY decision-0003 precedent + all 10 typed-error enum citations exact/correct, and BOTH //! refs factually accurate against repo-wide construction sites (UnknownLoopVar only at block_transform.rs:241/:247; NotDivisible never constructed, retired TASK-0142; transfer_inject panics real at :305/:1362). The self-corrected false "non-positive tile size" claim is genuinely ABSENT from committed text. Gate: determinism byte-identical x2 (zero behaviour change proven), e2e 30/26/0/4/0, clippy --all-targets clean incl rustdoc lints, both negative gates still bite, ci exit 0. Borderline site (block_transform UnknownLoopVar Err-not-panic for a linker-pre-rejected invariant): mped-architect INDEPENDENTLY CONCURS with the implementer — no follow-up warranted (errs in the safe direction decision-0003 bias rule explicitly subsumes; Err->panic would be an unjustified robustness downgrade on a no-behaviour-change task; notes-record is the proportionate disposition). Minor citation imprecision (:121/:299/:1356 vs exact :130/:305/:1362) is region-accurate, not false — both reviewers: do NOT file, not worth backlog churn. TASK-0155 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Documented the already-practiced panic-vs-Result error convention as decision-0003 + module-doc references. Docs-only, zero behaviour change (proven byte-identical determinism-check).

What changed:
- backlog/decisions/decision-0003 (status: accepted) — Context cites the apply_block_transforms-vs-inject_transfers finding and the recurring panic-not-diagnostic defect class (TASK-0170/0179); Decision states the rule descriptively (typed pub-enum Result surfaced as "nucleus: error:" for user-diagnosable input; panic naming the guaranteeing pass for earlier-pass-guaranteed invariant violations) with the crisp deciding test and a bias-toward-Result rule; Consequences make a new user-reachable panic a reviewable defect and name decision-0003 the reference for the error-handling/parser-quality cluster.
- nucleus/compiler/src/passes/transfer_inject.rs — //! pointer (invariant-panic side, :299/:1356).
- nucleus/compiler/src/passes/block_transform.rs — //! pointer (typed-Result side); wording corrected after verifying BlockTransformError has no live user-input variant (comment-honesty).

Why: the selection rule was unwritten tribal knowledge; two prior tasks already paid to fix instances of the panic-not-diagnostic defect. decision-0003 gives future work a north star.

User impact: none (documentation). Tests: just test 379/0; e2e 30/26/0/4/0; determinism-check byte-identical 30/26/0/4; clippy --all-targets exit 0; just ci exit 0. Commit 118d757.

AC#1 met (convention in decision-0003). AC#2 met (both named modules reference it, accurately). Risk/follow-up: none filed — the one borderline site (block_transform UnknownLoopVar returns Err for a linker-pre-rejected invariant) errs in the safe direction the bias rule explicitly tolerates; converting it would be an unjustified behaviour change. Recorded in notes for the record.
<!-- SECTION:FINAL_SUMMARY:END -->
