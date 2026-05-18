---
id: TASK-0159
title: >-
  Event contract must carry loop-nest structure (rolled Repeat: iter-var name +
  symbolic/const bounds) for EventList-only codegen
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 16:42'
updated_date: '2026-05-18 22:39'
labels:
  - M2
  - compiler
  - backend
  - blocker
dependencies:
  - TASK-0160
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Event::Loop { iter_var: IterVar, range: Range<i64>, body: Vec<Event> } variant to event.rs (option (a)). Manual Hash hashing iter_var + range.start/end + recursing body (mirror IterTile/FireBinding precedent; Range<i64> & IterVar are Hash). serde-gated like siblings. Doc the variant: structure-preserving, mirrors ACFGNode::Repeat; trailing-partial-tile = two SIBLING Loops with different ranges (NOT one parameterised loop), falls out of mirroring Sequence/Repeat.
2. petri_to_events.rs walk(): replace the Repeat unroll arm with structure preservation. Build the loop body by walking into a SCRATCH per-worker map, then wrap each worker's produced body slice in one Event::Loop and append to that worker's real list. Keep saturating/degenerate-range semantics in the carried Range (no firing-count math). Empty body for a worker => still emit Loop (carries structure) — decide & document.
3. Update petri_to_events.rs + acfg_to_petri.rs module docs (AC#4): correct the stale "we unroll the EventList" note; state Net unrolls (analysis: boundedness/deadlock, acfg_to_petri UNTOUCHED) vs EventList preserves structure (codegen contract). DO NOT touch acfg_to_petri unroll.
4. Flip tests/petri_to_events.rs: repeat_unrolls_in_event_list -> repeat_preserves_structure_in_event_list (assert one Event::Loop with range 0..3 wrapping the Fire, NOT 3 flat Fires). repeat_empty_range -> assert a Loop with empty range still emitted (or documented choice). Add a flatten/recurse helper; update eventlist_alone_reconstructs_stencil_kernel_call, eventlist_carries_bindings_for_all_e2e_examples, e2e_example_02_* , e2e_example_01_* to recurse into Loop bodies (flip stale flat-iteration assumption, keep coverage). Add a 05-stencil/blocked.sched.nuc test asserting the trailing-partial-tile = sibling Loops with different ranges shape (TASK-0142 forward-carried).
5. event.rs unit tests: Loop order preserved, nested Loop recursion, serde roundtrip, Hash distinct-structure inequality.
6. AC#2: ACFGNode::Repeat.range is concrete Range<i64> (H-1 folded to const at acfg.rs ~695 BEFORE the ACFG). Symbolic bound does not exist at this layer. Carry the concrete range. AC#2 genuinely blocked on TASK-0160 (types/consts no-fold). Add dep task-0159->task-0160, leave AC#2 UNCHECKED with honest note. Do NOT fake.
7. Gate before every commit: nix develop -c just test / e2e / determinism-check / determinism-check-negative / clippy -D warnings. e2e+determinism MUST be byte-identical (no EventList codegen consumer). Commit per logical unit, no push, no AI credit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0142: CONFIRMED how loop structure currently survives to the backend. The pthreads-sync single-worker path (backends/pthreads-sync/src/lib.rs render_main_rs, ~lines 25-33 and 78-81) does NOT use the ACFG loop structure at all — it walks LinkedIR::algo SOURCE IrStmt directly and emits `for VAR in lo..hi` from the source loop. Consequence for this task: block_transform/tiling (and any other ACFG-level loop rewrite) is STRUCTURALLY INVISIBLE in the single-worker emitted code today; it only shapes the ACFG consumed by acfg_to_petri/petri_to_events/boundedness/deadlock. Also: acfg_to_petri/petri_to_events UNROLL every Repeat by range.end-range.start (static i64 bounds) — the Petri/Event path has NO rolled-loop representation; loop nest + iter-var-name + symbolic bounds are LOST before events. So EventList-only codegen genuinely needs the new rolled-Repeat event contract this task proposes; it cannot recover loop structure from the current Event stream or from acfg_to_petri output. TASK-0142 deliberately kept the tiling as static-range Repeat decomposition (full nest + trailing partial tile) precisely to avoid a dynamic Repeat bound rippling into these unroll-by-length consumers — a rolled-Repeat event contract here must decide how it represents a partial/trailing tile (a tile whose inner trip count differs from the others).
<!-- SECTION:NOTES:END -->
