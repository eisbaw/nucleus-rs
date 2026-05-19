---
id: TASK-0159
title: >-
  Event contract must carry loop-nest structure (rolled Repeat: iter-var name +
  symbolic/const bounds) for EventList-only codegen
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 16:42'
updated_date: '2026-05-19 00:25'
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
- [x] #1 Event projection preserves loop-nest structure (no blanket unroll) OR an equivalent sidecar carries iter-var name + symbolic loop bound expr so a backend can re-emit the rolled for-loop verbatim
- [x] #2 Symbolic/const loop bound expression (e.g. H-1 unevaluated) survives to the contract; backend can render (16_i64 - 1_i64) without AlgoIR
- [x] #3 Determinism + bit-identical e2e for 01/02/03/05/07 preserved; acfg_to_petri / boundedness / deadlock passes still correct (they consume the unrolled order today)
- [x] #4 petri_to_events + acfg_to_petri module docs updated to reflect the new contract; the stale M2 'we unroll' note corrected
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

TASK-0159 implemented (commit ee309ff). Option (a): added Event::Loop { iter_var, range: Range<i64>, body: Vec<Event> } to event.rs, mirroring ACFGNode::Repeat. petri_to_events::walk now projects a Repeat body ONCE into a scratch per-worker BTreeMap and wraps each worker slice in one Event::Loop; nested Repeat recurses to nested Loop. acfg_to_petri UNTOUCHED (analysis Net still unrolls — boundedness/deadlock/determinism decoupled by design).

GATE (actual): just test 33 ok-groups / 0 failed. e2e total 10 pass 8 fail 0 skip 2 required-fail 0 (byte-identical — confirmed no non-test EventList codegen consumer; driver/pthreads-sync walk AlgoIR). determinism-check 10/8/0/2 byte-identical. determinism-check-negative correctly bites. clippy --workspace -D warnings clean.

AC#1 MET (no blanket unroll). AC#3 MET (determinism + bit-identical e2e; analyses untouched). AC#4 MET (petri_to_events + acfg_to_petri module docs corrected).

AC#2 NOT MET — honestly deferred. ACFGNode::Repeat.range is a concrete Range<i64>; build_acfg eval_const folds H-1 -> 15 (panics on non-const) at acfg.rs ~694-697, BEFORE the ACFG layer this pass reads. The symbolic expr does not exist here; carrying it requires not-folding at lowering = TASK-0160 (types/consts; its AC#2 explicitly owns "pre-resolved loop-bound info reaches the contract"). Added dep task-0159->task-0160. Concrete range IS carried verbatim so a backend re-emits for v in lo..hi exactly with the const bound.

GOTCHAS / decisions: (1) Event needed a manual recursive Hash (Range not Hash + Vec<Event> recursion) — mirrors IterTile/FireBinding. (2) A worker contributing nothing to a loop body gets NO Loop (not an empty-bodied one) — matches old unroll observable behaviour for silent workers; tested. (3) A degenerate range (5..5) STILL emits a Loop carrying the empty range (faithful to source for; backend yields zero replays) — this DIFFERS from old behaviour which emitted zero events and lost the loop. (4) Trailing partial tile = TWO SIBLING Event::Loops with different ranges (falls out of mirroring Sequence/Repeat; NOT one parameterised loop) — pinned by blocked_stencil_trailing_partial_tile_is_two_sibling_loops. (5) e2e binding-reconstruction + Push/Wait-pairing tests had to recurse into Loop bodies (flat top-level walk now only sees the Loop wrapper) — flipped, coverage kept.

