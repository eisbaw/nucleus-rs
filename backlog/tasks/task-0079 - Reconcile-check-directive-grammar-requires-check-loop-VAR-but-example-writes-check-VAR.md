---
id: TASK-0079
title: >-
  Reconcile check directive: grammar requires 'check loop VAR' but example
  writes 'check VAR'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:49'
updated_date: '2026-05-19 14:53'
labels:
  - M0
  - docs
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
docs/grammar-sched.md §4.3 documents a divergence: PRD §6.3.5 and the EBNF specify 'check loop VAR : ...;', but examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc line 105 writes 'check frame : latency_max = 10ms;' without the 'loop' keyword.

Resolve one way:
  (a) Relax grammar: make 'loop' optional after 'check'. Cheap, matches example, but blocks future per-transfer 'check' variants from being unambiguous.
  (b) Fix example: add 'loop' keyword in embedded_multimcu.sched.nuc. Keeps grammar/PRD aligned, preserves room for future 'check transfer X : ...;' syntax.

Recommendation: (b). Future PRD §6.3.5 work (buffer_max, jitter_max) wants the 'loop'/'transfer' qualifier slot to remain distinct.

Acceptance:
- One of (a) or (b) is implemented.
- docs/grammar-sched.md §4.3 is updated to remove the KNOWN DIVERGENCE notice.
- Decision is recorded in the commit message and in the doc.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc uses the grammar-conformant 'check loop <var> : ...;' form (the 'loop' qualifier present), per PRD §6.3.5 / the schedule EBNF (option b)
- [x] #2 docs/grammar-sched.md §4.3 KNOWN DIVERGENCE notice is removed; the doc now states grammar, PRD, and example are aligned
- [x] #3 The decision (option b chosen to preserve the loop/transfer qualifier slot for future check transfer/buffer_max/jitter_max) is recorded in docs/grammar-sched.md and the commit message
- [x] #4 The edited schedule file parses successfully under the schedule parser with no regression to the existing test/e2e gate
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Onboard: parser enforces `check loop IDENT` (loop NOT optional); old `check frame` REJECTED (proven by existing known_failing test). `frame` IS a loop var (line 78). One check directive (line 105). Example 14 excluded from e2e matrix; only sched_parser.rs references it.
2. AC#1: edit embedded_multimcu.sched.nuc line 105 `check frame :` -> `check loop frame :` (preserve metric/value exactly).
3. AC#2: docs/grammar-sched.md §4.3 — remove KNOWN DIVERGENCE notice; state grammar/PRD/example aligned. Update §4.2 table row 105. Update §4 design-question item 4.
4. AC#3: record option-b rationale (qualifier slot for future check transfer/buffer_max/jitter_max) in §4.3 and commit body.
5. AC#4: flip sched_parser.rs known_failing test -> positive parses_ test asserting Ok + count_checks==1. Update module header comment.
6. Gate inside nix develop: just test / e2e (30/26/0/4) / determinism / clippy / ci. Report actual numbers + AC#4 parse evidence (old Err vs new Ok).
7. Commit (no push, no AI credit), scoped msg + rationale body. Notes + check-ac only gate-verified. -s Done only if all 4 green.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0079 implemented via option (b) — example fixed, grammar NOT relaxed.

PARSER ACTUAL ENFORCED GRAMMAR (onboarding finding): check_directive() in nucleus/compiler/src/sched/parser.rs:589 mandates `check loop IDENT : ...` — the `loop` keyword is NOT optional (.ignore_then(pad(keyword("loop")))). The OLD form `check frame : ...` was REJECTED by the parser (proven by the pre-existing known_failing_14_hearing_aid_embedded_multimcu_pending_task_0079 test asserting expect_err). So option (b) was the correct reconciliation: it makes the example conformant without spending the qualifier-slot disambiguation budget.

KEY FACTS: `frame` IS a genuine loop variable (line 78: `loop frame : pipeline=3;`), so `check loop frame` is semantically correct — the var-is-loop-var assumption HELD. Exactly ONE check directive in the example (line 105). Example 14 embedded_multimcu is NOT in the e2e matrix nor the lower/link/acfg positive suites (excluded by scope — far-future M11 multi-MCU); only sched_parser.rs referenced it.

CHANGES: (1) example line 105 `check frame :` -> `check loop frame :` (metric/value unchanged). (2) docs/grammar-sched.md §4.3 KNOWN DIVERGENCE notice removed -> aligned statement + option-b rationale; §4.2 table row 105 + §4 design-question item 4 updated. (3) parser.rs doc-comment updated. (4) sched_parser.rs: flipped known_failing test -> positive parses_14_hearing_aid_embedded_multimcu (asserts Ok, count_checks==1, var==frame, latency 10ms); ADDED negative_check_without_loop_qualifier_is_rejected (pins option-b: bare `check VAR` rejected at line 5). (5) stale "failing parse" comments in link.rs/acfg.rs/sched_lower.rs corrected to "excluded by scope" + point at follow-up TASK-0192.

