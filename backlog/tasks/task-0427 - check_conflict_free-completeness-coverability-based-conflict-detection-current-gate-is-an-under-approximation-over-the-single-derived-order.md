---
id: TASK-0427
title: >-
  check_conflict_free completeness: coverability-based conflict detection
  (current gate is an under-approximation over the single derived order)
status: To Do
assignee: []
created_date: '2026-06-02 06:45'
labels:
  - compiler
  - petri
  - fail-loud
  - prd-invariant-audit
  - cycle-241-followup
  - completeness
dependencies:
  - TASK-0421
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0421 (architect review P1). check_conflict_free is SOUND in the safe direction (never false-rejects a valid build) but INCOMPLETE: it replays the single derive_firing_order and inspects only markings along that one order. PRD §8.6 single-order-replay ≡ all-reachable-markings holds only FOR conflict-free nets (the property being checked), so a free-choice conflict reachable only on a NON-derived interleaving is NOT detected (false negative). Architect constructed a concrete counterexample: places s1:1,s2:1,p(cap2):0; load1(s1->p), cons_x(p-1), load2(s2->p), cons_y(p-2), cons_z(p-2). Derived order fires cons_x as soon as p=1, draining p, so the p=2 marking that co-enables cons_y+cons_z is reachable but never visited -> check returns Ok, missing the conflict. This is documented as an Honest limitation in net_soundness.rs (TASK-0421 fold-back). SCOPE (if ever picked up): a coverability/state-space conflict check (bounded BFS like the proptest_petri.rs oracle) that detects co-enablement at ANY reachable marking, not just the derived order. LOW priority: the gate is a provably-dead-today tripwire (acfg_to_net control-place threading makes conflicts structurally impossible on every shipping schedule), so the under-approximation has zero impact today; this only matters if a future inject-pass regression emitted an off-order conflict. Pointer: nucleus/nucleus-compiler/src/passes/net_soundness.rs check_conflict_free Honest-limitation section.
<!-- SECTION:DESCRIPTION:END -->
