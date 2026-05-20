---
id: TASK-0201
title: Evaluate DataflowStmt-RHS-Pure rule (PRD vs grammar vs examples conflict)
status: Done
assignee:
  - '@self'
created_date: '2026-05-19 21:47'
updated_date: '2026-05-20 19:58'
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
- [x] #1 Investigate whether PRD §6.2.2 row 77 was intended bidirectional or whether grammar §2 note 5 (unidirectional) is the canonical rule
- [ ] #2 If bidirectional: redesign IO semantics across all examples (likely introduce a separate IO/capture form distinct from kernels) — DO NOT just rewrite examples to add intermediate pure kernels (that hides the side-effect)
- [x] #3 If unidirectional (the grammar reading): update PRD §6.2.2 table-row 77 wording AND the algo/ir.rs module-doc to remove the misleading bidirectional language, citing grammar §2 note 5 as canonical
- [x] #4 Outcome recorded as a decision-NNNN entry under backlog/decisions/ so the question does not silently re-surface
- [x] #5 TASK-0089 closes ONLY the EffectStmt direction; this task is the follow-up that decides the other direction. No code change to the algo lowering pass is forced by this task — its output is a written decision
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Notes

Resolved spec ambiguity around DataflowStmt-RHS purity. Decision: **unidirectional** — grammar §2 note 5 is canonical; the DataflowStmt RHS may call either pure or effectful kernels. No code change.

Recorded as `backlog/decisions/decision-0004`. PRD §2 row at line 77 tightened from the loose `where pure mandatory; where !effectful opt-in` to a v2-accurate phrasing naming the actual mechanism (kernel-level pure/effectful annotation + EffectStmt rule). algo/ir.rs module-doc and EffectCalleeNotEffectful variant doc lose their TASK-0201 hedges and cite decision-0004. tests/algo_lower.rs commentary retargeted from "pending TASK-0201" to "fixed by decision-0004" — the load-bearing `pure_dataflow_with_effectful_rhs_load_lowers` regression-guard test stays in place.

### Per-AC status
- AC#1 — DONE. Unidirectional decided. Verbatim citations of PRD line 77 and grammar §2 note 5 (lines 137-139) live in the decision file.
- AC#2 — N/A (the IF bidirectional branch). Recorded as such in decision-0004 consequences.
- AC#3 — DONE. PRD line 77 tightened; algo/ir.rs module-doc updated; EffectCalleeNotEffectful inline doc updated; algo_lower.rs test commentary retargeted. Grammar §2 note 5 cited as canonical throughout.
- AC#4 — DONE. `backlog/decisions/decision-0004 - DataflowStmt-RHS-purity-is-unidirectional-grammar-note-5-canonical-no-RHS-pure-enforcement.md`.
- AC#5 — DONE. TASK-0089 retains its EffectStmt-only enforcement; zero lowering-pass code change.

### Doc sweep (cycle-5 discipline)
Grepped for `RHS.*pur|<--.*pur|where pure|where !effectful|DataflowStmt.*pur` across `*.rs` and `*.md`. Carriers audited:
- PRD.md:77 — UPDATED (loose 2013-vs-v2 table cell).
- nucleus/compiler/src/algo/ir.rs module-doc — UPDATED (TASK-0089's "pending TASK-0201" hedge replaced).
- nucleus/compiler/src/algo/ir.rs EffectCalleeNotEffectful variant doc — UPDATED (TASK-0201 reference → decision-0004).
- nucleus/compiler/tests/algo_lower.rs lines 1746-1817 — UPDATED (commentary retargeted; load-bearing test untouched).
- SKETCH.md:60-78 — LEFT (pre-v2 brainstorming using 2013-style `where pure {{...}}` block syntax; not a normative spec carrier; modernising it is a separate housekeeping task).
- examples/05-stencil/README.md:21 — LEFT (historical pointer to legacy syntax; descriptive, not normative).
- backlog/tasks/task-0005/0012/0031/0078/0089/0188/0201 — backlog history, NOT rewritten per project convention.

Verified zero remaining `TASK-0201` references in source/docs outside `backlog/`.

### Gate (7-step, fresh runs)
1. `just test`: 468 / 0 / 2.
2. `cargo clippy --workspace --all-targets -- -D warnings`: clean (only "Git tree dirty" — expected pre-commit).
3. `just e2e`: total 30, pass 26, fail 0, skipped 4, required-fail 0.
4. `just determinism-check` x2: byte-identical both runs.
5. `just determinism-check-negative`: 26/30 cells perturbed, gate bites.
6. `just xbackend-check-negative`: 13 mp-tcp cells corrupted, 1 detected, gate bites.
7. `just ci`: EXIT=0.

### Honest limits / followups
- The decision is a SPEC decision recorded against the current evidence (PRD wording, grammar wording, example usage). If a future thesis-claim or backend genuinely requires DataflowStmt-RHS-Pure enforcement, reopen decision-0004 with the new evidence — do not silently overcommit past the grammar.
- SKETCH.md was deliberately not touched; it is a pre-v2 brainstorming artefact that uses 2013-style `where pure {{...}}` block syntax. A future modernisation task can fold it into the v2 form or move it to cruft/.
- No new tasks filed; no new latent risks surfaced by this change.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
**Decision: unidirectional** (grammar §2 note 5 canonical). decision-0004 recorded; no code change. PRD §2 line 77 tightened; algo/ir.rs module-doc + variant doc + algo_lower.rs commentary retargeted from "pending TASK-0201" to "fixed by decision-0004". Comprehensive doc-sweep done — zero TASK-0201 references remain in source/docs outside backlog/. Gate 468/0/2 + 30/26/0/4/0 + 2x byte-identical + 2 neg-gates bite + just ci EXIT 0. Commit 6562333.
<!-- SECTION:FINAL_SUMMARY:END -->
