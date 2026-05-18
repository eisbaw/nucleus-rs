---
id: TASK-0150
title: 'transfer_inject: precise loop-invariance via AlgoIR index expressions'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 05:45'
updated_date: '2026-05-18 14:07'
labels:
  - followup
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0143 hoists Waits structurally: any data not produced inside the intra-tile body is treated as invariant w.r.t. the inner iteration. This is conservative; for a stencil pattern such as img_in[y][x] read inside a y-blocked inner loop, the access depends on y so the data is NOT actually y-invariant — yet the structural rule hoists anyway because the producer (load_image) lives above the tile loop. This is fine for correctness (the full data ranges arrive once per tile and the consumer reads the right slice within the tile) but it means the hoisted Push carries one *full* tile of data per outer-tile iteration even when only a halo strip is actually needed. To get tighter per-tile transfers (halo synthesis) the pass needs access-pattern info: re-plumb IndexExpr through DataflowEdge so transfer_inject can synthesise the actual y-band slice. Coupled with TASK-0117 (distributed placements) since halo only matters when the data is partitioned.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 DataflowEdge carries per-firing index/access expressions (DataId + ordered IrExpr indices) for every input read and the output write, in argument/LHS order; existing bare data_in/data_out fields stay so existing consumers are untouched
- [x] #2 build_acfg populates the new access info from AlgoIR (Dataflow + Effect statements), preserving argument order and duplicate reads (e.g. stencil img[y-1][x] and img[y+1][x])
- [x] #3 IrExpr/IrBinOp/IndexedRef gain serde derives (same gated pattern as ACFG/event types) so the enriched DataflowEdge still serialises under the default serde feature
- [x] #4 transfer_inject's whole-symbol/structural-invariance behaviour is unchanged (this task only plumbs data; the precise per-tile halo-synthesis CONSUMER is explicitly deferred to a follow-up coupled with TASK-0117 distributed placement) — documented honestly in code + notes
- [x] #5 Determinism preserved (no HashMap iteration introduced) and bit-identical e2e for examples 01/02/03/05/07 (7 pass / 0 fail / 3 skipped); just test, just e2e, just determinism-check, clippy -D warnings all green
- [x] #6 Unit tests assert build_acfg emits the access expressions for a representative read (indexed stencil-like and scalar) and that argument order + duplicates are preserved
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add serde derives (gated on feature="serde", matching event.rs pattern) to IrExpr, IrBinOp, IndexedRef in algo/ir.rs.
2. Add a DataAccess struct to acfg.rs: { data: DataId, indices: Vec<IrExpr> } with serde gating. Add data_in_access: Vec<DataAccess> and data_out_access: Option<DataAccess> fields to DataflowEdge (additive; keep data_in/data_out).
3. In build_acfg (build_dataflow + build_effect), populate the access vectors by walking the call args / LHS IndexedRef in order, recovering DataId+indices. Keep collect_dataref_names for the legacy bare lists so existing passes are untouched.
4. Update block_transform.rs synthetic DataflowEdge construction to fill the new fields (empty/derived) so it still compiles.
5. Add unit tests in tests/acfg.rs: indexed stencil-like read keeps per-arg index exprs in order incl. duplicates; scalar/whole-array read has empty indices; effect-stmt args captured; data_out_access set for dataflow, None for effect.
6. Document in acfg.rs module docs + transfer_inject.rs that indices are now AVAILABLE but the precise per-tile halo CONSUMER is deferred (file follow-up task coupled to TASK-0117).
7. Run just test / e2e / determinism-check / clippy. Verify 7/0/3 and bit-identical.
8. Commit compiler(M2): plumb AlgoIR index expressions through DataflowEdge (TASK-0150).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Context from TASK-0136/0151/0153: the current whole-symbol cross-scope model uses STRUCTURAL loop-invariance (data not produced inside the loop body) instead of precise AlgoIR index analysis. This is conservatively SAFE, not a bug: it can only OVER-serialise (treat an index-varying access as a whole-symbol crossing, transferring more/serialising more than strictly necessary) — it never under-synchronises and never collapses a genuinely per-iteration transfer into one. TASK-0150 is the precision upgrade (per-firing index expressions through ACFG so e.g. disjoint a[i] slices can pipeline); until it lands, the over-serialisation is a performance ceiling, not a correctness defect. Also relevant: TASK-0151 over-approximation (block-entangled non-block transfers stranded) and the single-assignment debug-assert added in TASK-0153.

