---
id: TASK-0205
title: >-
  For-loop body independent-error loss when loop bound references a
  cascade-poisoned name (undercount, NOT cascade-class)
status: To Do
assignee: []
created_date: '2026-05-19 23:00'
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
- [ ] #1 Decide whether to fix (descend into for-body to collect independent errors even when bound evaluation fails) or to explicitly disclaim in lower.rs / TASK-0092 docstring contract. If fix: descend with a fresh accumulator branch when bound-eval fails; carry independent errors out; do NOT emit cascade errors from references to the dead iter-var. If disclaim: extend the lower_algo counting-contract docstring at lower.rs:109-122 to explicitly state 'a for-body with a cascade-poisoned bound is not visited; independent errors inside it are not reported'
- [ ] #2 Either way: add a SIZE-PARAMETRISED regression fixture that pins the chosen behaviour for K∈{1,3} independent errors inside K∈{1,3} cascade-scoped for-bodies — if fix, K independents → K errors + the bound root; if disclaim, the bound root only and the test pins K independents are lost (assertion-strength PRESERVED — no len==1 blanket assertion masking)
- [ ] #3 just test / just ci / clippy clean; no behaviour change for valid input (e2e 30/26/0/4/0)
<!-- AC:END -->
