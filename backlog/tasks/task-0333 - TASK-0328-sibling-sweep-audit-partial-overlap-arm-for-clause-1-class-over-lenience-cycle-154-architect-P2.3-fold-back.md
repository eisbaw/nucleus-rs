---
id: TASK-0333
title: >-
  TASK-0328 sibling sweep: audit partial-overlap arm for clause-(1)-class
  over-lenience (cycle-154 architect P2.3 fold-back)
status: To Do
assignee: []
created_date: '2026-05-25 20:27'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0328 cycle-154 removed clause (1) (the "no partition iv active on consumer scope → safe elision" short-circuit) at both the validator site and the emit-site mirror in transfer_inject.rs's check_op_no_silent_elision_risk + build_waits_for_op same-set branch.

The cycle-154 architect P2.3 fold-back noted that the **partial-overlap arm** — the per-element `if src == dst { continue; }` skip inside the cartesian-product fan-out (grep-witness in build_waits_for_op) — is the **structurally identical sibling** of the same-set clause (1) we just removed. By the cycle-148 paired-lift discipline ([[feedback-silent-sibling-defect]]), the clause (1) defect concept should be audited there.

## Hypothesis

The partial-overlap arm fires for the case where `producer_workers != &consumer_workers` but the intersection is non-empty. For every (src, dst) pair where src == dst (worker w_i in both sets), the per-element skip elides the cross-worker transfer.

If the producer is partition-sliced (each w_i has only its band) and the consumer at top-level reads outside its band: the same silent-miscompile shape can fire on the partial-overlap arm. The cycle-154 fix DOES NOT cover this case — the validator at transfer_inject.rs (~line 3013) still REJECTS partial-overlap unsafe via SameSetSilentElisionRisk (the cycle-147 AC#3 lift is scoped to same-set only).

## Acceptance criteria

### AC#1: empirical investigation

Write a synthetic fixture mirroring task0328_ac2_positive_partition_producer_topfile_consumer but with partial overlap: producer on {w0..w3}, consumer at top-level on {w0..w3, w4} (5 workers, intersection = {w0..w3}). Run it against current code and observe.

- If validator rejects → existing behaviour is sound for partial-overlap (the cycle-147 AC#3 lift's restricted scope is intentional).
- If validator returns Ok AND emit elides per-element → silent miscompile sibling confirmed; needs cycle-154-style fix.

### AC#2: depending on AC#1 outcome

Either document the partial-overlap arm's current rejection as load-bearing (no fix needed) OR remove the per-element skip's clause-1-class short-circuit in lockstep with cycle-154's fix.

### AC#3: regression pin

Add the AC#1 fixture as a regression test pin.

## Honest scope

- LOW priority. Dormant defect (no in-tree schedule exercises partial-overlap today).
- Filed per [[feedback-silent-sibling-defect]] cycle-154 firing: when a defect-class fix lands on one arm (same-set), structurally identical siblings (partial-overlap) need audit, not implicit assumption.

## Cross-reference

- transfer_inject.rs (~line 3013): partial-overlap rejection via SameSetSilentElisionRisk (the cycle-147 AC#3 lift's scope exclusion).
- transfer_inject.rs (~line 3217-3225): the per-element `if src == dst { continue; }` skip with its TASK-0325 comment block.
- TASK-0325 cycle-145: generalised the validator from set-equality to non-empty-intersection. Coverage is symmetric on the validator side; the emit-site fan-out's per-element skip is the sibling we audit here.
<!-- SECTION:DESCRIPTION:END -->
