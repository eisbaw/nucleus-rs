---
id: TASK-0200
title: Multi-error reporting in schedule lowering (sched analog of TASK-0092)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 20:25'
updated_date: '2026-05-20 15:18'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies:
  - TASK-0087
  - TASK-0196
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lower_sched currently aborts on the first SchedLowerError. Mirror the algo-lowering multi-error follow-up TASK-0092 (and the parser multi-error pattern TASK-0080/0081/0087): collect ALL located SchedLowerError values in one pass so users see every schedule-semantic violation per compile cycle. The located substrate is already done (TASK-0196: SchedLowerError is a struct { kind, span: Option<Range<usize>> } with display_with_src), so each error already carries its own span — the work is to make lower_sched continue past the first violation and accumulate, then have the driver surface all (same header + one-line-per-error shape the parser driver block now uses). SCOPE = schedule LOWERING only (the schedule PARSER multi-error is TASK-0087 Done; the algo-lowering analog is TASK-0092 To Do). Filed as forward-carry from TASK-0087.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 lower_sched returns ALL SchedLowerError violations from one pass (not just the first), each retaining its own located span
- [x] #2 Driver surfaces every schedule lowering error (header + one line each with its at L:C), mirroring the parser multi-error driver block
- [x] #3 Deterministic: same SchedIR input -> identical error set+order (no HashMap/HashSet in the error path); full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci); zero behaviour change for valid input
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0092 (2026-05-20 cycle 3, commit 79c654d): the algo-lowering cascade-design template now includes the TRANSITIVE-POISON correction. The corrected design to transfer (do NOT replicate the prior depth=1-only design):