Implemented (commit on master). Added DataAccess struct + data_in_access/data_out_access on DataflowEdge (additive; data_in derived from access list = single source of truth). serde derives added to IrExpr/IrBinOp/IndexedRef gated on feature="serde". build_acfg populates from AlgoIR for both Dataflow and Effect statements; LHS indices captured in data_out_access. DataflowEdge::new constructor added for synthetic/test callers (empty indices) — converted all 8 test construction sites to it. Consumer (halo synthesis) deliberately NOT done; filed TASK-0158 coupled to TASK-0117 and documented in acfg.rs + transfer_inject.rs docs. NOTE/honest limitation: pre-existing clippy::len_zero warnings exist in tests/acfg_to_petri.rs (len()>0 on net.transitions) — NOT introduced by this task and NOT flagged by the workflow clippy command (cargo clippy --workspace -- -D warnings, no --tests); left untouched as out of scope. Verified: just test all green; just e2e 7 pass/0 fail/3 skipped bit-identical; just determinism-check 7/0/3 byte-identical; required clippy clean.

Post-review hardening (mped-architect P1, blocking): the struct doc claimed the cross-field invariant was "asserted in build_acfg" but NO assertion existed (comment lied about the code; TASK-0150 was Done on that overclaim). Added DataflowEdge::debug_check() asserting #1 data_in==data_in_access projected and #2 data_out_access.data==data_out; called from new() and both build_acfg sites (build_dataflow/build_effect). Also corrected the doc: the args<->data_in_access relationship is NOT asserted (a k(a+b) arithmetic-of-data arg is one Scalar while data_in_access still flattens a,b — documented limitation owned by TASK-0158, not an invariant break). Softened build_arg_bindings doc to "total for link-valid IR" (it contains a guard panic on undeclared DataRef). Verified: full test green (assertions hold across all examples), e2e 7/0/3 bit-identical, determinism 7/0/3, clippy clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Plumbed AlgoIR per-firing index expressions through the ACFG DataflowEdge so downstream passes can see the real access pattern (e.g. img_in[y-1][x+1]), not just bare DataId sets.

Changes:
- acfg.rs: new DataAccess { data: DataId, indices: Vec<IrExpr> }; DataflowEdge gains data_in_access (parallel to data_in) and data_out_access (parallel to data_out). data_in/data_out are now derived from the access lists (single source of truth). Added DataflowEdge::new for index-less synthetic callers. build_acfg populates real indices for Dataflow (args + LHS) and Effect statements, preserving argument order and duplicate reads.
- algo/ir.rs: serde derives on IrExpr/IrBinOp/IndexedRef (gated on feature="serde", matching event/ACFG pattern) so the enriched edge serialises.
- lib.rs exports DataAccess. block_transform + 8 test construction sites converted to DataflowEdge::new.
- Module docs (acfg.rs, transfer_inject.rs) updated: data is now AVAILABLE; the precise per-tile halo-synthesis CONSUMER is explicitly deferred to TASK-0158, coupled to TASK-0117 (halo only matters under distributed placement). transfer_inject behaviour is unchanged (structural hoist, conservatively safe).

User impact: enables TASK-0156 (per-Fire value bindings) and TASK-0158 (precise halo). No behavioural change to existing schedules.

Tests: 3 new acfg.rs tests (stencil indices verbatim/in-order/with duplicates; access/bare alignment invariant across 01/02/03/05/07; whole-array & effect reads have empty indices). just test all green; just e2e 7 pass/0 fail/3 skipped bit-identical; just determinism-check 7/0/3 byte-identical; clippy (workspace -D warnings) clean.

Follow-ups filed: TASK-0158 (halo-synthesis consumer of the new index data, coupled to TASK-0117).

Honest limitation: pre-existing clippy::len_zero in tests/acfg_to_petri.rs (unrelated, not flagged by the required clippy command) left untouched as out of scope.
<!-- SECTION:FINAL_SUMMARY:END -->
