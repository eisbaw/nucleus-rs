---
id: TASK-0373
title: >-
  Distributed gather: opaque-index transfer + whole-array broadcast
  (TASK-0341.03.01 broaden)
status: To Do
assignee: []
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 01:10'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0375 (2026-05-31): VERIFIED that collect_dataref_access_expr (acfg/build.rs:427-453) matches IrExpr::DataRef and pushes ONLY the OUTER array into data_in_access — it does NOT recurse into the indices vector. So for a gather x[col[k]] the inner index array col is currently NOT added to the firing data_in_access (hence not to the derived data_in). This is inert for single-worker (codegen emits from AlgoIR, ignoring ACFG dataflow edges — see memory project-backend-emits-from-algoir-not-acfg), but a CORRECT DISTRIBUTED gather MUST get col into data_in so the conservative whole-array broadcast of col reaches every worker. TASK-0373 must decide: recurse into index DataRefs here (perturbs data_in for shipped programs — gate carefully), or inject the broadcast at the transfer layer. The acfg/build.rs docstring was corrected under TASK-0375 to state this non-collection honestly and points here.
<!-- SECTION:NOTES:END -->
