---
id: TASK-0373
title: >-
  Distributed gather: opaque-index transfer + whole-array broadcast
  (TASK-0341.03.01 broaden)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 14:03'
labels:
  - compiler
  - gather
  - transfer_inject
  - distributed
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BROADEN follow-up to TASK-0341.03.01 (native single-worker gather landed). To support a DISTRIBUTED gather (partition over a kernel whose arg is x[col_idx[i][k]]) the conservative path must broadcast the WHOLE gathered array x to every worker. Architect P2.1 (gather review): transfer_inject::collect_ivs_from_expr descends into the inner DataRef of a gather index and mis-attributes the inner ivs (k from col_idx[i][k]) to the OUTER array x dim 0 -> compute_partition_bounds_with_dim_prefix would emit a WRONG slice band for x. INERT TODAY: halo_inference DataDependentStride is fatal-under-partition and fires before inject_transfers, so a partitioned gather is fail-loud rejected (verified). To LIFT that rejection into a working distributed gather: (1) mark a dim OPAQUE when its index contains a DataRef (do not attribute inner ivs); (2) compute_partition_bounds_with_dim_prefix returns whole-array for an opaque dim; (3) relax the halo fatal-under-partition gate to whole-array-broadcast for the opaque case; (4) add a distributed 17-spmv/gather e2e cell bit-identical across backends. Carries a pinning negative test for the mis-attribution.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 transfer_inject: an index dim whose index expression contains a DataRef/Call (data-dependent gather index) is recorded OPAQUE (empty iv set) by collect_data_dim_iv_map/record_access_per_dim, so compute_partition_bounds_with_dim_prefix returns whole-array for the gathered outer array; do NOT attribute the inner ivs (i,k from col_idx[i][k]) to the outer arrays dim 0
- [x] #2 A pinning negative unit test asserts that for x[col_idx[i][k]] under partition=workers(i), x dim-0 is NOT i-attributed (whole-array), demonstrating the mis-attribution fix
- [x] #3 halo_inference: the DataDependentStride fatal-under-partition gate is relaxed to advisory (whole-array-broadcast) for the data-dependent READ/gather case that whole-array broadcast correctly serves; it MUST remain fatal for any case NOT served by whole-array broadcast (data-dependent WRITE/scatter stays rejected, TASK-0384). Unit tests pin both arms
- [x] #4 The gather index array (col_idx) reaches every compute worker (via data_in broadcast or i-band). The decision (recurse in acfg collect_dataref_access_expr vs transfer-layer inject) is recorded with rationale; existing shipped programs data_in is unperturbed (verified by e2e byte-identity on all existing cells)
- [x] #5 New schedule schedules/distributed_gather.sched.nuc for prog.gather.algo.nuc (partition=workers on i; x whole-array broadcast; val/col_idx/y i-banded) + e2e-matrix.toml cells; the gather-distributed cell is byte-identical to reference.bin AND bit-identical cross-backend on every backend whose capability admits it
- [x] #6 Gate green: build/clippy/test/test-release all 0-fail (modulo the known TASK-0291 release should_panic divergence); e2e existing baseline 350/293/0/57/0 preserved byte-identical, new cells added on top; all numbers re-run not transcribed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
STEP A (verify current rejection): create distributed_gather.sched.nuc cloned from masked distributed.sched.nuc but pointing at prog.gather.algo.nuc; confirm fail-loud rejection via halo DataDependentStride.
STEP B (transfer_inject opacity, AC#1/#2): in record_access_per_dim, when an index dim expr contains a DataRef/Call mark dim OPAQUE (empty iv set) rather than descending into nested ivs. Add pinning unit test that x dim-0 is NOT i-attributed (whole-array) for x[col_idx[i][k]] under partition=workers(i). Update collect_ivs_from_expr DataRef/Call descent docstring to match.
STEP C (halo relax, AC#3): relax error_is_fatal_under_partition DataDependentStride arm to advisory for the READ/gather case served by whole-array broadcast; KEEP fatal for anything not served (scatter is on LHS and NOT seen by halo visit_arg, so it stays rejected by its own path — verify). Unit tests pin both arms.
STEP D (col_idx into data_in, AC#4): decide recurse-in-acfg vs transfer-layer; record rationale. col_idx is iv-affine [i][k] so once in data_in axis-filter i-bands it. Verify shipped data_in unperturbed via e2e byte-identity.
STEP E (schedule + matrix, AC#5): distributed_gather.sched.nuc + 7 e2e cells; byte-identical to reference.bin + cross-backend.
STEP F (gate, AC#6): build/clippy/test/test-release/e2e; preserve baseline 350/293/0/57/0; report actual numbers.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0375 (2026-05-31): VERIFIED that collect_dataref_access_expr (acfg/build.rs:427-453) matches IrExpr::DataRef and pushes ONLY the OUTER array into data_in_access — it does NOT recurse into the indices vector. So for a gather x[col[k]] the inner index array col is currently NOT added to the firing data_in_access (hence not to the derived data_in). This is inert for single-worker (codegen emits from AlgoIR, ignoring ACFG dataflow edges — see memory project-backend-emits-from-algoir-not-acfg), but a CORRECT DISTRIBUTED gather MUST get col into data_in so the conservative whole-array broadcast of col reaches every worker. TASK-0373 must decide: recurse into index DataRefs here (perturbs data_in for shipped programs — gate carefully), or inject the broadcast at the transfer layer. The acfg/build.rs docstring was corrected under TASK-0375 to state this non-collection honestly and points here.

IMPLEMENTED (commits b121365 + 14cc5b8). Design decision on col_idx-into-data_in: RECURSE in acfg/build.rs collect_dataref_access_expr (not transfer-layer inject). Rationale: (1) data_in is the single source of truth for what each worker needs; the existing transfer machinery (axis-mapping i-band filter + whole-array opaque fallback) is fully data-driven from data_in_access, so once col_idx is in data_in it i-bands for free — a transfer-layer inject would duplicate that logic. (2) The already-existing value-flow walker walk_dataref_names ALREADY recurses into index DataRefs; collect_dataref_access_expr was the OUTLIER. Recursing brings the two into alignment (CLOSES a silent-sibling divergence rather than creating one). (3) NO-OP for non-gather programs (no nested index DataRef), so data_in unperturbed — verified by e2e byte-identity on all existing cells.

GOTCHA 1 (scatter accidentally admitted): the first halo relaxation made DataDependentStride ALWAYS advisory. That ACCIDENTALLY ADMITTED a distributed SCATTER (08-histogram prog.scatter.algo.nuc) because the scatter RHS histogram[input[i]] is a data-dependent READ that the halo pass DOES walk (only the LHS write index is invisible to halo). It even emitted byte-identical output (the TASK-0343 accumulator combine handles it). Per task constraint scatter MUST stay rejected (TASK-0384). FIX: added is_scatter_rmw flag to DataDependentStride, threaded from the LHS-write-is-data-dependent bit (lhs.indices.any(expr_contains_dataref_or_call)) down CallSite->IndexSite. Gather (affine LHS) advisory; scatter RMW (data-dependent LHS) stays FATAL under partition. 3 halo unit tests pin both arms + the unpartitioned-scatter boundary.

GOTCHA 2 (FIFO send/recv order, the bufsync/poll FAIL/run): col_idx-into-data_in initially recursed OUTER-FIRST (x then col_idx). Host Push order follows producer/declaration order (val, col_idx, x); worker Wait order follows data_in order. With outer-first, worker waited x-then-col_idx while host sent col_idx-then-x. Event/shared-mem backends (5 of 7) demux per-seq and tolerated it; strict-FIFO mp-tcp-bufsync + mp-tcp-poll use read_msg_expect and FAILED with tag mismatch. FIX: recurse index-FIRST in collect_dataref_access_expr so col_idx precedes x in data_in, matching the host send order. This is the documented project-mp-tcp-event-vs-bufsync-safety-profile asymmetry. All 7 now byte-identical.

GOTCHA 3 (single-worker unaffected): single-worker gather (17-spmv/gather, all 7) emits from AlgoIR and ignores ACFG dataflow edges (memory project-backend-emits-from-algoir-not-acfg), so none of this machinery touches it — re-verified still PASS.

REJECTED approach: NOT a sticky-IterVar-sentinel for opacity (BTreeSet<IterVar> cannot hold a sentinel cleanly); instead opacity = empty iv set + a separate sticky opaque_dims BTreeMap<DataId,BTreeSet<usize>> threaded through the walk, so a sibling affine access cannot un-opaque a gather dim.

GATE: build/clippy/test(dev all ok)/test-release(rc=0)/e2e 357/300/0/57/0 (baseline 350/293 preserved byte-identical, +7 gather-distributed all PASS). check-textual-replace OK, check-include-str-coverage OK, check-mega-files OK (transfer_inject/halo_inference pre-existing allow-list).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Distributed gather x[col_idx[i][k]] under partition=workers(i) now lowers + codegen + runs byte-identical to reference.bin on ALL 7 tier-1 backends (was fail-loud rejected). Commits: b121365 (compiler machinery + 5 unit tests), 14cc5b8 (schedule + 7 e2e cells + FIFO order fix), 6a4ed0c (doc-lie fix).

Mechanism: (1) transfer_inject::record_access_per_dim marks a data-dependent index dim OPAQUE (empty, sticky iv set) so the gathered outer array x falls to whole-array broadcast instead of being mis-attributed inner ivs. (2) acfg::collect_dataref_access_expr recurses INDEX-FIRST into nested DataRefs so the gather index array col_idx reaches data_in (i-bands like val) AND the worker Wait order matches the host Push order on strict-FIFO backends. (3) halo_inference DataDependentStride relaxed to a gather/scatter split via is_scatter_rmw: pure gather (affine LHS) advisory, scatter RMW (data-dependent LHS) STAYS fatal under partition (TASK-0384 unaffected, empirically reconfirmed rejected). (4) expr_contains_dataref_or_call lifted to passes::common as the single shared data-dependence predicate.

Gate (all re-run, not transcribed): just ci rc=0. build OK; clippy 0; test (dev) all ok 0-fail; test-release rc=0; e2e POSITIVE arm 357/300/0/57/0 (baseline 350/293/0/57/0 preserved byte-identical; +7 gather-distributed all PASS). check-textual-replace OK, check-include-str OK, check-mega-files OK. compiler lib tests 160->165 (+5 TASK-0373 pins).

LIMITS / honest caveats: (a) The opacity stickiness (affine sibling cannot un-opaque a gather dim) is tested but no SHIPPED program exercises the mixed affine+gather-same-dim case — defensive. (b) The scatter-still-rejected guarantee rests on the is_scatter_rmw flag = LHS-write-is-data-dependent; a firing that gathers one symbol AND scatters a DIFFERENT symbol would also be kept fatal (conservative, correct). (c) col_idx-into-data_in recursion is a NO-OP for non-gather programs (verified by e2e byte-identity) but DOES change data_in ORDER for gathers (index-first) — intentional for FIFO consistency, documented. Forward-carried the scatter findings to TASK-0384.
<!-- SECTION:FINAL_SUMMARY:END -->