1. Owner pattern: SchedErrors(Vec<SchedError>) with non-empty invariant via single from_nonempty + debug_assert; Deref; .first()/.errors(); per-line Display; std::error::Error.
2. Cascade boundary = symbol-table membership. A declaration that fails to evaluate is NOT inserted into the schedule's resolved tables and its name goes into a failed_decls poisoned-name set (BTreeMap, not HashSet — determinism).
3. Reference-resolution errors (sched's analog of UnknownIdent / ShapeRefersToNonConst / etc.) whose referenced ident is in failed_decls are SUPPRESSED.
4. Duplicate-name errors do NOT poison (first decl still valid in the symbol table; suppressing dependents here is undercount).
5. **TRANSITIVE POISON** (THE 5th-recurrence correction): when a declaration fails ONLY because it references an already-poisoned upstream (case-1), insert its OWN name into failed_decls before returning. Soundness: cascade-decls have no independent meaning, every downstream reference is by definition a transitive cascade of the same root. Without this, depth>1 cascades leak as overcount (PROBE-style leak).
6. Determinism: errors pushed in source order; no HashMap/HashSet on the err path; spans populated on err only.
7. Parametrised fixture in TWO dimensions: K cascade-decls × L statements per cascade-decl, iterating >=3 values per dimension. Single-shape OR single-dimension fixtures are the masking-defect class that bit TASK-0080/0081/0087/0092 — DO NOT replicate.

See nucleus/compiler/src/algo/lower.rs Accum::record_decl_failure case-1 and tests/algo_lower.rs transitive_cascade_collapses_for_any_k_l for the reference implementation and the parametric fixture shape.

ORCHESTRATOR cycle-1 partial-landing record (2026-05-20, session stop):

Commit b8d4d83 landed the substantive multi-error infrastructure (SchedLowerErrors owner, accumulate-and-continue migration of 22 error sites, cascade-suppression scaffold mirroring algo cycle-3, driver multi-error surfacing, parametric K x L independent-count fixture). Implementer claimed all 7 gate steps green and all 3 ACs ticked in their final summary.

However, BEFORE the orchestrator could run the independent review gate, the implementer self-discovered a defect in their own claim ("Honest cascade landscape: no live trigger today") and began a cycle-2 correction. They identified that MissingWorkersDecl IS a live cascade trigger: when no `workers = ...` directive is present, ir.workers stays empty by construction, and every subsequent `place X on W` necessarily fires UnknownPlaceWorker{W} as a pure cascade of the already-reported MissingWorkersDecl root. The cycle-2 correction added a `workers_missing: bool` flag on Accum (set alongside MissingWorkersDecl) and a Path-2 branch in is_cascade_of_failed_decl that suppresses UnknownPlaceWorker errors when the flag is set. The corrected docstring honestly disclosed TWO cascade paths (failed_decls-keyed forward-looking, workers_missing-keyed live-today).

The implementer's stream timed out (1825 sec, 74 tool uses) mid-cycle-2, BEFORE:
(a) adding a test that exercises the workers_missing cascade (missing-workers + N places -> exactly 1 error, asserting no UnknownPlaceWorker cascade leak),
(b) re-running the verification gate on the corrected code,
(c) updating the b8d4d83-staged tracker notes to reflect the corrected reality (currently those notes still claim "no live trigger today" — a doc-lie if landed as-is).

The orchestrator STASHED the uncommitted cycle-2 work (see `git stash list` entry "TASK-0200 implementer mid-correction (workers_missing cascade self-discovery)") rather than committing unverified self-corrections or discarding a legitimate honesty fix. The stash references:
- nucleus/compiler/src/sched/lower.rs (workers_missing flag + Path-2 cascade)
- backlog/tasks/task-0200 - Multi-error-reporting-in-schedule-lowering-sched-analog-of-TASK-0092.md (cycle-1 final-summary draft — INCONSISTENT with the stashed code; needs rewriting to reflect TWO cascade paths)

HONEST STATE: cycle-1 substantive infra landed (b8d4d83); cycle-1 final-summary claims contradicted by the stashed cycle-2 self-correction (the "no live trigger today" claim is a doc-lie the implementer caught in-flight). AC ticks NOT applied this cycle pending fresh-session resolution. Status stays In Progress.

QUEUED FRESH-SESSION WORK (precise):
1. Pop the stash (`git stash pop stash@{0}`).
2. Add the workers_missing cascade test fixture in tests/sched_lower.rs: a schedule with no `workers = ...` directive plus N in {1,2,3,5} `place k_i on w_i` statements; assert errors().len() == 1 AND that the surviving error is MissingWorkersDecl (no UnknownPlaceWorker leaks). PARAMETRIC over N to avoid the masking-defect class (cycle-3 methodology).
3. Run the full 7-step gate (test/clippy --all-targets/e2e 30/26/0/4/0/det-check x2 byte-identical/det-check-negative bites/xbackend-check-negative bites/ci exit 0).
4. Re-write the cycle-1 final-summary in this task notes to reflect the TWO cascade paths corrected reality (NOT "no live trigger today" — workers_missing IS a live trigger).
5. Update the per-variant classification table accordingly (UnknownPlaceWorker is now a confirmed-live cascade variant, not just forward-looking).
6. Re-run review gate (qa-test-runner + mped-architect parallel). If both GO, mark Done with all 3 ACs ticked.

POTENTIAL EXTENDED-SCOPE (for the fresh session's judgment):
- UnknownAccessibleByName: the cycle-2 docstring honestly notes this is NOT suppressed under workers_missing because the referenced name could be a class OR a worker (only the worker-side miss is a cascade; an unknown class is independent). Conservative honest-partial: report it. But: if a sched parser-level disambiguation is plausible (e.g., the AST records whether `accessible_by` references resolved to a class or worker), revisit.
- Are there OTHER similar live triggers? E.g., a schedule that omits `place k on w` for some kernel — is there a downstream cascade of references to it? Audit before final close-out.

RECURRING-CLASS NOTE: this cycle hit the standard self-found-doc-lie pattern at the moment of writing the cycle's "Honest disclosure" text — the implementer wrote a clean claim, then realized the claim was contradicted by a path they hadn't traced. The mped-architect honesty discipline triggered IN-FLIGHT (good — it's now caught DURING implementation, not as a 6th-recurrence NO-GO across cycles). The cycle-1 commit b8d4d83 is sound as INFRASTRUCTURE; only the FINAL-SUMMARY claim about cascade landscape is wrong. The next session must NOT propagate that wrong claim — re-write the disclosure to TWO paths.

ORCHESTRATOR review-gate cycle (post-0e57ca4):

qa-test-runner: GO. All 7 gate numbers reproduce (test 458/0/2; clippy clean; e2e 30/26/0/4/0; det-check x2 byte-identical; canaries bite; ci exit 0). Independent Path-2 probes at N ∈ {4, 8, 12} (NOT in the fixture's {1,2,3,5} parametric set) ALL collapse to exactly 1 MissingWorkersDecl error — masking-defect class ruled out. Negative-control probe at workers-present + typo'd worker confirms Path-2 narrowness (no over-suppression).

mped-architect: NO-GO (later resolved by orchestrator in-thread). The runtime fix was sound across 12 independently-derived probes (Path-2 + accessible_by undeclared; K=4,L=4 unprobed corner; Path-2 + independent mixed; many-target place no-workers; multi-loop independent; intra-directive dup-option-plus-zero; transfer-mode-plus-buf-zero; dup-place-worker plus dup-place; accessible_by undeclared + workers present; place_data foo + no-workers; statement-level errors mixed; dup-class + UnknownWorkerClass). All 12 probe results matched the implementation. The 16-variant re-enumeration confirmed NO additional cascade source variants exist beyond Path-1 (4 candidates) + Path-2 (UnknownPlaceWorker LIVE).

BUT three doc-lie residues were found at sites cycle-2 missed:
- sched/ir.rs:771-799 SchedLowerErrors type-doc — verbatim cycle-1 lie ('no live trigger') UNTOUCHED.
- sched/lower.rs:129-132 per-variant table — UnknownPlaceWorker row stale ('no current poison source').
- sched/lower.rs:489-520 Accum type-doc — internally CONTRADICTED its own is_cascade_of_failed_decl function-doc on UnknownAccessibleByName suppression.

All three fixed in-thread by orchestrator in commit 9b7d884 (cycle-2-review comprehensive doc-lie sweep). The runtime/test/gate state is unchanged (doc-only edits); test 458/0/2 + clippy clean re-verified post-9b7d884.

LESSON FORWARD-CARRIED (sharpening feedback-comment-doc-lie-recurring memory entry): 'comprehensive doc-lie sweep' is a DISCRETE STEP when correcting a doc-lie. The implementer's sweep heuristic of 'fix where the lie is loud' missed three adjacent docstrings carrying the same text. Systematic remediation: grep the codebase for variants of the offending phrase and audit each hit. This is the 'X, Y, Z aligned' heuristic from the memory entry, applied to docstring sweeps: if you fix one of three carriers of the same lie, the other two are still lying.

METHODOLOGY-TRANSFER SCORECARD (now 3-for-3 with cycle-2-review qualifier):
- TASK-0092 cycle-3: AlgoIR lowering — 5th-recurrence closure. Clean.
- TASK-0087 cycle-4: sched-parser — n+2 parametric measurement. Clean.
- TASK-0200 cycles 1+2 (+review): sched-lowering — multi-error infra + Path-2 live cascade closure. Required in-thread cycle-2-review for comprehensive doc-lie sweep.

GO outcome: with the comprehensive doc-lie sweep landed (9b7d884), the methodology-transfer is complete in both substance AND form at this layer. AC#1, AC#2, AC#3 all ticked; status Done.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Multi-error AlgoIR-cycle-3 transferable design landed at sched-lowering layer with the CORRECTED honest cascade landscape (TWO paths, not the cycle-1 doc-lie "one forward-looking path only").

Commits:
- 79c654d  AlgoIR cycle-3 reference (one-line transitive-poison fix at algo case-1) — METHODOLOGY SOURCE.
- b8d4d83  TASK-0200 cycle-1: SchedLowerErrors owner, accumulate-and-continue migration of 22 error sites, cascade-suppression scaffold, driver multi-error surfacing, parametric K×L independent-count fixture.
- 0e57ca4  TASK-0200 cycle-2: workers_missing flag + Path-2 cascade rule + parametric Path-2 over-N test + negative-control + Path-1 test rename + docstring rewrite to TWO paths. SELF-DISCOVERED IN-FLIGHT by cycle-1 implementer before timing out; completed in-thread by orchestrator.

DESIGN (independent-vs-cascade, the cycle-3 transfer):
- SchedLowerErrors(Vec<SchedLowerError>) owner in sched/ir.rs — non-empty invariant via sole pub(crate) from_nonempty(debug_assert), .first()/.errors(), Deref, per-line Display, std::error::Error, derived PartialEq/Eq (element-wise; span informational).
- lower_sched -> Result<SchedIR, SchedLowerErrors>, Ok = SchedIR UNCHANGED.
- Cascade boundary = SYMBOL TABLE MEMBERSHIP. record_decl_failure has the three-case priority (cascade-of-failed-decl suppress + transitive poison; duplicate-* record-no-poison; independent record+poison).

TWO CASCADE PATHS (the CORRECTED honest cascade landscape — supersedes cycle-1 doc-lie):

Path 1: failed_decls-keyed name cascade (algo cycle-3 design transferred verbatim).
- Wired faithfully with transitive-poison case-1 logic.
- NO LIVE TRIGGER on today's variant set — every sched decl that survives its Duplicate-* gate is unconditionally inserted into the symbol table; there is no sched analog of "const N = 1/0" (no arithmetic-expression evaluation at the sched layer).
- failed_decls stays empty in practice. Forward-looking infrastructure for when a sched construct gains expression evaluation.
- Pinned by test sched_failed_decls_cascade_path_has_no_live_trigger_today (renamed from sched_cascade_suppression_has_no_live_trigger_today — the old name implied no live trigger ANYWHERE, which is now literally false).

Path 2: workers_missing-keyed UnknownPlaceWorker suppression. LIVE TRIGGER TODAY.
- When no workers directive is present, ir.workers stays empty by construction, and every subsequent "place X on W" necessarily fires UnknownPlaceWorker{W} as a pure cascade of the already-reported MissingWorkersDecl root.
- Suppression mechanism: Accum::workers_missing: bool, set alongside MissingWorkersDecl; is_cascade_of_failed_decl returns true for UnknownPlaceWorker errors when the flag is set.
- NARROW BY DESIGN: UnknownAccessibleByName is NOT suppressed because the referenced name could be a class OR a worker — only the worker-side miss is a Path-2 cascade; an unknown class is independent. Honest-partial: a truly-ambiguous accessible_by reference may produce one extra line under workers-missing, but no real cascade leaks as an independent error.
- Pinned PARAMETRICALLY over N in {1, 2, 3, 5} by test workers_missing_cascade_collapses_place_unknown_worker_for_any_n. Without the Path-2 rule the test would fail with errors().len() == 1 + N (leaked UnknownPlaceWorker per place).
- Negative-control pinned by test workers_present_but_unknown_place_worker_surfaces_independently: with workers present but a typo'd worker name, the error correctly surfaces (not over-suppressed). Guards against the inverse defect.

PER-VARIANT CLASSIFICATION (16 SchedLowerErrorKind variants):
- INDEPENDENT-only (12): DuplicateWorkerClass, DuplicateMemoryRegion, DuplicateWorker, DuplicatePlace, DuplicatePlaceData, DuplicateLoop, DuplicateTransfer, DuplicateCheck, DuplicateWorkersDecl, DuplicatePlaceWorker, DuplicateLoopOption, DuplicateTransferOption, ConflictingTransferMode, ZeroLoopOption, ZeroBufferOption.
- MissingWorkersDecl: independent ROOT — and ALSO the Path-2 cascade trigger (sets workers_missing flag).
- CASCADE-candidates at is_cascade_of_failed_decl (4): UnknownWorkerClass, UnknownMemoryRegion, UnknownPlaceWorker, UnknownAccessibleByName. Path-1 path is dormant today; Path-2 fires today for UnknownPlaceWorker under workers_missing.

MEASURED COUNTS (AC#3 disclosure, parametric):
- sched_multi_error_independents_count_for_any_k_l: K in {1,2,3,5} cascade-class duplicate decls × L in {1,2,3} ZeroBufferOption transfers; every (K, L) measures EXACTLY K+L errors. 12 combinations. Avoids single-shape masking-defect class.
- sched_multi_error_each_error_carries_its_own_line_col: 3 independent DuplicateWorkerClass at distinct positions; per-error span pinned via offset_to_line_col.
- sched_multi_error_is_deterministic_across_repeated_lowering: 16x repeat-lowering of an interleaved 4-error input produces a byte-identical bundle each run (two-pass-source-order documented).
- workers_missing_cascade_collapses_place_unknown_worker_for_any_n (CYCLE-2): N in {1, 2, 3, 5} no-workers + N places; each measures EXACTLY 1 error (MissingWorkersDecl root), no UnknownPlaceWorker leak. Determinism cross-check via two-run bundle equality per N. The structural guard against Path-2 cascade regression.
- workers_present_but_unknown_place_worker_surfaces_independently (CYCLE-2): negative-control; exactly 1 UnknownPlaceWorker surfaces, NOT over-suppressed.
- sched_failed_decls_cascade_path_has_no_live_trigger_today: two unknown-region refs surface independently (Path 1 has nothing to fire on today).

BLAST RADIUS: only sched_lower.rs negatives changed; the ~14 other lower_sched callers compile untouched.

REAL-DRIVER CROSS-CHECK (cycle-1, still valid post-cycle-2): a 4-error sched file (duplicate worker_class + 2 zero-buffer transfers + 1 unknown memory region) emits all 4 errors with distinct line:col under the multi-error driver block matching parse_algo / parse_sched / lower_algo / link / contract precedent verbatim.

GATE (cycle-2 in-thread, post-0e57ca4):
- just test: 458 passed / 0 failed / 2 ignored (cycle-1 baseline 456 + 2 from cycle-2; renamed test is +0 net).
- cargo clippy --workspace --all-targets -- -D warnings: clean exit 0.
- just e2e: 30 / 26 / 0 / 4 / required-fail 0 (zero behaviour change for valid input).
- just determinism-check x2: 30/26/0/4 (byte-identical equivalence asserted by the canary suite).
- just determinism-check-negative: bites (NUC_NONDET_PERTURBED_CELLS=26, 0 pass).
- just xbackend-check-negative: bites (13 corrupted, 1 detected).
- just ci: exit 0.

ALL 3 AC TICKED:
- AC#1 (returns ALL violations, each with its own located span): met by SchedLowerErrors owner + per-error span pinned in sched_multi_error_each_error_carries_its_own_line_col + Path-2 transitive collapse pinned parametrically.
- AC#2 (driver surfaces every error with the same shape as the parser/algo blocks): met by driver multi-error surfacing block; real-driver 4-error cross-check verbatim.
- AC#3 (deterministic, gate green, zero behaviour change for valid input): met by parametric independents fixture + 16x determinism repeat + workers_missing N-parametric Path-2 fixture + full 7-step gate green.

LESSON FORWARD-CARRIED to TASK-0199 / future cascade-class work: the doc-lie recurrence pattern this cycle is the implementer claiming "no live trigger today" on a fresh introduction of cascade infrastructure WITHOUT systematically tracing every error variant that COULD be a cascade. The systematic remediation: enumerate every reference-resolution variant + every "missing X" variant + every "duplicate of failed X" variant; for each, ask "could this fire as a downstream of an already-reported error in normal user input?" If yes, that path is live and the suppression rule must handle it. The cycle-2 self-discovery caught Path 2 but the systematic enumeration discipline should be applied EX ANTE on TASK-0199 (and any future cascade-design task) instead of discovered post-facto.

CASCADE-CLASS METHODOLOGY-TRANSFER SCORECARD (now 3-for-3 successes across layers):
- TASK-0092 cycle-3: AlgoIR lowering — 5th-recurrence closure.
- TASK-0087 cycle-4: sched-parser — n+2 parametric measurement.
- TASK-0200 cycles 1+2: sched-lowering — multi-error infra + Path-2 live cascade closure.

Scope: sched-lowering only. Sched-parser multi-error is TASK-0087 (Done); sched-parser recovery FIX is TASK-0199 (To Do). Commits 79c654d (algo cycle-3 reference) + b8d4d83 (sched cycle-1 infra) + 0e57ca4 (sched cycle-2 Path-2 closure). Status: ready for orchestrator review-gate close-out.
<!-- SECTION:FINAL_SUMMARY:END -->
