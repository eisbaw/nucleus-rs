---
id: TASK-0325
title: >-
  check_no_silent_elision_risk silent sibling: per-element fan-out (src,dst)
  same-worker skip can elide unsafely (TASK-0324 cycle-144 reviewer P1.1
  fold-back)
status: To Do
assignee: []
created_date: '2026-05-25 14:18'
labels:
  - compiler
  - transfer_inject
  - silent-sibling
  - silent-miscompile
  - M6
  - forward-carried-from-TASK-0324
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 cycle-144 reviewer (mped-architect) P1.1 finding: the validator `check_no_silent_elision_risk` only fires when `producer_workers == &consumer_workers` (set equality at the line-2501 short-circuit). The structurally-identical sibling at the per-element `if src == dst { continue; }` inside the fan-out loop (`build_waits_for_op`, currently at transfer_inject.rs:2994 — search for `if src == dst` inside the cartesian-product loop) can elide unsafely for partially-overlapping (not equal) worker sets.

## Scenario (not currently exercised in-tree)

producer_workers = {w0, w1, w2, w3}, consumer_workers = {w0, w1, w2, w3, w4}. The set-equality check at line 2501 does NOT fire (sets differ). The fan-out emits 5×4 = 20 (src, dst) pairs, of which 4 are eliminated by `src == dst` (the (w_i, w_i) self-pairs for i in 0..4). The remaining 16 emit cross-worker pairs.

Each (w_i, w_i) self-skip is the same structural pattern as the line-2501 elision: a same-worker on both sides where the consumer's read could reach outside the local producer's slice. The line-2501 validator does NOT cover this case because the test is at the WHOLE-SET level (BTreeSet equality), not at the PER-ELEMENT (src, dst) level.

## Why this is a recurring silent-sibling per feedback-silent-sibling-defect

cycle-128/138/140/141/142/142b/143/144 has fired the silent-sibling meta-rule 8 times now. The cycle-144 architect caught this on read-only review — the cycle-144 implementer (orchestrator) did not search for the per-element analogue of the set-equality short-circuit before claiming closure of AC#2.

## Acceptance criteria

### AC#1: detection

Extend `check_no_silent_elision_risk` (or add a sibling helper) to walk the cartesian product of producer × consumer worker sets and apply the same per-axis discriminator to every same-worker (src == dst) pair in the cartesian product, NOT just the whole-set case.

### AC#2: positive + negative tests

- Positive fixture: producer={w0..w3}, consumer={w0..w3, w4}, consumer reads at a non-aligned slice on a partition axis → expect `Err(SameSetSilentElisionRisk)` (the same variant; the elision is structurally identical to the set-equality case).
- Negative fixture: same shape but consumer's read iv matches producer's partition iv on every partitioned axis → expect `Ok`.

### AC#3: matrix audit

No in-tree schedule today places a kernel on partially-overlapping worker sets. Audit the 06/07/13 schedules + any new schedule landing alongside this task. If none is exercising the pattern, AC#3 is met. Document the dormant-but-defended state in the validator's docstring.

## Dependencies

- TASK-0324 (cycle-144 same-set elision validator) — this task generalises the validator to the per-element analogue.

## Honest scope

- LATENT defect. No in-tree schedule today exercises partially-overlapping placements. The line-2994 silent skip is dormant. Filing the gap so a future M6+ schedule landing this pattern is caught by the existing architectural check.

- Reviewer P1.1 was specifically noted as a recurring silent-sibling firing. Memory entry `feedback-silent-sibling-defect` should be updated with cycle-144 P1.1 as the 8th firing.
<!-- SECTION:DESCRIPTION:END -->
