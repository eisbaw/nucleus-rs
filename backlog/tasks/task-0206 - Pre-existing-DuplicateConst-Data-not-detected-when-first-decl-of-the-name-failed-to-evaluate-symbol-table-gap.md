---
id: TASK-0206
title: >-
  Pre-existing: DuplicateConst/Data not detected when first decl of the name
  failed to evaluate (symbol-table gap)
status: To Do
assignee: []
created_date: '2026-05-19 23:00'
labels:
  - compiler
  - diagnostics
  - follow-up
  - M0
  - latent
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced during TASK-0092 cycle-3 mped-architect 21-probe sweep (PROBE 5 shape). Source: 'const N = 1/0; const N = 7;' — the SECOND decl is NOT caught as DuplicateConst because the first failure left ir.consts empty for N (the failed first decl is not inserted, so the second decl thinks it's the only one). Same defect class for 'data x : f32[BAD]; data x : f32[4];' — first fails (cascade), second 'duplicates' but no DuplicateData fires. This is PRE-EXISTING (NOT introduced by TASK-0092 cycle-3 — the new transitive-poison fix is unrelated; it correctly poisons failed_decls but duplicate-detection consults ir.consts/data/kernels, not failed_decls). Surfaces as a quiet symbol-table gap: a user fixing one error can silently introduce a duplicate. Filed from TASK-0092 cycle-3 review (2026-05-20).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Investigate: should DuplicateConst/Data fire even when the first decl failed to evaluate? PRO: symmetric semantics, catches the silent-typo class. CON: a user fixing one error and re-declaring would see a spurious duplicate where they expected to be fixing forward. Document the decision in the task notes + an updated docstring near record_decl_failure or the lower_const/data/kernel sites.
- [ ] #2 Implement chosen direction (probably: consult failed_decls in addition to ir.consts/data/kernels when emitting DuplicateConst/DuplicateData; ensure cascade-poisoned names also trigger duplicate detection on re-declaration) OR explicitly disclaim in the lower_algo counting-contract docstring at lower.rs:109-122. Either way, a SIZE-PARAMETRISED regression fixture pinning the chosen behaviour across K duplicate-of-failed re-decls
- [ ] #3 If fix: a 'const N = 1/0; const N = 7;' fixture produces EXACTLY 2 errors (the DivByZero + the DuplicateConst); a 'data x:f32[BAD]; data x:f32[4];' fixture produces EXACTLY 2; cascade chains downstream of the now-redeclared name still suppress correctly (no new cascade-class regression)
- [ ] #4 just test / just ci / clippy clean; no behaviour change for valid input (e2e 30/26/0/4/0)
<!-- AC:END -->
