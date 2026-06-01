---
id: TASK-0417
title: >-
  Audit: silent statement/edge-drop sweep of nucleus-compiler None/skip arms
  (TASK-0360 follow-on)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 22:58'
updated_date: '2026-06-01 23:07'
labels:
  - hardening
  - audit
  - silent-drop
  - prove-the-check-bites
  - cycle-239
dependencies:
  - TASK-0360
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-239 hardening, motivated by TASK-0360 which found build_dataflow silently dropped a bare-LValue dataflow statement (a same-worker copy compiled to nothing). Per memory feedback-option-none-skip-arm-silent-drop, audit every other None/skip arm in nucleus-compiler for the SAME class (a per-statement/per-edge/per-node transformer whose None silently drops semantic content with no diagnostic).

SCOPE: all 26 `=> None` / `return None;` / `Ok(None)` sites in nucleus-compiler/src (non-test), PLUS the other drop mechanisms (.retain / .filter / .filter_map that rebuild a node/event/stmt list, and `continue` in per-item walkers).

CONCLUSION (orchestrator, pending architect verification): build_dataflow was the ONLY silent statement-drop. All 26 None-arms classify as one of: (1) tree-search accessor [find_first_inner_repeat/first_repeat_in/find_outer_of_2d in partition_blocks2d/partition_rows/partition_workers/block_transform; call_callee; decompose_grid; error.rs did-you-mean; contract.rs ReturnType::Default] — None = not-found/no-value; (2) find_map/filter_map filter over a DERIVED set [sched/lower block/unroll/check-span; link/pipeline depth+buffer; transfer_inject/inject name->entity; sync_inject data_out] — None = item legitimately excluded; (3) DOCUMENTED conservative whole-array fallback [transfer_inject/tiles.rs + partition.rs compute_*_bounds] — None = over-approximate (never under-transfers), TASK-0301/0302 lineage; (4) LOUD drop [algo/lower lower_stmt_into / For-lowering] — None always accompanied by acc.record_stmt_error (TASK-0205 pattern), so program fails to compile; (5) test-only filter_map [host_data_relay_inject]. No .retain on node collections; no filter/filter_map rebuilds the ACFG node list or event list.

OUT OF SCOPE (filed separately): the BACKEND emit surface (backend-common multi_worker_walker + per-backend fire renderers) — a silent event-drop there is a different mechanism + much larger surface.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle-239 architect verification (read-only) — GO; conclusion holds ===

mped-architect independently re-traced all 5 buckets + the caller side for the risky ones (loud-drop, whole-array-fallback). Verdict: GO, no P1/P2. "build_dataflow was the only silent statement-drop in nucleus-compiler" is CORRECT as stated. Zero .retain in non-test src; no filter/filter_map rebuilds the ACFG node list or event list (sync_inject/ordering/halo_strip rebuilds are arity-preserving, only insert/reorder, debug_assert-pinned). acfg_to_petri walk is exhaustive (no catch-all); emit_xfer exhaustive over the 2-variant XferRole.

TWO P3 CORRECTIONS folded into this conclusion (the audit text was slightly off; honesty fixes):

P3-1 (my mislabel — feedback-comment-doc-lie on my own audit): tiles.rs cumulative_band_bounds None is NOT a "conservative whole-array fallback". It is a FAIL-LOUD guard: caller rewrite_cumulative_band_tiles returns Err(TransferInjectError::CumulativeWholeArrayFallback) when partition_ranges is non-empty (TASK-0366 cycle-214 anti-xN-double-count fix). Whole-array is kept silently ONLY when partition_ranges is empty (case B = single-worker cumulative symbol, genuinely correct). So this site is STRICTER than the audit claimed — verified safe, but for the right reason now.

P3-2 (completeness): the SCOPE line claimed "continue in per-item walkers" but the conclusion did not name the one such site: petri_to_events.rs:331 `if body_events.is_empty() { continue; }` drops a per-worker Event::Loop. CLASSIFIED BENIGN (intentional emission projection — petri_to_events.rs:304-308: a worker that does nothing in a loop gets no Loop, not an empty one; NOT a transformer dropping its own input content). But it is silent-by-design and is the one place a future build_dataflow-class regression could hide (relies on upstream host_data_relay_inject body population). Defense-in-depth debug_assert filed as TASK-0419 (with a mandatory invariant-verification precondition so it cannot false-fire on valid input).

NO CODE CHANGED this cycle (pure audit + classification). qa-test-runner N/A (no build/test delta); last verified gate stands (test 1243/0/3, test-release 1242/0/3, e2e 385/328/0/57/0 from the TASK-0360 cycle). The reviewable artifact was the classification, verified by the architect.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Audited all 26 None/skip arms in nucleus-compiler/src + .retain/.filter/.filter_map/continue drop mechanisms for the build_dataflow silent-statement-drop class. CONCLUSION (architect-verified GO, no P1/P2): build_dataflow (fixed by TASK-0360) was the ONLY silent statement-drop. All other sites are accessors / derived-set filters / fail-loud guards / loud error-recorded drops (algo/lower TASK-0205) / intentional benign projections (petri_to_events:331) / test-only. 2 P3 honesty corrections folded into notes (tiles.rs is a fail-loud guard not a whole-array fallback; petri_to_events:331 named explicitly). Follow-ups filed: TASK-0418 (backend emit silent-event-drop audit — out of scope here), TASK-0419 (petri_to_events:331 defense-in-depth debug_assert). No code changed; risk discharged.
<!-- SECTION:FINAL_SUMMARY:END -->
