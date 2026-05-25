---
id: TASK-0317
title: >-
  Silent-bypass guard for rewrite_partition_tiles_inner default-order fall-back
  (TASK-0315 silent-sibling follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 09:55'
labels:
  - compiler
  - hardening
  - transfer_inject
  - silent-sibling
  - forward-carried-from-TASK-0315
dependencies:
  - TASK-0315
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle 134 (TASK-0315) added a `nuc_trace!` diagnostic on the default-order fall-back branch of `order_halo_strip_bounds_by_data_dim` in `nucleus/nucleus-compiler/src/passes/transfer_inject.rs` to surface axis-mapping-defence bypass on synthetic fixtures.

While reviewing TASK-0315 for silent-sibling defects per `feedback-silent-sibling-defect`, the orchestrator identified a structurally-identical fall-back pattern at `rewrite_partition_tiles_inner` (transfer_inject.rs line ~1731):

```rust
let bounds = match compute_partition_bounds_with_dim_prefix(...) {
    Some(b) => b,
    None => {
        // Pre-TASK-0301 fall-back: iterate the partition_axis_order
        // ... synthetic fixtures + bare-aggregate-only data.
        ...
    }
};
```

Same risk class as TASK-0315: when `data_dim_iv_map` does not carry an entry for the data symbol (no observed indexed accesses), the code falls back to a nest-order emit that bypasses the TASK-0301 axis-mapping defence. Production callers always observe accesses; synthetic test fixtures built via `DataflowEdge::new` reach the fall-back silently.

## Acceptance criteria

1. Add a `crate::nuc_trace!(...)` log line on the `None` arm of the `compute_partition_bounds_with_dim_prefix` match in `rewrite_partition_tiles_inner` (transfer_inject.rs ~line 1731). Diagnostic should identify the function, the data id, the worker id, and note that the TASK-0301 axis-mapping defence is bypassed on this call (expected only on synthetic fixtures).
2. Verify the trace is byte-silent on `NUC_TRACE` unset (cycle-134 / TASK-0315 has already verified the macro is byte-silent in this crate; this AC is satisfied by re-running e2e + determinism baselines).
3. Optionally: pin the fall-back path with a unit test that builds a synthetic ACFG with no observed accesses and asserts the fall-back returns the nest-order vector.

## Honest scope

LOW priority. Same defensive-observability rationale as TASK-0315. The cycle-115 (TASK-0294) and TASK-0301 axis-mapping defences in `compute_partition_bounds_with_dim_prefix` are correct today; this task adds visibility so a future regression masked by the default-order fall-back is observable in production.

## Forward-carried from TASK-0315 cycle 134 orchestrator inline architecture review

Sibling-grep audit identified `rewrite_partition_tiles_inner` as the only other call site that pattern-matches the cycle-133 axis-mapping defence shape; `order_halo_strip_bounds_by_data_dim` and `compute_partition_bounds_with_dim_prefix` are the two data-dim-aware emit helpers in transfer_inject.rs, both with the same fall-back-on-synthetic-fixtures policy.
<!-- SECTION:DESCRIPTION:END -->
