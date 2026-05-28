---
id: TASK-0359
title: >-
  e2e coverage-test gate bands hardcode top=M6 — revisit when first M7+
  [[required]] cell lands
status: To Do
assignee: []
created_date: '2026-05-28 04:23'
updated_date: '2026-05-28 04:23'
labels:
  - e2e
  - validation
  - tech-debt
  - cycle-228-follow-up
dependencies:
  - TASK-0045
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-228 (TASK-0346) architect P3, advisory. After extending the milestone enum to M0..M11, two e2e coverage tests in nucleus/e2e/src/tests.rs still iterate HARDCODED gate bands that assume the highest [[required]] milestone is M6: (a) real_manifest_has_no_coverage_gaps_at_every_milestone stops its gate loop at Some(Milestone(4)); (b) required_counts_strictly_grow_per_milestone discovers 'top' from [[required]] rows only (max tag today = M6) and loops 1..=top, so it never reaches M7..M11. This is CORRECT today (M7..M11 are skip-only, zero required obligations) and was NOT introduced by TASK-0346. It becomes load-bearing the moment a future milestone lands its first [[required]] cell (M7 = TASK-0045 MPI blocking): at that point the hardcoded None..M4 band list and the implicit top==M6 assumption must be widened so the cumulative-coverage + strict-grow invariants actually exercise the new tier. Fix when M7 work starts: parameterise the gate-band iteration off Milestone::MAX (or the manifest's actual max required tag) instead of a literal. LOW — latent, fires only at M7+ required work.
<!-- SECTION:DESCRIPTION:END -->
