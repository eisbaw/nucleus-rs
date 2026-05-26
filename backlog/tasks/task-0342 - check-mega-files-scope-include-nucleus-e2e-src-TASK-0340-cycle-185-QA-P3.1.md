---
id: TASK-0342
title: 'check-mega-files scope: include nucleus/e2e/src/ (TASK-0340 cycle-185 QA P3.1)'
status: Done
assignee: []
created_date: '2026-05-26 16:38'
updated_date: '2026-05-26 23:57'
labels:
  - tech-debt
  - hygiene
  - check-mega-files
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Filed cycle 185b in response to qa-test-runner P3.1: the cycle-176 check-mega-files recipe scope explicitly EXCLUDES nucleus/e2e/src/ — neither pre-carve main.rs (7316 LoC) nor post-carve main.rs (4716 LoC) nor new tests.rs (2635 LoC) entered the gate. AC#5's '1000-LoC threshold' wording does not bind e2e today. The cycle-176 architect P2.3 had a defensible rationale (e2e/src is harness code, different shape than compiler/backend code), but the scope gap is now a documentation/expectation lag. Option A (extend + allow-list) is the cleanest fix; option B (document the exclusion) is acceptable if e2e is meant to be permanently exempt.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 just check-mega-files recipe scope extended to include nucleus/e2e/src/. The recipe currently scans backend-common/src, nucleus-compiler/src, backends/*/src only (cycle-176 architect P2.3 explicit scope decision)
- [ ] #2 Either (a) extend the scope and add nucleus/e2e/src/main.rs (4716 LoC post slice-10) and nucleus/e2e/src/tests.rs (2635 LoC post slice-10) to the allow-list with one-line rationale each, OR (b) document the scope exclusion in the recipe header so future maintainers know e2e is intentionally not policed
- [ ] #3 Decision recorded as an addendum to TASK-0340 AC#5 close-out (cycle 185 + 185b architect P3.6 — AC checkboxes lag the cycle's actual close)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 190 close — Option A (extend + allow-list both files)

Decision: extended check-mega-files scope to include nucleus/e2e/src; allow-listed nucleus/e2e/src/main.rs (4716 LoC post slice-10) and nucleus/e2e/src/tests.rs (2638 LoC post slice-10) with the same printf-list shape the canonical allow-list uses. Recipe docstring updated to (a) explicitly list nucleus/e2e/src in the scope walk and (b) explain the cycle-190 motivation (qa cycle-185b P3.1 surfaced the prior exclusion as a documentation/expectation lag after TASK-0340 slice-10 carved e2e/src).

Why Option A over Option B (document the exclusion): Option A keeps the fence's coverage symmetric with the rest of nucleus/**/src — e2e harness code is allowed to be large but the FENCE itself polices growth and stale-entry. Option B would have left a permanent scope hole the next time e2e grew. The cycle-176 architect P2.3 'different shape' rationale for excluding e2e was true at the time but does not justify exempting from the regression-fence; it justifies allow-listing.

### Gate
- just check-mega-files: OK (positive arm passes with 2 new allow-list entries; negative arm + stale-arm structurally unchanged).

### AC coverage
- AC#1: scope extended (justfile:check-mega-files find arguments now include nucleus/e2e/src).
- AC#2: Option A landed (allow-list extended with both files + recipe docstring updated).
- AC#3: addendum on TASK-0340 — to be done in same commit.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 190 Option A: extended check-mega-files recipe scope to include nucleus/e2e/src; allow-listed main.rs (4716 LoC) + tests.rs (2638 LoC) with one-line rationale via recipe docstring. Recipe passes; no scope hole next time e2e grows. AC#1+#2+#3 satisfied.
<!-- SECTION:FINAL_SUMMARY:END -->
