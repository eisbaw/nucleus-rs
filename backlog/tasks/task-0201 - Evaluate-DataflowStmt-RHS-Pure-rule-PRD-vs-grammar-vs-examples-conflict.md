---
id: TASK-0201
title: Evaluate DataflowStmt-RHS-Pure rule (PRD vs grammar vs examples conflict)
status: To Do
assignee: []
created_date: '2026-05-19 21:47'
labels:
  - compiler
  - ir
  - follow-up
  - M0
  - spec
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Spec ambiguity surfaced during TASK-0089 onboarding. The AlgoIR doc-comment (algo/ir.rs line 27-30) and PRD §6.2.2 table-row 77 (where pure mandatory; where !effectful opt-in) suggest dataflow-stmt RHS Call must reference a Pure kernel. But grammar §2 note 5 only specifies one direction (EffectStmt callee must be effectful) and EVERY shipped example puts an effectful load/capture kernel on the RHS of <-- (01..07 use load_input/load_image/load_a; 13 uses load_input; 14 uses fe_capture/rf_receive). Implementing the strict bidirectional rule would trip every example. TASK-0089 ships only the unidirectional grammar-supported direction. This task: decide whether DataflowStmt-RHS-Pure is intended; if YES, redesign IO across all examples; if NO, update PRD §6.2.2 row 77 and the AlgoIR doc-comment to remove the misleading bidirectional language. NOTE: shipping examples currently rely on the value-and-effect semantics of effectful kernels returning data (e.g. fe_capture reads a fresh audio frame each call AND returns it as a value). The 2013 sin (where-clauses could side-effect silently) is closed by requiring the EffectStmt direction; the dataflow direction is a separate, weaker claim.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Investigate whether PRD §6.2.2 row 77 was intended bidirectional or whether grammar §2 note 5 (unidirectional) is the canonical rule
- [ ] #2 If bidirectional: redesign IO semantics across all examples (likely introduce a separate IO/capture form distinct from kernels) — DO NOT just rewrite examples to add intermediate pure kernels (that hides the side-effect)
- [ ] #3 If unidirectional (the grammar reading): update PRD §6.2.2 table-row 77 wording AND the algo/ir.rs module-doc to remove the misleading bidirectional language, citing grammar §2 note 5 as canonical
- [ ] #4 Outcome recorded as a decision-NNNN entry under backlog/decisions/ so the question does not silently re-surface
- [ ] #5 TASK-0089 closes ONLY the EffectStmt direction; this task is the follow-up that decides the other direction. No code change to the algo lowering pass is forced by this task — its output is a written decision
<!-- AC:END -->
