---
id: TASK-0265
title: 'M5 Stage 2: backend walker emits delay-line / circular buffer for reuse_widths'
status: To Do
assignee: []
created_date: '2026-05-24 02:33'
updated_date: '2026-05-24 02:43'
labels:
  - M5
  - compiler
  - codegen
  - reuse
  - stage-2
dependencies:
  - TASK-0261
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Stage 2 of the TASK-0261 reuse loop-option family. Stage 1 (TASK-0261, cycle 82) landed reuse inference + sidecar persistence. This task wires the BACKEND WALKER (multi_worker_walker + each per-backend Plan) as the consumer.

## Acceptance criteria
1. The backend walker reads reuse_widths from the NameSidecar at the Event::Loop emit site.
2. For each slot at (iter_var, data_id, axis): emit a circular-buffer declaration of length L at loop entry, an initial-fill prologue if min_offset < 0, and per-iteration update logic that loads the most-distant element once and indexes the others via buf[(iter_var + b - min_offset) % L].
3. Every grid[iv + b] DataRef read inside the loop body is rewritten to buf[(iv + b - min_offset) % L] where the (data, axis) matches a slot.
4. A new e2e cell on an example with 'loop V : reuse;' in the schedule shows bit-identical output to the non-reuse baseline AND has a measurably smaller emitted Vec capacity for the reused symbol (or some other observable: emitted code contains 'circular' / 'delay_line' / similar marker).
5. The Stage 1 driver call site (currently apply_reuse_inference_advisory swallowing all errors) MUST evolve: either switch to the strict variant once every shipped example is affine-only, OR check the errors vec against whether any reuse directive demanded the slot.

## Coordination with TASK-0263 (halo Stage 2)
Both halo Stage 2 and reuse Stage 2 touch the SAME backend walker emit site (multi_worker_walker + per-backend Plan at Event::Loop). They are ORTHOGONAL feature toggles:
- Halo widens per-tile transfer ranges (interacts with partition shape).
- Reuse rewrites read patterns INSIDE a tile (independent of partition shape).
A loop that carries BOTH a halo entry AND a reuse entry needs both code paths active simultaneously. Recommend scheduling TASK-0263 first (halo has a real consumer + e2e cell forecast in TASK-0260 cycle 81), then this task; the per-feature integration in each backend's Plan is independent.

## Forward-carry from TASK-0261 cycle 82
- The 'one walk per reuse iv' Stage 1 shape carries forward: at Event::Loop emit, look up reuse_widths.get(iter_var) and iterate the per-(DataId, axis) slots independently. Multiple slots on the same loop combine via separate delay-line variables; no cross-slot interaction (cf. nested_reuse_outer_inner_independent_accumulators test).
- The advisory driver wire emits nuc_trace! lines under NUC_TRACE=1; Stage 2 either promotes these to fatal Err or keeps the lenient stance with a partition-aware policy check.
- The Stage 1 sidecar shape is BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>> with ReuseSlot { min_offset: i64, length: u64 }. The codegen rewrite is buf[(iv + b - min_offset) % length].

## Honest scope
- Performance NOT a Stage 2 acceptance criterion — only correctness + bit-identity. Quantified perf is M6+ scope (PRD §11).
- The actual circular-buffer Rust template is per-backend (pthreads-sync vs pthreads-async vs mp-tcp-* will share most of it but the worker-init prologue differs). Plan structure choice — shared helper in backend-common vs per-backend duplication — to be determined when the work starts.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0261 cycle-82 review (architect 5 items):

When Stage 2 wiring lands (backend walker delay-line codegen):

1. **Promote driver from lenient apply_reuse_inference_advisory to strict.** Once codegen reads reuse_widths, a silently-swallowed typed error becomes wrong output. Pick partition-policy-aware fatality (same pattern Stage 2 of TASK-0260 needs to apply for halo).

2. **Variant rename for cross-pass consistency**: reuse_inference uses ReuseInferenceError::UnknownLoopVar { var }; halo_inference uses HaloInferenceError::UnknownIterVarInScope { iter_var }. Stage 2 may want one shared passes::common::IvScopeError when both pass diagnostics surface in the same driver pass. Consider rename for parity at the lift cycle.

3. **Sidecar serde round-trip golden test**: reuse_widths is a TRIPLE-NESTED BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>>. Non-trivial to deserialise from JSON. Add a serde round-trip golden test pinning the JSON shape BEFORE Stage 2 codegen consumes the format.

4. **Defensive variants UnknownLoopVar + UnknownDataInRef have NO direct test** — only 6 of 8 ReuseInferenceError variants pinned. Cross-module invariant guards merit one test each (parity with halo's UnknownIterVarInScope, also untested as a defensive belt).

5. **Cosmetic normalisation**: tests/partition_workers.rs:566 has bare 'reuse_widths: BTreeMap::new()' while other fixtures use fully-qualified 'std::collections::BTreeMap::new()'. One-line normalisation when Stage 2 touches the test crate.
<!-- SECTION:NOTES:END -->
