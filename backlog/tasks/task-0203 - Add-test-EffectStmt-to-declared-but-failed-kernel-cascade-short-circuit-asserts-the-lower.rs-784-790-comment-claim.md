---
id: TASK-0203
title: >-
  Add test: EffectStmt to declared-but-failed-kernel cascade short-circuit
  (asserts the lower.rs:784-790 comment claim)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 22:33'
updated_date: '2026-05-20 05:30'
labels:
  - compiler
  - diagnostics
  - tests
  - follow-up
  - M0
  - doc-lie-audit
dependencies:
  - TASK-0092
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lower.rs:784-790 (Stmt::Effect arm purity check) asserts in a comment that if the kernel was declared but its body failed to lower, the existing is_cascade_of_failed_decl UnknownIdent suppression collapses the error to the root declaration failure. The existing test effect_stmt_to_unknown_kernel_stays_unknown_ident only covers the never-declared path. The declared-but-failed-body path is asserted in the comment but NOT measured — the exact comment-doc-lie class this project keeps recurring on. Filed from TASK-0089 architecture review (Finding #4, 2026-05-20). NOTE: this test will FAIL until TASK-0092's case-1 transitive-poison one-line fix lands (because the cascade-decl path currently does not poison the kernel name in failed_decls); so the test is the right discriminator to land ALONGSIDE the transitive fix.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 New test in compiler/tests/algo_lower.rs: a kernel signature contains e.g. i32[BAD_CONST] so the kernel lowering fails (kernel poisoned in failed_decls); a downstream bare-call bad_kernel(); produces EXACTLY one error (the upstream BAD_CONST root failure, NOT also the UnknownIdent cascade); located line:col pinned
- [x] #2 Asserts both: (a) no UnknownIdent cascade for the bare-call; (b) the root failure has the correct kind and span; (c) no EffectCalleeNotEffectful spuriously emitted (purity check naturally short-circuits when kernel is not in ir.kernels)
- [x] #3 just test passes; just ci exit 0; no behaviour change for valid input; clippy --workspace --all-targets clean
- [x] #4 Lands AFTER (or as part of) the TASK-0092 case-1 transitive-poison one-line fix; depends on TASK-0092 for that reason — without the fix this test would FAIL by design
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## 2026-05-20 closure (TASK-0203 Done)

Implementer: phase3-backlog-ralph

### Outcome
Commit 7df54f3 — single commit landing the new test + minimal comment forward-reference + tracker In-Progress mark.

### New test
- File: nucleus/compiler/tests/algo_lower.rs (immediately after effect_stmt_to_unknown_kernel_stays_unknown_ident)
- Name: effect_stmt_to_declared_but_failed_kernel_collapses_to_root
- Source (exact):
    const BAD_CONST : usize = 1 / 0;
    kernel bad_kernel : (i32[BAD_CONST]) -> () pure;
    bad_kernel();
- Expected vs actual: EXACTLY 1 error survives, kind ConstDivByZero{in_const:"BAD_CONST"}, span pins to line 1 col 27 (the 1/0 token). All sibling cascade errors suppressed.

### Discrimination strength
Asserts ALL of:
  (a) NO UnknownIdent("bad_kernel") leaks — discriminates the TASK-0092 case-1 transitive-poison path (would fail if cycle-3 fix reverted)
  (b) NO EffectCalleeNotEffectful{callee:"bad_kernel"} leaks — kernel declared `pure` so a regression half-inserting into ir.kernels would surface
  (c) NO ShapeRefersToNonConst{unknown_ident:"BAD_CONST"} leaks — kernel-decl-cascade suppression check
  (d) exact line:col span of surviving root error pinned via offset_to_line_col

### Gate measurements
1. just test: 451 passed / 0 failed / 2 ignored (baseline 450 + my 1)
2. cargo clippy --workspace --all-targets -- -D warnings: clean
3. just e2e: 30/26/0/4/0
4. just determinism-check (x2): byte-identical 30/26/0/4 both runs
5. just determinism-check-negative: bit correctly (26 of 30 perturbed cells detected)
6. just xbackend-check-negative: bit correctly (13 corruptions applied, 1 detected — bites)
7. just ci: exit 0

### Per-AC
- AC#1: PASS — kernel with i32[BAD_CONST], bare-call bad_kernel(), exactly 1 error, line:col pinned
- AC#2: PASS — (a) UnknownIdent absence asserted; (b) ConstDivByZero{BAD_CONST} kind + span asserted; (c) EffectCalleeNotEffectful absence asserted
- AC#3: PASS — just test passes, just ci exit 0, clippy clean, valid-input behaviour unchanged (e2e 30/26/0/4/0 stable)
- AC#4: PASS — landed on master AFTER commit 79c654d (TASK-0092 cycle-3 transitive-poison fix); the test depends on that fix and would fail without it

### Comment honesty (audit closure)
lower.rs Stmt::Effect comment (line 783-) now forward-references both sibling tests by name:
  - effect_stmt_to_unknown_kernel_stays_unknown_ident (never-declared branch)
  - effect_stmt_to_declared_but_failed_kernel_collapses_to_root (declared-but-failed-body branch)
The doc-lie is now closed: both branches of the claim are measured by named tests.

### Honest limits / gotchas
- The test relies on the chumsky parser accepting `i32[BAD_CONST]` as a valid kernel param type and `1 / 0` as a const-expr. Confirmed by the lower-pass reaching ConstDivByZero (means parse succeeded). If parser surface changes break this, the test would surface a parse-failure panic at lower_str's `.expect("source must parse")` — that's a parser-test surface, not this test's concern.
- Did NOT broaden scope: no new error variants, no cascade-suppression rule changes, no other tests touched.
- The "no spurious EffectCalleeNotEffectful" assertion is currently somewhat redundant with the EXACTLY-1-error check, but pins the half-insert regression channel for defence in depth.
- Cycle-3 transitive-poison fix is now measured at the kernel-cascade path (this task), the data-cascade path (one_failed_const_with_n_dependents_yields_exactly_one_error, transitive_cascade_collapses_for_any_k_l), and broadly via the K×L cascade test. Coverage of case-1 transitive poisoning is now adequate.

### Follow-ups
None filed. Scope discipline held — single test + single comment tightening, no scope creep.
<!-- SECTION:NOTES:END -->
