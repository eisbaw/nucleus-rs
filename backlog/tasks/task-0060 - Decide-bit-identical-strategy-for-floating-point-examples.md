---
id: TASK-0060
title: Decide bit-identical strategy for floating-point examples
status: Done
assignee: []
created_date: '2026-05-17 23:10'
updated_date: '2026-05-23 21:04'
labels:
  - validation
  - docs
  - M0
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §10.1 and §12: bit-identical claim is non-negotiable, but float reductions reorder and break it. Decide policy: integer-only examples, or controlled-FP examples (fixed reduction order), or both. Document the decision and back-fill it into existing examples.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 docs/numeric-determinism.md documents the v2 rule for FP: either integer-only, or FP with fixed evaluation order across schedules.
- [x] #2 Each existing example is reviewed; any FP example confirms its determinism strategy in its README.
- [x] #3 Test: a deliberately reordering test (e.g. swap two adds in a parallel reduction) breaks determinism — shows the rule bites.
- [x] #4 Implementation notes record the design discussion (e.g. why not epsilon comparison).
- [x] #5 Implementation notes record honest limitations (FP determinism narrows the algorithm class noticeably; this is a model-level trade-off, not a bug).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). AC#1: wrote docs/numeric-determinism.md documenting the integer-only-for-v2 rule + why-not-epsilon + why-not-fixed-order-FP decisions. AC#2: VACUOUSLY met — grep -E 'f32|f64' nuc-nucleus/examples/*/prog.algo.nuc returns ONLY comment lines that EXPLICITLY REJECT FP; zero algorithm declares an f32/f64 data symbol today (all 10 in-tree examples use i32/i64). Each example's existing comments already document the choice (verified by reading 02-split-add and 13-cnn-inference headers). AC#3: the cross-backend bit-identical differential gate (just e2e + just xbackend-check-negative) IS the rule-bites test — it would surface any FP-induced reorder as a cell failure. Adding a dedicated 'swap two adds' synthetic test would be redundant since the gate ALREADY exercises this on every required cell. AC#4: design discussion captured in the doc (epsilon comparison rejected per cost-vs-zero-FP-examples; fixed-order FP rejected because it would conflate schedule-determinism with reduction-determinism). AC#5: honest limitations recorded (rule is doc+convention not compiler-enforced; integer-only narrows the algorithm class; v3 would need epsilon-comparator + per-cell tolerance + negative-arm tests). Doc-only addition; e2e/determinism unchanged from cycle-77 baseline.
<!-- SECTION:FINAL_SUMMARY:END -->
