---
id: TASK-0204
title: >-
  Broaden K×L cascade fixture to exercise all 4 cascade-error variants
  (named-fixture coverage gap)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 22:59'
updated_date: '2026-05-20 16:45'
labels:
  - compiler
  - diagnostics
  - tests
  - follow-up
  - M0
  - cascade-class
dependencies:
  - TASK-0092
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0092 cycle-3 review (mped-architect + qa-test-runner both flagged independently): the new transitive_cascade_collapses_for_any_k_l fixture at compiler/tests/algo_lower.rs:1009-1055 is genuinely parametric in K×L (4×3=12 combinations, all pass), but uses a SINGLE underlying cascade-shape: root const N=1/0; cascade-decls are 'data dki : f32[N]' (always ShapeRefersToNonConst); statements are 'dump(dki)' (always UnknownIdent via Effect-call lookup). The other three cascade-suppression variants — AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1 — and cross-kind cascades (cascade-kernel-sig, cascade-const-via-other-const, dataflow-assign-target on cascade-data) are exercised in mped-architect's 21-probe set + real-driver probes, but NOT in the NAMED fixture. The K×L axis prevents single-shape masking for the bare-call path; broadening the parametric dimension across cascade-error variant + cascade-kind would make the structural guard explicit and persistent. Filed from TASK-0092 cycle-3 review (2026-05-20).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Extend (or add a sibling parametric fixture to) transitive_cascade_collapses_for_any_k_l so that cascade-error variant is a third parametric dimension iterated over: UnknownIdent (existing), AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1 — at least 4 variants × K∈{1,2,3,5} × L∈{1,2,3} measured; each combination asserts errors().len()==1 AND the surviving error is the root kind (not a leaked variant)
- [ ] #2 Add at least 3 cross-kind cascade shapes: cascade-data-via-shape (current), cascade-kernel-via-signature-shape, cascade-const-via-other-const; each transitively poisons and downstream references collapse to 1
- [ ] #3 If the broader fixture surfaces a NEW defect (an axis the transitive-poison fix does not actually cover), STOP and file a precise follow-up rather than papering over — the honest-stop discipline applies; the 5th-recurrence-closed claim must remain measurement-backed
- [ ] #4 just test passes; just ci exit 0; clippy --workspace --all-targets clean; no behaviour change for valid input (e2e 30/26/0/4/0; det-check byte-identical x2)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Broaden the named K×L cascade fixture to be a 4-dimensional sweep: (cascade-kind in {data-via-shape, kernel-via-signature-shape, const-via-other-const}) × (cascade-error variant in {UnknownIdent, AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst-at-depth>1}) × (K in {1,2,3,5} cascade-decls) × (L in {1,2,3} references-per-cascade-decl). For each combination: build the program, assert errors.len()==1, kind == root ConstDivByZero{N}, no leaked cascade-error variant of any of the 4 kinds. The existing named fixture transitive_cascade_collapses_for_any_k_l is the stylistic template (same assertion idiom, same parametric loop, but parameter-broadened). NOT papered over if a new defect surfaces — STOP and file precise follow-up.
<!-- SECTION:PLAN:END -->
