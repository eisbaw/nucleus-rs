---
id: TASK-0202
title: >-
  Multi-error EffectStmt purity test: pin exact (line, col) per violation
  (assertion-strength gap)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 22:32'
updated_date: '2026-05-20 17:46'
labels:
  - compiler
  - diagnostics
  - tests
  - follow-up
  - M0
dependencies:
  - TASK-0089
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
compiler/tests/algo_lower.rs::multiple_effect_purity_violations_each_reported pins .len()==2, the two callee names, and spans[0] != spans[1]. It does NOT pin the exact (line, col) of each violation via offset_to_line_col — only the singular-violation test located_effect_purity_error_has_correct_line_col does. A regression placing the multi-violation spans at distinct-but-wrong positions (the wrong-token-on-the-line bug class) would slip through. Filed from TASK-0089 architecture review (Finding #3, 2026-05-20).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Extend multiple_effect_purity_violations_each_reported to compute expected (line, col) for each callee occurrence via offset_to_line_col and assert each pair (analogous to located_effect_purity_error_has_correct_line_col but for the multi-error path); negative-fixture is the two-effect-stmt-to-pure-kernel pattern; both line:col pairs pinned with no len-only blanket assertion smell
- [x] #2 just test passes; just ci exits 0; no behaviour change for valid input (e2e 30/26/0/4/0; determinism-check byte-identical x2); clippy --workspace --all-targets clean
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. ONBOARD: read PRD §6.2.2 + TASK-0089 (parent) + TASK-0202 (this) + the template test `located_effect_purity_error_has_correct_line_col` and the cycle-3 cascade test `effect_stmt_to_declared_but_failed_kernel_collapses_to_root` to confirm the (line, col) idiom. Verify the emit site (lower.rs:792 Stmt::Effect arm uses `call.callee.span`).
2. ANALYZE the test source layout. Source has 10 lines. Violation 1 is `pure_a(x);` on line 9 (col 1); violation 2 is `pure_b(y);` on line 10 (col 1). `pure_a` appears 3 times (kernel decl line 5, dataflow RHS line 8, effect-stmt line 9); the violation is the 3rd occurrence. `pure_b` appears 2 times (kernel decl line 6, effect-stmt line 10); violation is the 2nd occurrence.
3. EXTEND the test: use `src.match_indices("pure_a")` to find the 3rd (index 2) occurrence; use `src.match_indices("pure_b")` to find the 2nd (index 1) occurrence. Compute expected (line, col) via offset_to_line_col. Assert each err.span.start matches its expected. Add sanity asserts on the literal (line, col) values (9, 1) and (10, 1). Keep the existing `.len() == 2`, callee-name, and `spans[0] != spans[1]` asserts (they remain meaningful in defence-in-depth).
4. STRENGTHEN the docstring: explain WHY the spans land at line-N col-1 (the callee Spanned wraps `call.callee` ident at its identifier-start position; effect-stmt is `<callee>(args);` so callee-start = stmt-start = col 1 for un-indented stmts).
5. RUN the full 7-step gate. If gate green, commit `algo-lower: pin exact (line, col) per multi-error EffectStmt purity violation (TASK-0202)`.
6. MARK Done if both ACs satisfied.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation (commit eab2356)

Doc+test only — no algo/lower.rs or algo/ir.rs change. Strengthened
multiple_effect_purity_violations_each_reported in
nucleus/compiler/tests/algo_lower.rs to pin each violation's exact
(line, col) via offset_to_line_col over src.match_indices.

Per-violation pinning (each computed from the source, with a
literal-(line, col) sanity assert to catch layout drift):
- 1st violation (pure_a): line 9 col 1 — 3rd match_indices('pure_a')
  occurrence (kernel decl line 5, dataflow RHS line 8, effect-stmt
  line 9).
- 2nd violation (pure_b): line 10 col 1 — 2nd match_indices('pure_b')
  occurrence (kernel decl line 6, effect-stmt line 10).

Discrimination strength (the load-bearing reason for this task):
imagine a regression that mis-uses call.args[0].span instead of
call.callee.span in the Stmt::Effect arm — both spans would then
point at the first argument identifier (x and y respectively), i.e.
line 9 col 8 and line 10 col 8. The older blanket assertion
(.len() == 2, callee names, spans[0] != spans[1]) would still pass
under that regression. The new pinned (9, 1) / (10, 1) assertion
bites.

Docstring grounded in the actual emit site (verified by reading
algo/lower.rs:792-828): Stmt::Effect arm at commit 6e77fce emits
EffectCalleeNotEffectful with call.callee.span, which begins at the
callee identifier's first character. Effect statement syntax is
'<callee>(args);' with no leading token, so for un-indented stmts the
callee start coincides with stmt start = col 1. Not a plausible
reconstruction — grounded in the code as read at this commit.

Gate (all under nix develop -c):
- just test: 459 passed, 0 failed, 2 ignored (baseline-equal; only
  in-place assertion strengthening, no new test function added).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 30/26/0/4/0 (required-fail=0). Zero behaviour change.
- just determinism-check x2: 30/26/0/4 byte-identical.
- just determinism-check-negative: bites (NUC_NONDET_PERTURBED_CELLS=26).
- just xbackend-check-negative: bites (NUC_XBACKEND_CORRUPTED_APPLIED=13,
  NUC_XBACKEND_CORRUPTED_DETECTED=1).
- just ci: exit 0.

## Honest limits / follow-ups

The strengthened assertion now bites against the "wrong-token-on-the-
line" regression class (e.g. mis-using call.args[0].span). It does NOT
discriminate against a regression that shifted spans by exactly zero
bytes from call.callee.span — e.g. if some future variant added a
leading sigil/keyword before the callee in the parsed Call, the
"col 1" expectation would need to be revisited. No such variant
exists at this commit, but the literal-(9, 1) / (10, 1) sanity
asserts will surface the question if the surface syntax evolves.

No follow-up filed: both ACs (#1 pin exact (line, col) per violation;
#2 gate green) are met. No new defect surfaced during strengthening
— the existing implementation already lands at the expected
positions.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Strengthened multiple_effect_purity_violations_each_reported (nucleus/compiler/tests/algo_lower.rs) to pin each EffectCalleeNotEffectful span's exact (line, col) via offset_to_line_col over src.match_indices — pure_a violation at (9, 1), pure_b at (10, 1). Doc+test only (no algo/lower.rs or algo/ir.rs change). Discriminates the wrong-token-on-the-line regression class (e.g. call.args[0].span instead of call.callee.span). Gate all green: test 459 passed (baseline), clippy clean, e2e 30/26/0/4/0, determinism-check x2 byte-identical, determinism-check-negative bites (26), xbackend-check-negative bites (13/1), ci exit 0. Commit: eab2356.
<!-- SECTION:FINAL_SUMMARY:END -->
