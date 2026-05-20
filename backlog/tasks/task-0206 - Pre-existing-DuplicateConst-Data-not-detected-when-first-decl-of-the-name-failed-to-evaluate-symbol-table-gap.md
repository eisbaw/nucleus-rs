---
id: TASK-0206
title: >-
  Pre-existing: DuplicateConst/Data not detected when first decl of the name
  failed to evaluate (symbol-table gap)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 23:00'
updated_date: '2026-05-20 18:13'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Decision: FIX (strict-symmetry / cascade-aware duplicate detection)

PRD-grounded rationale:
- PRD §6.2.1: "Single-assignment within a scope. Mutation only via a fresh binding."
- PRD §6.2.5: single-assignment is keyed by data symbol name.
- PRD §6.2.3: identifiers share one namespace at the algorithm level.

The PRD is SILENT about declaration uniqueness explicitly, but the existing
language semantics interpret name uniqueness strictly. The current code's intent
(lower.rs:317-323, 344-352, 366-374) already enforces cross-namespace collision
even before evaluation succeeds. The natural extension of that intent is: a
re-declaration of a name whose first decl was cascade-poisoned IS still a
duplicate. The first-decl SOURCE TEXT used the name; that is what counts for
uniqueness, not whether evaluation succeeded.

Forward-fix-by-redecl is a code smell anyway — the user should edit the
existing decl, not add a second one with the same name.

## Implementation plan

1. lower_const / lower_data / lower_kernel duplicate-check: extend to consult
   an additional "declared names" set that includes failed declarations.
   Threading: Accum.failed_decls already tracks failed names; lift it (or a
   "declared_names" set) so the decl-lowering paths see it.
2. The cleanest design (no scope creep, minimum surgical change): add a
   declared_names: BTreeMap<String, ()> to Accum (parallels failed_decls),
   record EVERY decl-name attempt (successful → into ir.X already + into
   declared_names; failed → into failed_decls + into declared_names). The
   duplicate-check at lower_const/data/kernel consults declared_names UNIONED
   with ir.X tables (the union is equivalent to declared_names if maintained
   carefully; pick one).
3. Soundness: cascade discipline must not regress. A re-decl of a cascade-poisoned
   name fires DuplicateX (independent error). The cascade-poison of the original
   stays poisoned: the second decl ALSO doesn't insert into ir.X (it errors at
   duplicate-check time before evaluation), so downstream references to the
   name remain cascade-suppressed. Confirmed by AC#3 fixture.
4. Size-parametrised fixture: K cascade-decls × M duplicate-of-failed re-decls.
   Asserts exactly K + M errors (K root failures + M DuplicateX), no cascade leak.
5. Migrate any existing positive-case duplicate tests — assertion strength
   preserved (no blanket-len weakening).

## Counting contract update

Add to the lower_algo docstring (the case-1 transitive-poison block):
- TASK-0206: duplicate-detection IS cascade-aware. A re-declaration of a name
  whose first decl was cascade-poisoned OR independently failed STILL fires
  DuplicateX. Net: M independent bad decls + N duplicate-of-failed re-decls
  = M + N errors.

## Resolution (2026-05-20, commit 2a42291)

DIRECTION: FIX (cascade-aware duplicate detection).

DESIGN DECISION rationale:
- PRD §6.2.1: "Single-assignment within a scope. Mutation only via a fresh binding."
- PRD §6.2.5: single-assignment keyed by data symbol name.
- PRD §6.2.3: identifiers share one namespace at the algorithm level.
The PRD is silent on declaration uniqueness EXPLICITLY, but the existing
language semantics (which the current code already enforces for the
success-path cross-namespace collision) interpret name uniqueness
strictly. The natural extension is: a re-declaration of a name whose
first decl was cascade-poisoned OR independently failed IS still a
duplicate — the source-text re-use of the name is the violation,
independent of whether the first evaluated.

The "fixing-forward by adding a sibling decl" reading is weaker — the
user would edit the existing decl, not add a second with the same name.
Adding a sibling decl with the same name is itself a code smell that
should be flagged.

IMPLEMENTATION (nucleus/compiler/src/algo/lower.rs):
- New helper is_failed_decl(failed_decls, name).
- lower_const / lower_data / lower_kernel take &BTreeMap<String, ()>
  for failed_decls and consult it (union with ir.X) in the duplicate-
  check arms.
- The cascade-poison of the original stays in failed_decls; the
  duplicate-of-failed re-decl errors at the duplicate check (before
  evaluation), so ir.X stays clean and downstream references continue
  to cascade-suppress correctly.

COUNTING CONTRACT updated in lower_algo docstring:
- K poisoned roots × M duplicate-of-failed re-decls -> K + K*M errors.
- Pinned by size-parametric fixture duplicate_of_failed_decl_fires_for_any_k_m
  (decl-kind in {const, data, kernel} x K in {1,2,3,5} x M in {0,1,2,3}
  = 48 cells), plus three named headline fixtures.

