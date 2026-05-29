---
id: TASK-0192
title: Bring 14-hearing-aid/embedded_multimcu into the lower/link/ACFG test matrix
status: Done
assignee:
  - '@Claude'
created_date: '2026-05-19 14:39'
updated_date: '2026-05-29 05:59'
labels:
  - M11
  - test
  - language
dependencies:
  - TASK-0054.01
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0079 made embedded_multimcu.sched.nuc parse cleanly (added the grammar-conformant 'check loop' qualifier). It is currently excluded from the sched_lower / link / acfg / transfer_inject / sync_inject positive test matrices by scope (it is a far-future M11 multi-MCU schedule, not part of the M3 matrix). Once the M11 multi-MCU lowering path is in scope, add this schedule to those positive suites and remove the scope-exclusion comments (which currently point here). Until then this is a deliberate, documented gap, not a regression.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 embedded_multimcu.sched.nuc is included in the sched_lower / link / acfg positive matrices (or a documented reason why a given suite still excludes it)
- [x] #2 The scope-exclusion comments in link.rs/acfg.rs/sched_lower.rs that reference this task are updated or removed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Depends on TASK-0054.01 (per-frame embedded algo). Probe outcome (throwaway test, recorded in notes):
- sched_lower: ADMITS the embedded_multimcu schedule cleanly (3 workers, 6 places, 4 transfers, 1 loop, 1 check) — the complex shape (worker classes, memory_region/place_data, pipeline=3, async/buffer transfers, check loop) all lowers.
- link: ORIGINALLY REJECTED with PipelineExceedsBuffer{depth=3, buffer=2} on mic_in/bt_in/spk_out/bt_out. Root cause = a LATENT EXAMPLE BUG: the schedule (initial commit 2cc372c, before the PipelineExceedsBuffer link invariant existed) declared buffer=2 while pipeline=3 — internally inconsistent (its OWN prose says 'three frames in flight'). Every other passing pipelined example pairs pipeline=D with buffer=D (09:4/4, 11:2/2, 13:3/3). FIX (not a workaround, not a test-weakening): raise buffer 2->3 to match documented pipeline depth + add a comment recording the de-risk finding.
- After fix: link OK, acfg OK (7 ops, 1 repeat, depth 1 — same shape as tier-1 naive's repeat).
Plan:
1. Raise buffer 2->3 on the 4 transfers in embedded_multimcu.sched.nuc (link-invariant-driven, documented inline).
2. Add positive tests: lowers_14_hearing_aid_embedded_multimcu (sched_lower.rs), links_14_hearing_aid_embedded_multimcu (link.rs, algo=prog.embedded.algo.nuc), acfg_14_hearing_aid_embedded_multimcu (acfg.rs).
3. REMOVE the scope-exclusion comments in sched_lower.rs/link.rs/acfg.rs that reference TASK-0192 (AC#2); replace with a one-line 'admitted at TASK-0192' note.
4. Full gate; e2e baseline 301/246/0/55/0 must not regress (cells stay [[skip]]). Report actual numbers.
OUTCOME: all 3 suites ADMIT — no deferred gap task needed (AC#1's 'included in matrices' branch, not the 'documented reason to exclude' branch).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== REOPENED cycle (M11 entry reached) ===
The designed reopen trigger fired: TASK-0049.01 (M11 inter-MCU transport de-risk) is DONE, the interconnect is decided (UART hub), and the M11 codegen arc is now active. Per this task's own closure note ("Reopen at M11 entry; the scope-exclusion comments in link.rs/acfg.rs/sched_lower.rs that reference this task are the trigger marker"), this task is reopened. Depends on TASK-0054.01 (per-frame embedded algorithm) which must land first so the schedule's kernel references resolve. Slice goal: ATTEMPT to admit 14-hearing-aid/embedded_multimcu into sched_lower/link/acfg positive matrices; for each lowering gap discovered, FILE a precise deep-codegen follow-up subtask under TASK-0049 (this de-risks the M11 codegen into an ordered backlog). AC#1's "or a documented reason why a given suite still excludes it" branch is the honest outcome if deep gaps remain.

=== De-risk OUTCOME (cycle: M11 entry) — ALL THREE SUITES ADMIT ===
Probed sched_lower / link / acfg admission of embedded_multimcu × prog.embedded.algo.nuc (TASK-0054.01).
- sched_lower: ADMITS immediately. The complex shape (3 typed worker classes, 2 memory_region + place_data, pipeline=3, 4 async buffered notify=event transfers, check loop latency_max) all lowers. NO machinery missing.
- link: ORIGINALLY REJECTED — LinkError::PipelineExceedsBuffer{loop=frame, data=mic_in/bt_in/spk_out/bt_out, depth=3, buffer=2} (4 errors). ROOT CAUSE = a LATENT EXAMPLE BUG, not missing machinery: the schedule declared pipeline=3 but buffer=2, internally inconsistent. It has been so since the INITIAL commit (2cc372c), authored BEFORE the PipelineExceedsBuffer link invariant (TASK-0099/TASK-0134) existed. The schedule's own prose contradicts itself: topology comment says 'Three frames in flight' (intent=depth3) while the transfer comment said '2-deep buffering'. Every other passing pipelined example pairs pipeline=D with buffer=D (09:4/4, 11:2/2, 13:3/3 — example 11's comment even names 'the pipeline-depth-<=-buffer-capacity link gate').
- FIX (NOT a workaround / NOT a test-weakening): raised buffer 2 -> 3 on all 4 transfers to match the documented pipeline depth + the convention, with an inline NOTE recording the de-risk finding. After the fix: link OK.
- acfg: ADMITS after the link fix — 7 ops, 1 repeat, max depth 1 (all 7 per-frame statements live inside the single Repeat; contrast the naive shape's 4 top-level + 3-in-repeat = also 7).

AC STATUS:
- AC#1 (included in sched_lower/link/acfg positive matrices): MET. All three suites admit (the 'included' branch, NOT the 'documented reason to exclude' branch). No deferred gap task needed — the M11 lower/link/acfg path required ZERO new machinery; the only fix was the latent example-schedule inconsistency.
- AC#2 (scope-exclusion comments updated/removed): MET. The 3 'excluded by scope / follow-up TASK-0192' comments in sched_lower.rs (module + section), link.rs (module), acfg.rs (module) are removed and replaced with 'ADMITTED at TASK-0192' notes; the buffer-fix rationale is inline in the schedule + the link.rs test docstring.

Tests added: lowers_14_hearing_aid_embedded_multimcu (sched_lower), links_14_hearing_aid_embedded_multimcu (link, algo=prog.embedded.algo.nuc), acfg_example_14_embedded_multimcu (acfg).

forward-carried from TASK-0192 (bears on deep-codegen M11 follow-ups under TASK-0049):
1. The lower/link/acfg FRONT-END fully admits the M11 multi-MCU schedule with NO new machinery — the remaining M11 work is purely BACK-END (embedded-pattern multi-worker guard lift at backends/embedded-pattern/src/lib.rs, cross-MCU UART transport codegen, Renode .resc, stateful kernels.rs, e2e-harness algo-selection). The compiler-side de-risk is GREEN.
2. acfg shape for the deep codegen: 7 ops / 1 repeat / depth 1; all per-frame IO (incl. the rf_transmit/fe_emit effects) lives inside the single frame Repeat — the backend will see a 3-worker partition with cross-worker Push/Wait on mic_in/bt_in/spk_out/bt_out, each ring buffer=3, plus an Event::Sync per pipeline epoch and a check loop on 'frame'.
3. buffer==pipeline-depth is now pinned for this schedule; if a future cycle changes pipeline depth, buffer must move with it (the PipelineExceedsBuffer gate will catch a regression).

GATE (full): build OK; clippy OK; test 1085/0/3; test-release 1084/0/3; e2e 301/246/0/55/0 (unchanged; 7 embedded cells stay [[skip]]).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
M11 lowering-admission de-risk COMPLETE. embedded_multimcu.sched.nuc (the M11 multi-MCU schedule) is now in all three positive matrices — sched_lower (lowers_14_hearing_aid_embedded_multimcu), link (links_14_hearing_aid_embedded_multimcu, against the per-frame prog.embedded.algo.nuc from TASK-0054.01), and acfg (acfg_example_14_embedded_multimcu). All scope-exclusion comments that referenced this task are removed (AC#2). The de-risk surfaced ONE latent example bug — pipeline=3 vs buffer=2 (PipelineExceedsBuffer), inconsistent since the initial commit, before the link invariant existed — fixed by raising buffer 2->3 to match the documented pipeline depth and the 09/11/13 convention (NOT by weakening the assertion). NO new lowering machinery was needed: the front-end fully admits the complex M11 schedule shape. No deferred gap subtask filed (AC#1's 'included' branch, not the 'documented exclusion' branch). Remaining M11 work is purely BACK-END (embedded-pattern multi-worker guard lift, cross-MCU UART codegen, Renode substrate, stateful kernels.rs, e2e-harness algo selection) — tracked under TASK-0049 / TASK-0054.01 parts 2-4. Gate: build/clippy OK; test 1085/0/3; test-release 1084/0/3; e2e 301/246/0/55/0 unchanged.
<!-- SECTION:FINAL_SUMMARY:END -->
