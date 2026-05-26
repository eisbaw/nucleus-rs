---
id: TASK-0342
title: 'check-mega-files scope: include nucleus/e2e/src/ (TASK-0340 cycle-185 QA P3.1)'
status: To Do
assignee: []
created_date: '2026-05-26 16:38'
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