AC STATUS:
- [x] #1 Investigated; decision documented in lower.rs docstring
       (`lower_algo` counting-contract block) + `record_decl_failure`
       case-2 docstring + this task note.
- [x] #2 Implemented (FIX direction); size-parametric regression
       fixture `duplicate_of_failed_decl_fires_for_any_k_m`.
- [x] #3 Headline fixtures pin exactly-2-errors for
       const N=1/0;const N=7 and data x:f32[BAD];data x:f32[4];
       cascade chains downstream of the re-declared name still
       suppress correctly (redecl_of_failed_does_not_unpoison_
       downstream_cascade — 2 errors, no leak).
- [x] #4 just test 463 passed; clippy clean; e2e 30/26/0/4/0;
       determinism byte-identical x2; just ci exit 0.

SIBLING-LAYER follow-up: TASK-0208 filed for sched-lowering
(DuplicateWorkerClass / DuplicateMemoryRegion / DuplicateWorker
not cascade-aware). Dormant today (no live trigger; sched-decl-eval
can't fail at sched layer), but structurally identical; pre-emptive
parity recommended.

HONEST LIMITS:
- This cycle did NOT touch the sched layer (scope creep avoidance per
  brief); the sched-layer concern is filed as TASK-0208 with full
  rationale and acceptance criteria.
- The parametric fixture pins const/data/kernel uniformly; it does
  NOT cross-cover "first decl is kind X, re-decl is kind Y" (e.g.,
  const N then data N). The existing const_and_data_share_namespace
  test covers the SUCCESS-path cross-namespace; the cascade-aware
  cross-namespace path is exercised at the language level (failed_decls
  is one set across all three namespaces, so the helper fires
  uniformly), but NOT pinned by a dedicated test. Risk: low; could be
  added if a regression surfaces.

CORRECTION (review-gate cycle, 2026-05-20): the prior "PRD-grounded rationale" sections above (in both the Implementation Notes header and the Final Summary) cite PRD §6.2.1 / §6.2.3 / §6.2.5 as grounding the cascade-aware-duplicate-detection FIX direction. mped-architect review independently re-read the PRD and found this is a MISATTRIBUTION. The cited sections are about STATEMENT-LEVEL single-assignment of DATA SYMBOLS via the dataflow `<--` operator, NOT about declaration uniqueness. Specifically:

- PRD §6.2.1 (PRD.md:234-239): "Single-assignment within a scope. Mutation only via a fresh binding." This is a bullet in "Storage and data", grouped with "No pointers", "Scalars are degenerate arrays", "Views are read-only slices". The neighboring "Mutation only via a fresh binding" makes it unambiguous — single-assignment refers to `<--` writes to array elements.
- PRD §6.2.5 (PRD.md:350-352): "Single-assignment is keyed by data symbol name, so a base-case + loop split (`out[0] <-- ...; for i : 1 .. N { ... }` on the same `out`) is a double-assignment." Verbatim about `<--` statements.
- PRD §6.2.3 (PRD.md:318-323): "Iteration variables and data variables share one namespace." Disambiguates iteration-var-vs-data-var, NOT const/data/kernel decl re-use.

The PRD does NOT explicitly state that `const N; const N;` (decl re-use) is an error. A grep of the PRD for "duplicate|redeclar|re-declar|unique" returns one hit (PRD.md:274, about effectful kernels never being duplicated — different topic).

TRUE RATIONALE (re-grounded, replaces the misattributed PRD citation): the FIX direction is grounded in:
(a) Existing codebase convention from TASK-0092 cycle-3: "first-decl-wins, Duplicate* fires on the second" is ALREADY the established behavior when the first decl SUCCEEDED (lower.rs `record_decl_failure` case-2). The cycle-6 TASK-0206 change extends this convention symmetrically to the failed-first case — the latent gap (failed-first dup-silent) was an asymmetry, not a deliberate design choice.
(b) Cross-namespace collision (data-vs-const, kernel-vs-data, etc.) is already enforced PRE-evaluation at lower.rs:317-323 / 344-352 / 366-374; the cycle-6 change extends that same intent to single-namespace failed-first re-decls.
(c) The "fixing-forward by adding a sibling decl" rejected reading: a user fixing a typo would EDIT the broken first decl, not ADD a second one with the same name. Spurious duplicate errors on edit-forward patterns are not a realistic concern.

The IMPLEMENTATION (commit 2a42291) and the chosen FIX DIRECTION are correct; only the rationale CITATIONS were misapplied. This is the cycle-6 "plausible-sounding rationale without verifying the actual source" pattern caught by independent grep+read+cite review discipline. Future cycles should ground rationale claims in actual code/spec reads with verbatim citations + line numbers, NOT plausible reconstruction.

Also: doc inconsistency in lower.rs:107 and lower.rs:363 where stale "M + N rule" references were used — actual rule defined at lower.rs:135-147 is "K + K*M". Fixed in-thread (the lower.rs:107 and 363 references now read "K + K*M rule").
<!-- SECTION:NOTES:END -->
