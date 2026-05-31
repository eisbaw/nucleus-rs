---
id: TASK-0383
title: >-
  just ci RED: check-mega-files fails on sidecar.rs (1164) +
  embedded-pattern/tests.rs (1030) — pre-existing, untracked
status: To Do
assignee: []
created_date: '2026-05-31 02:39'
labels:
  - ci
  - hygiene
  - mega-file
  - tech-debt
dependencies:
  - TASK-0340
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRE-EXISTING just ci gate failure, surfaced by the TASK-0370 cycle-220 review gate (qa-test-runner). just check-mega-files (an arm of just ci) FAILS direction-A (exit 1): two files exceed the 1000-LoC fence and are NOT in the allow-list, NOT covered by any existing TASK-0340 split slice: (1) nucleus/nucleus-compiler/src/sidecar.rs = 1164 LoC; (2) nucleus/backends/embedded-pattern/src/tests.rs = 1030 LoC. Both grew past 1000 during recent cycles (gather / embedded work, last touched 2026-05-30) and were never split nor allow-listed, so just ci has been RED on this arm since they crossed the threshold. NOTE: the cheap pre-commit subset (build/clippy/test/test-release/e2e) is GREEN and unaffected; this is a full-just-ci-only failure. Resolution per the recipe fix-options: split each along its module-doc seams (TASK-0340 AC#2 preferred), OR add to the check-mega-files allow-list with a one-line rationale if genuinely a coherent unit. Sibling of the TASK-0340 mega-file split epic.
<!-- SECTION:DESCRIPTION:END -->
