---
id: TASK-0373
title: >-
  Distributed gather: opaque-index transfer + whole-array broadcast
  (TASK-0341.03.01 broaden)
status: To Do
assignee: []
created_date: '2026-05-30 22:46'
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
