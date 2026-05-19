---
id: TASK-0200
title: Multi-error reporting in schedule lowering (sched analog of TASK-0092)
status: To Do
assignee: []
created_date: '2026-05-19 20:25'
updated_date: '2026-05-19 22:46'
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
<!-- SECTION:NOTES:END -->
