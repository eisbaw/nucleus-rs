---
id: TASK-0200
title: Multi-error reporting in schedule lowering (sched analog of TASK-0092)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 20:25'
updated_date: '2026-05-20 07:36'
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
- [ ] #1 lower_sched returns ALL SchedLowerError violations from one pass (not just the first), each retaining its own located span
- [ ] #2 Driver surfaces every schedule lowering error (header + one line each with its at L:C), mirroring the parser multi-error driver block
- [ ] #3 Deterministic: same SchedIR input -> identical error set+order (no HashMap/HashSet in the error path); full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci); zero behaviour change for valid input
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
<!-- SECTION:NOTES:END -->