ORCHESTRATOR REVIEW GATE (phase3-ralph): qa-test-runner GO + mped-architect GO (both read-only). Numbers RE-RUN by reviewers this cycle: just test 345 passed/0 failed/1 ignored (compiler 33/0; flipped repeat_preserves_structure_in_event_list + new Loop/blocked-stencil/empty-range/idle-worker tests all pass); just e2e UNCHANGED total 10/pass 8/fail 0/skip 2/required-fail 0 (no codegen consumer of EventList -> cannot regress, confirmed); determinism-check 8/0 byte-identical; determinism-check-negative bites 2/2 non-flaky; clippy clean; acfg_to_petri.rs change confirmed DOC-ONLY (Net/unroll path untouched -> AC#3 satisfied by non-modification); new path determinism verified. Architect: Event::Loop design sound, manual recursive Hash correct+consistent with IterTile/FireBinding precedent (Eq/Hash agree), Net/EventList decoupling sound and triple-documented against re-unification, AC#2 deferral honest (genuine eval_const fold at acfg.rs ~694; not AC-gaming), forward-carry to TASK-0124/0160 accurate+actionable. ORCHESTRATOR HARDENING (4 architect doc-nits fixed in-thread, commit pending): event.rs Sync/Hash rationale destaled; event.rs wire-format doc now shows Loop shape; petri_to_events.rs line-77 stale "unrolled" clause corrected; test repeat_empty_range_emits_no_loop renamed -> repeat_empty_range_emits_loop_with_empty_range (it asserts a Loop IS emitted). Re-verified: event 33/0, petri_to_events 15/0, clippy clean. AC#1/#3/#4 met+verified; AC#2 honestly unchecked (dep task-0160); status In Progress — correct.

Forward-carried from TASK-0160 (commit 4a79d6e): TASK-0159 AC#2 (symbolic/const loop bound survives to the contract; backend can render (16_i64 - 1_i64) without AlgoIR) is now SATISFIABLE. TASK-0160 landed the NameSidecar with loop_bounds: BTreeMap<IterVar,LoopBound{lo,hi:IrExpr}> keyed by the SAME IterVar Event::Loop carries — the unevaluated `for y:1..H-1` bounds are captured additively at the build_acfg boundary (eval_const fold UNTOUCHED; ACFGNode::Repeat.range stays concrete; Net/boundedness/deadlock untouched). Proven by tests/petri_to_events.rs::sidecar_renders_stencil_symbolic_loop_bound_in_source_form: from Event::Loop.iter_var + sidecar.loop_bounds + sidecar.consts ALONE a backend reconstructs `(1_i64)..((16_i64 - 1_i64))` exactly (== pthreads-sync render_const_expr output).

NOTE the design chose a SIDECAR rather than an optional symbolic field on Event::Loop (consistent with the prefer-sidecar steer: bounds are per-program schedule metadata like name_data, Event::Loop stays lean and still carries the concrete range for the analysis-aligned consumers). So TASK-0159 Event::Loop itself is unchanged; AC#2 is met by the COMBINATION Event::Loop.iter_var (TASK-0159) + sidecar.loop_bounds (TASK-0160). I (TASK-0160 implementer) am deliberately NOT checking TASK-0159 AC#2 — the TASK-0159 owner should verify against this and decide.

ORCHESTRATOR RECONCILIATION (post TASK-0160 + TASK-0124): AC#2 ("symbolic/const loop-bound expr survives; backend renders (16_i64 - 1_i64) without AlgoIR") is now GENUINELY MET — TASK-0160 captured the unevaluated bound in NameSidecar.loop_bounds; TASK-0124 consumes it and renders the source-form bound (emit.rs golden test asserts the (16_i64 - 1_i64) / 256_i64 spelling and passes; qa-test-runner re-verified in the TASK-0124 gate). Verified by the TASK-0160 + TASK-0124 review gates, not self-cert. Closing TASK-0159 Done.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Structure-preserving EventList loop nest (commit ee309ff). 3 of 4 ACs met; AC#2 honestly deferred to TASK-0160 (dependency added) — task stays In Progress.

WHAT CHANGED
- event.rs: new Event::Loop { iter_var: IterVar, range: Range<i64>, body: Vec<Event> } (option (a)), mirrors ACFGNode::Repeat one-for-one. Event gets a manual recursive Hash (Range<i64> not Hash + Vec<Event> recursion; mirrors IterTile/FireBinding). Event::loop_over constructor.
- petri_to_events.rs: Repeat arm no longer unrolls — projects body once into a scratch per-worker BTreeMap, wraps each worker slice in one Event::Loop; nested Repeat -> nested Loop. Deterministic (BTreeMap WorkerId order). Module docs corrected.
- acfg_to_petri.rs: module doc corrected — analysis Net STILL unrolls (deliberate, decoupled); the stale "matches what EventList needs" claim removed. No code change to the Net path.
- tests: repeat_unrolls_in_event_list flipped to repeat_preserves_structure_in_event_list (+ empty-range, silent-worker, blocked-stencil sibling-loop tests); e2e binding/Push-Wait tests recurse into Loop bodies; 4 event.rs Loop unit tests.

USER IMPACT: the EventList is now a faithful rolled-loop codegen contract — unblocks the loop-structure half of TASK-0124. Trailing partial tile = two sibling Event::Loops with different ranges (forward-carried from TASK-0142).

GATE (actual): just test 33/0; e2e 10 pass 8 fail 0 skip 2 byte-identical; determinism-check 10/8/0/2 byte-identical; determinism-check-negative bites; clippy -D warnings clean.

HONEST LIMITATION (AC#2): loop bound is a CONCRETE Range<i64>; build_acfg folds H-1 -> 15 before this layer (acfg.rs ~694-697). Symbolic bound requires TASK-0160 (not-fold at lowering / const sidecar). Dep task-0159->task-0160 filed; AC#2 unchecked; status remains In Progress until TASK-0160 lands and TASK-0124 can re-render (16_i64 - 1_i64).
<!-- SECTION:FINAL_SUMMARY:END -->