FOLLOW-UP FILED: TASK-0192 — bring embedded_multimcu into lower/link/ACFG matrix when M11 multi-MCU lowering is in scope (it now parses; the suites still exclude it by scope, documented not silent).

GATE (actual, inside nix develop): just test = 390 passed / 0 failed / 2 ignored. just e2e = total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0. determinism-check = byte-identical 30/26/0/4; determinism-check-negative bit (26 perturbed >=1); xbackend-check-negative bit (1 detected >=1). just clippy --workspace --all-targets -D warnings = clean (exit 0). just ci = exit 0 (its internal pass:0/fail:26 and pass:25/fail:1 blocks are the negative sub-stages biting as designed, each followed by "OK: ... correctly bit").

AC#4 EVIDENCE: negative_check_without_loop_qualifier_is_rejected (OLD form `check frame` -> Err at line 5) + parses_14_hearing_aid_embedded_multimcu (NEW form -> Ok) both green — reconciliation evidenced in-tree, not asserted.

ORCHESTRATOR review-gate close (phase3-ralph): qa-test-runner clean GO (all gate numbers verified: 390/0 tests, e2e 30/26/0/4/0, determinism byte-identical, both negatives bite, clippy --all-targets clean, ci exit 0; reconciliation test pair real; frame genuinely a loop var; TASK-0192 well-formed). mped-architect GO with one genuinely-needed finding: the stale-reference sweep was INCOMPLETE — PRD.md:564 §6.3.5 normative example still showed the rejected bare "check frame : latency_max = 10ms;", an internal PRD self-contradiction (vs its own grammar at PRD:542 "check loop VAR"), which made the new docs/grammar-sched.md §4.3 "Grammar, PRD §6.3.5, and the examples are in agreement" claim itself a doc-lie (the recurring comment-lie defect class, recursively). ORCHESTRATOR APPLIED THE FIX IN-THREAD (phase3-ralph step 4, small precise doc-overclaim): PRD.md:564 -> "check loop frame : latency_max = 10ms;" (semantically true — PRD:563 declares "loop  frame : pipeline=3;"). Verified zero behaviour change: PRD.md is only a repo-root .exists() marker (determinism.rs:33/concurrency_stress.rs:56), never parsed for content; cargo test --workspace 390/0 unchanged; determinism byte-identical 30/26/0/4; clippy --all-targets clean; grep confirms no bare "check frame" remains in PRD so §4.3 alignment claim is now genuinely TRUE. AC#2 now fully honest. TASK-0079 Done stands (no AC re-gamed — the in-thread fix makes the existing doc claim true rather than weakening it).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Reconciled the `check`-directive grammar-vs-example divergence via option (b): fixed the example, did NOT relax the grammar.

What changed:
- examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc line 105: `check frame : latency_max = 10ms;` -> `check loop frame : latency_max = 10ms;` (metric/value unchanged; `frame` is a genuine loop var per line 78).
- docs/grammar-sched.md: removed the §4.3 KNOWN DIVERGENCE notice, replaced with an alignment statement + the option-b decision rationale; updated §4.2 table row 105 and §4 design-question item 4.
- nucleus/compiler/src/sched/parser.rs: updated check_directive doc-comment (no behaviour change — the parser already mandated `loop`).
- nucleus/compiler/tests/sched_parser.rs: flipped the pre-existing known-failing test into a positive parses_14_hearing_aid_embedded_multimcu test, and added negative_check_without_loop_qualifier_is_rejected (pins that bare `check VAR` is still rejected) — together these are the AC#4 reconciliation evidence (old form Err, new form Ok), in-tree and permanent.
- nucleus/compiler/tests/{link,acfg,sched_lower}.rs: corrected stale "failing parse" exclusion comments to "excluded by scope" (M11 multi-MCU; now parses cleanly) pointing at follow-up TASK-0192.

Why option (b): the word after `check` is a qualifier slot; PRD §6.3.5 anticipates future `check transfer X : buffer_max = N;` / jitter_max / throughput_min. Keeping `loop` mandatory keeps that future syntax unambiguous with no grammar break. Relaxing now would spend the disambiguation budget for a one-char convenience.

Parser actual enforced grammar (verified): `check loop IDENT : ...` — `loop` is mandatory, not optional. The old `check frame : ...` form was rejected before this change (pinned by the old known-failing test); it is still rejected (pinned by the new negative test). Reconciliation is evidenced, not asserted.

Gate (actual): just test 390 passed / 0 failed / 2 ignored; just e2e total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0 (unchanged — embedded_multimcu is not in the e2e/lower/link/acfg matrices); determinism-check byte-identical 30/26/0/4; determinism-check-negative + xbackend-check-negative both still bite; clippy --workspace --all-targets -D warnings clean; just ci exit 0.

Follow-up filed: TASK-0192 — bring embedded_multimcu into the lower/link/ACFG matrix once M11 multi-MCU lowering is in scope (currently a documented scope exclusion, not silent).

Commit: 64482f3 (not pushed). Backlog tracker files left unstaged for the orchestrator.
<!-- SECTION:FINAL_SUMMARY:END -->
