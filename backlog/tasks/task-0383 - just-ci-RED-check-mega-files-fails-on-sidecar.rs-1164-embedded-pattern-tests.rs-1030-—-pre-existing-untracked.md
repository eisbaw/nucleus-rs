---
id: TASK-0383
title: >-
  just ci RED: check-mega-files fails on sidecar.rs (1164) +
  embedded-pattern/tests.rs (1030) — pre-existing, untracked
status: In Progress
assignee:
  - '@orchestrator'
created_date: '2026-05-31 02:39'
updated_date: '2026-05-31 02:58'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
SPLIT (not allow-list) per TASK-0340 AC#2:
Part A: extract sidecar.rs inline `#[cfg(test)] mod cumulative_tests` (~265 LoC, lines 899-1164) into `sidecar/cumulative_tests.rs` child module via `mod cumulative_tests;` decl (2018-edition file-module dir resolution; precedent sched/, acfg/). `use super::*` reaches private parent items.
Part B: extract embedded-pattern/tests.rs BIN-shape tests (`*_bin_*`, check-frame bin tests + the bin helper) into `tests/bin_shape.rs` child module via `mod bin_shape;`. Helpers stay in tests.rs, child calls via `super::`.
Pure code-move, ZERO behavior change. Same test count before/after.
DoD: both files <1000 LoC via split, just check-mega-files GREEN, all tests pass at same count. Then run check-doc-citation-staleness + check-doc-links + clippy.
<!-- SECTION:PLAN:END -->
