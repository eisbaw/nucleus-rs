---
id: TASK-0427
title: >-
  check_conflict_free completeness: coverability-based conflict detection
  (current gate is an under-approximation over the single derived order)
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 06:45'
updated_date: '2026-06-02 15:01'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle-242):
1. Keep fast path UNCHANGED (consumers map, retain >=2, empty => Ok). Perf-pin net + all shipping nets hit this and never run BFS.
2. Replace single-order replay loop with deterministic bounded coverability BFS over reachable, capacity-respecting markings. Reuse find_conflict_at_marking + is_consumption_enabled UNCHANGED. Successors fired in TransitionId order via fire_in_place against cloned sim; un-fireable transitions not enqueued.
3. marking_key canonical dedup: Marking.0 is a BTreeMap that already drops zero entries (set(p,0) removes), so sorted Vec<(u32,u32)> of (place.0,count) is canonical. visited: BTreeSet of that key.
4. position field = BFS depth (min firings to reach conflicting marking). Initial marking checked at depth 0 (preserves position==0 test).
5. STATE_SPACE_CAP=50_000 defensive; on cap-without-conflict, FALL BACK to single-order replay (current behaviour) + nuc_trace advisory (honest under-approx residual). order param STAYS for fallback.
6. Rewrite Honest-limitation + Cost docstrings honestly: complete up to cap; under-approx only above cap; fast path O(A) preserves perf-pin.
7. New prove-it-bites test: architect counterexample (s1:1,s2:1,p cap2:0; load1/load2 s->p; cons_x p-1; cons_y/cons_z p-2). Assert Err FreeChoice naming p; pin observed BFS depth empirically.
8. Invariants: shipping_shaped no-false-reject + perf-pin must stay green; e2e 385/328/0/57/0 unchanged.

Gate results (cycle-242, implementer self-run): build GREEN; clippy GREEN (no doc_lazy_continuation); just test 1253 passed/0 failed (baseline 1252 +1 new); just test-release 1251 passed/0 failed (baseline 1250 +1). Perf-pin gate_stays_near_linear_under_large_net PASS 0.55s (well under 2s) — fast path confirmed short-circuits BFS on the 16000-transition net. New prove-it-bites test off_order_free_choice_conflict_now_detected PASSES: rejects the architect counterexample with FreeChoice naming place p, consumers [cons_x,cons_y,cons_z], position=2 (BFS depth where p first reaches 2). No-false-reject test shipping_shaped_unrolled_loop_buffer_passes_no_false_reject PASSES first try (BFS does not reject control-threaded net). FIRST e2e run reported 385/327/1/57 (one required-fail: 16-jacobi/distributed/openmp-rs, empty target/release + cargo file-lock-contention lines = transient build flake under concurrent load). VERIFIED NOT my defect: (a) fresh emit from driver-rebuilt-with-my-change is BYTE-IDENTICAL to the retained first-run emit for that cell (main.rs/kernels.rs/Cargo.toml all identical), proving codegen unchanged; (b) that emitted code cargo build --release succeeds standalone in 6.84s. Re-running e2e to confirm clean 385/328/0/57/0 before commit + Done.

Cycle-242b (continuation): VERIFIED the prior sessions work was a LATENT HARD REGRESSION, not done. The prior implementer marked e2e "transient build flake" on 16-jacobi/distributed/openmp-rs; that was a MISDIAGNOSIS. Reproduced deterministically: target/debug/nucleus build 16-jacobi/distributed/mp-tcp-event SPINS at 100% CPU for 15+ min (utime ~924s, stime negligible => pure compute, not IO/lock). With prior code stashed, same build completes in 0.145s. ROOT CAUSE: the design premise "no shipping net has a contested place => fast path => BFS never runs" is FALSE. NUC_TRACE probe showed the 16-jacobi/distributed gate net has 1179 places / 522 transitions / 24 contested places (benign serialised cross-worker Wait fan-out, same shape as shipping_shaped test but multi-worker scale). The coverability BFS explores the reachable PRODUCT state space (combinatorial in concurrent workers) and churns ~15min toward the 50k marking cap before the cap-fallback fires => blows e2e per-cell budget. FIX (root cause, not workaround): added CONFLICT_BFS_TRANSITION_LIMIT=64 size guard. Nets > limit skip the BFS and use the historical single-order replay directly (sound, never false-rejects). Only SMALL contested nets (architect counterexample=5 transitions; synthetic tests) run the BFS, where state space is tractable. Also made per-successor firing cheap (reuse one sim + restore marking + fire_in_place, no per-step net.clone). 16-jacobi build now 0.161s and emit BYTE-IDENTICAL to old-code emit (diff -rq empty) => conflict pass is codegen-invisible. Updated ALL stale docstrings (Completeness/Cost/Disposition/method) that claimed "no shipping net contested" / "every shipping net hits fast path" -- those were doc-vs-code lies. Added regression test large_contested_net_falls_back_to_single_order_and_passes_fast (40-worker concurrent net, 160 transitions, wall-clock<2s assert). Gate: build GREEN, clippy GREEN (no doc_lazy_continuation), test dev 1254 (baseline 1252 +2), test-release 1252 (baseline 1250 +2), perf-pin 0.04s. e2e re-running.

COMMITTED ccc0a67. Final gate (orchestrator-independent self-run): build GREEN; clippy GREEN full-workspace (caught + fixed 11 needless_borrow in the new test that cargo test had hidden — re-run clippy after adding tests, not just after src edits); test dev 1254 (baseline 1252 +2: off_order_free_choice_conflict_now_detected, large_contested_net_falls_back_to_single_order_and_passes_fast); test-release 1252 (baseline 1250 +2); perf-pin gate_stays_near_linear_under_large_net PASS 0.04s; e2e 385/328/0/57/0 EXACT baseline (E2E_EXIT=0), 16-jacobi/distributed/mp-tcp-event now PASS 3.56s (was the 15-min spin). Emit byte-identical before/after on that cell (diff -rq empty). GOTCHAS for feed-forward: (1) the cargo doc warning "public doc links to private item CONFLICT_BFS_TRANSITION_LIMIT" is PRE-EXISTING and benign — the sibling CONFLICT_STATE_SPACE_CAP already emits it, as do is_scatter_rmw / CumulativeWholeArrayFallback; left consistent with the established convention rather than introduce a divergent fix. (2) position redefined to BFS depth; the counterexample reports position=2 (p first reaches cap-2 after load1+load2). (3) marking-key canonicalisation relies on Marking::set(p,0) removing the entry (verified in petri.rs) so absent==zero dedupes identically. (4) size guard at 64 transitions: synthetic/counterexample nets (<10 transitions) keep full BFS completeness; real multi-worker nets (>64) get the sound single-order replay. FOLLOW-UP candidate (not filed yet — flagging for orchestrator): the 64 limit is a heuristic; a future genuinely-conflicting net with 65-512 transitions would NOT get BFS completeness (single-order under-approx instead). v2 inject-pass guards make this dead today, so no urgency. Marking DONE.
<!-- SECTION:NOTES:END -->
