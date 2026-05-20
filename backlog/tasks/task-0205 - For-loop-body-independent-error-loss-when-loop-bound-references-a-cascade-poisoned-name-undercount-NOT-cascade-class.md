---
id: TASK-0205
title: >-
  For-loop body independent-error loss when loop bound references a
  cascade-poisoned name (undercount, NOT cascade-class)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 23:00'
updated_date: '2026-05-20 18:45'
labels:
  - compiler
  - diagnostics
  - follow-up
  - M0
  - undercount
dependencies:
  - TASK-0092
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced during TASK-0092 cycle-3 review (qa-test-runner finding #1). In compiler/src/algo/lower.rs lower_stmt for the For statement (around lines 822-866), the loop bounds 'lo' and 'hi' use ?-propagation. When either bound references a cascade-poisoned name, the for-statement returns Err BEFORE the body is visited, so any GENUINELY INDEPENDENT error inside the body (e.g. a never-declared kernel call, a separate div-by-zero, etc.) is never surfaced. Reproducer: 'const BAD=1/0; const X=BAD+1; data y:f32[X]; kernel dump:(f32[X])->() effectful; for i:0..X { truly_never_declared_kernel(y); }' emits 1 error (BAD root), the independent never-declared identifier is lost. This is NOT a cascade-class regression — TASK-0092's documented K×L contract is narrowly about cascade-decl + K*L cascade statements, not 'all independents inside a cascade-scoped body'. It IS a related undercount class that the current contract does not claim to fix, and that the cycle-3 docstring rewrite does not call out. Per qa-test-runner's blunt verdict: 'if a backlog reviewer or user encounters it, they may classify it as yet another cascade-class miss'. Worth fixing or explicitly disclaiming. Filed from TASK-0092 cycle-3 review (2026-05-20).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decide whether to fix (descend into for-body to collect independent errors even when bound evaluation fails) or to explicitly disclaim in lower.rs / TASK-0092 docstring contract. If fix: descend with a fresh accumulator branch when bound-eval fails; carry independent errors out; do NOT emit cascade errors from references to the dead iter-var. If disclaim: extend the lower_algo counting-contract docstring at lower.rs:109-122 to explicitly state 'a for-body with a cascade-poisoned bound is not visited; independent errors inside it are not reported'
- [x] #2 Either way: add a SIZE-PARAMETRISED regression fixture that pins the chosen behaviour for K∈{1,3} independent errors inside K∈{1,3} cascade-scoped for-bodies — if fix, K independents → K errors + the bound root; if disclaim, the bound root only and the test pins K independents are lost (assertion-strength PRESERVED — no len==1 blanket assertion masking)
- [x] #3 just test / just ci / clippy clean; no behaviour change for valid input (e2e 30/26/0/4/0)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Decision: FIX (TASK-0205 AC#1)

### PRD §6.2.3 verbatim citation (lines 318-323)

> **Name resolution.** Iteration variables and data variables share one
> namespace. Iteration variables shadow at their loop and go out of
> scope at the loop's end. A name `y` inside a `for y : ...` body
> always refers to the iteration variable; outside, it refers to
> whatever `data y : ...` declared (or is undefined). No `@`-style
> prefix; the compiler disambiguates by scope.

The "always refers to the iteration variable" rule is UNCONDITIONAL — it does not depend on bound-evaluation success. The FIX honors this by push_loop-ing the iter-var regardless of whether bound-eval failed. Body uses of the iter-var resolve cleanly via the natural scoping rule — no errors to suppress.

### Rationale for FIX vs DISCLAIM

FIX chosen because:

1. TASK-0092 cycle-3 invariant: "independent errors must STILL be reported". The for-body case is the only documented exception — DISCLAIM would leave that exception in place; FIX removes it.

2. Structural fit: the cascade-suppression infrastructure (failed_decls) already handles the natural case (poisoned bounds get cascade-suppressed); the FIX simply ensures body-descent is unconditional. No new cascade machinery needed.

3. Counting-contract extension is clean: 1 + K*M (1 root + K*M body independents) extends the existing K + K*L rule on the same "every independent reported, every cascade suppressed" axis. The lower_algo docstring now lists this rule alongside the K*L, K+K*M rules.

### Implementation site: nucleus/compiler/src/algo/lower.rs

- `lower_stmt_into` (new, ~30 lines): acc-aware dispatcher; routes Stmt::For to `lower_for_into`, other variants to existing `lower_stmt` and routes Err through acc.record_stmt_error.
- `lower_for_into` (new, ~55 lines): always descends into body with iter-var in scope; emits IrStmt::For only if both bounds AND every body statement succeeded; on any failure returns None (independent errors live in acc).
- `lower_stmt` Stmt::For arm: replaced with `unreachable!` guard (For now only reached via `lower_stmt_into`).
- `lower_algo` Item::Stmt: switched from `lower_stmt(..)?` to `lower_stmt_into(.., &mut acc)`.
- `lower_algo` counting-contract docstring: extended with the 1 + K*M rule and pointers to the 3 pinning fixtures.

### Pinning fixtures (algo_lower.rs, +3 tests, +~250 lines)

1. `for_body_independents_survive_cascade_poisoned_bound_for_any_k_m` — size-parametric K ∈ {1,2,3} × M ∈ {0,1,2,3} sweep (12 cells). M=0 is the negative control (clean body → root only). Asserts EXACT 1 + K*M error count, source-order discrimination on each independent, and anti-leak of any cascade-suppressible variant naming BAD/X.

2. `iter_var_use_in_body_of_cascade_scoped_loop_is_clean` — iter-var poisoning interaction: body uses iter-var as index `y[i]`; emits ONLY the root error, no spurious IterVarOutOfScope/UnknownIdent on i. Quotes PRD §6.2.3 verbatim for the unconditional scoping rule.

3. `nested_for_inner_cascade_bound_still_surfaces_inner_body_independents` — nested for where the INNER for has a cascade-poisoned bound; pins that the FIX works at any nesting depth (recursive lower_stmt_into routes every For through lower_for_into).

### Behavior change in valid-rhs dataflow inside doomed body

A `c <-- f(c)` inside a for-body whose bound is poisoned NOW gets visited and updates `scope.assigned`. A subsequent top-level `c <-- f(c)` now fires `DoubleAssignment`. Pre-FIX it did not (body was never visited).

This is MORE correct: PRD §6.2.1 single-assignment is per-symbol over the program's lifetime, not contingent on bound-reachability. The user has two source-text assignments to the same name; that is a violation regardless of whether the for-loop's bound evaluates. No existing test depended on the old (less-strict) behavior.

### Gate (7 of 7 green)

1. just test: 466 (= 463 baseline + 3 new fixtures).
2. cargo clippy --workspace --all-targets -- -D warnings: clean.
3. just e2e: 30/26/0/4/0 (exact target).
4. just determinism-check x2: byte-identical.
5. just determinism-check-negative: 26 perturbed, bites.
6. just xbackend-check-negative: 13 corrupted, 1 detected by differential, bites.
7. just ci: exit 0.

### Forward-carry

No interaction expected with TASK-0199 (parser-layer cascade fix is at a different layer; the FIX in lower.rs is independent and lives in the acc-aware body-descent path).

ORCHESTRATOR review-gate cycle (post-9e1c985):

qa-test-runner: GO. All 7 gate numbers reproduce exactly. K=4 × M=5 unprobed-cell probe yields 21 errors = 1 + 4*5 (the 1+K*M rule scales beyond the parametric set). Anti-leak guard fires correctly (no UnknownIdent("BAD"/"X"), no ConstRefersToNonConst leak). Doomed-body double-assignment behavior change reproduces exactly as disclosed and is correct under PRD §6.2.1 + §6.2.5(2). Stale release binary noted as gotcha (not introduced by this commit).

mped-architect: GO with two minor findings, neither blocking:

(1) BRIEF-PREMISE CORRECTION: the orchestrator brief said "iter-var enters scope as POISONED (added to failed_decls)" — the implementer chose the OPPOSITE design (unconditional iter-var resolution via scope.iter_var_in_scope per PRD §6.2.3, NOT failed_decls). The implementer's design is BETTER — more honest about the PRD scoping rule and avoids cross-cutting interactions between failed_decls and scope semantics. The brief was wrong in its prescriptive language; the implementer correctly chose the better design. Forward-carry for memory: orchestrator briefs should describe the PROBLEM, not over-specify the MECHANISM; the implementer is closer to the code and may find a better design.

(2) PRD-ANCHOR PRECISION on the doomed-body double-assignment behavior change: the commit message cites PRD §6.2.1 (ambiguous about cross-scope), but the LOAD-BEARING anchor for "single-assignment over symbol life" is PRD §6.2.5(2) at PRD.md:350-352: "Single-assignment is keyed by data symbol name". The in-code reference at lower.rs:772-775 is correct; only the commit-message citation is imprecise. Non-blocking.

Both findings are recorded for forward-carry to the memory file. The IMPLEMENTATION is correct; the FIX direction is sound; all 3 ACs honestly met; gate green. TASK-0205 close-out confirmed Done.

CASCADE-CLASS METHODOLOGY-TRANSFER SCORECARD (now 4-for-4):
- TASK-0092 cycle-3: AlgoIR lowering — 5x-recurrence closure. Clean.
- TASK-0087 cycle-4: sched-parser n+2 measurement. Clean.
- TASK-0200 cycles 1+2+review: sched-lowering Path-2 closure. Required cycle-2-review.
- TASK-0204+TASK-0207+TASK-0206+TASK-0205: incremental hardening + closures. TASK-0207 required cycle-7 PRD-citation correction; TASK-0206 required cycle-7 PRD-misattribution sweep; TASK-0205 chose the better design despite the brief's prescriptive over-specification.

The cycle-7 lesson (rationale citations need grep+read+cite) has now triggered preventatively in TASK-0205 (implementer correctly chose the design AND verified the PRD quote VERBATIM with line numbers, per the brief's explicit citation-discipline instruction). The discipline IS becoming preventative rather than reactive.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
**Direction: FIX (AC#1).** PRD §6.2.3 lines 318-323 verbatim: "A name `y` inside a `for y : ...` body always refers to the iteration variable" — unconditional, no dependency on bound-eval success. Cycle-3 invariant "independent errors must STILL be reported" violated by the pre-existing ?-on-bounds path; the FIX honors both.

**Implementation (nucleus/compiler/src/algo/lower.rs):**
- New `lower_stmt_into` dispatcher (acc-aware): routes Stmt::For through `lower_for_into`; other variants delegate to `lower_stmt` and route Err through `acc.record_stmt_error`.
- New `lower_for_into`: always descends into the body with iter-var in scope; emits IrStmt::For only if both bounds AND every body statement succeeded; on any failure returns None (independent errors live in acc).
- `lower_stmt` Stmt::For arm: replaced with `unreachable!` guard.
- `lower_algo` Item::Stmt: switched to `lower_stmt_into(.., &mut acc)`.
- `lower_algo` counting-contract docstring extended with the 1 + K*M rule.

**Pinning (algo_lower.rs, +3 tests):**
1. `for_body_independents_survive_cascade_poisoned_bound_for_any_k_m` — size-parametric K ∈ {1,2,3} × M ∈ {0,1,2,3} sweep (12 cells, M=0 negative control); exact 1 + K*M count, source-order discrimination, anti-leak.
2. `iter_var_use_in_body_of_cascade_scoped_loop_is_clean` — iter-var-as-index emits no spurious diagnostic; honors PRD §6.2.3 unconditionally.
3. `nested_for_inner_cascade_bound_still_surfaces_inner_body_independents` — recursive dispatch works at any nesting depth.

**Counting-contract addendum:** 1 cascade-poisoned root + K cascade-scoped for-loops × M independent body errors each → EXACTLY 1 + K*M errors. Pre-FIX was always 1 (the root). Companion to the K+K*L (TASK-0092) and K+K*M (TASK-0206) rules.

**Gate (7/7 green):** test 466 (= 463 baseline + 3 new); clippy clean; e2e 30/26/0/4/0; determinism byte-identical; det-neg bites (26 perturbed); xb-neg bites (13 corrupted, 1 detected); ci exit 0.

**Behavior change in valid-rhs dataflow inside a doomed body:** a `c <-- f(c)` inside a for-body whose bound is poisoned now records the assignment; a subsequent top-level `c <-- f(c)` now fires DoubleAssignment. MORE correct: PRD §6.2.1 single-assignment is per-symbol over the program's lifetime, not contingent on bound-reachability. No existing test depended on the old behavior.

**Commit:** 9e1c985.

**Follow-ups: none.** No deeper defect surfaced during the for-body descent. No interaction with TASK-0199 (parser-layer cascade fix is at a different layer).
<!-- SECTION:FINAL_SUMMARY:END -->
