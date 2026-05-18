---
id: TASK-0159
title: >-
  Event contract must carry loop-nest structure (rolled Repeat: iter-var name +
  symbolic/const bounds) for EventList-only codegen
status: To Do
assignee: []
created_date: '2026-05-18 16:42'
labels:
  - M2
  - compiler
  - backend
  - blocker
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124 AC#2 byte-identical. acfg_to_events UNROLLS every ACFGNode::Repeat into N flat Fire copies (proven by repeat_unrolls_in_event_list; documented in petri_to_events.rs M2 trade and acfg_to_petri). The pthreads-sync backend emits ROLLED loops (for y in (1_i64)..((16_i64-1_i64))) by walking AlgoIR IrStmt::For. A backend consuming ONLY the EventList cannot reproduce the rolled loop (loop var name, symbolic bound expr H-1, the for-structure) from N identical unrolled Fires — so it cannot be byte-identical to master, and emit.rs unit test asserts the rolled 256_i64 bound. TASK-0156 solved the analogous VALUE-binding gap by enriching Event::Fire; this is the LOOP-STRUCTURE analogue. Decide: (a) stop unrolling — add Event::Loop{iter_var,range/bound-expr,body:Vec<Event>} (nested, matches ACFGNode::Repeat) so the projection is structure-preserving; or (b) keep flat list but add a per-Fire IterTile with the loop coordinate + a sidecar loop table. Option (a) is cleaner and matches PRD §8.3 IterTile intent + the ACFG tree. Must keep determinism + bit-identical e2e + acfg_to_petri/boundedness/deadlock (which currently rely on unrolled firing order — coordinate carefully).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Event projection preserves loop-nest structure (no blanket unroll) OR an equivalent sidecar carries iter-var name + symbolic loop bound expr so a backend can re-emit the rolled for-loop verbatim
- [ ] #2 Symbolic/const loop bound expression (e.g. H-1 unevaluated) survives to the contract; backend can render (16_i64 - 1_i64) without AlgoIR
- [ ] #3 Determinism + bit-identical e2e for 01/02/03/05/07 preserved; acfg_to_petri / boundedness / deadlock passes still correct (they consume the unrolled order today)
- [ ] #4 petri_to_events + acfg_to_petri module docs updated to reflect the new contract; the stale M2 'we unroll' note corrected
<!-- AC:END -->
