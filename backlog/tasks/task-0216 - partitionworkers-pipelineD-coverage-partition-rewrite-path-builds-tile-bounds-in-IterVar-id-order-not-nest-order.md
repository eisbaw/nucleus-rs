---
id: TASK-0216
title: >-
  partition=workers + pipeline=D coverage; partition-rewrite path builds tile
  bounds in IterVar id order not nest order
status: Done
assignee: []
created_date: '2026-05-21 14:10'
updated_date: '2026-05-21 20:12'
labels:
  - compiler
  - partition
  - M4
  - latent
dependencies:
  - TASK-0134
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0134 cycle): rewrite_partition_tiles_inner at transfer_inject.rs:1583+ builds IterTile bounds by iterating partition_ranges (BTreeMap<IterVar, ...>) in IterVar-id order, NOT nest order. For typical schedules id-order coincides with nest-order (IterVar IDs are walk-order assigned), but it is not guaranteed by the IterTile::bounds convention 'outer-most first'. The 'innermost wins' semantic in annotate_pipeline_depth_for_seq's .rev() walk silently breaks when nest-order != id-order. Combining partition=workers with pipeline=D is also not exercised by any test or example.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Synthetic fixture (or example-13 batch_parallel + pipeline=) that combines partition=workers with pipeline=D on the partitioned loop. Assert pipeline_depth_for_seq is populated correctly for each per-worker fan-out pair.
- [ ] #2 Fix rewrite_partition_tiles_inner: build bounds in nest-order, not IterVar-id order. The fix is to walk the enclosing Repeat stack instead of partition_ranges. Or: assert at construction that the produced ordering matches the existing tile's ordering.
- [ ] #3 Add forward-carry into TASK-0042.01 (pthreads-async): the codegen ring-buffer pre-fill must apply per fan-out (src,dst) pair, not per data symbol — one initial_marking entry per (data, src_worker, dst_worker) tuple.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle (with TASK-0224 scope-split for the deeper fix)

AC#1: synthetic-fixture test in tests/transfer_inject.rs (`partition_with_pipeline_populates_pipeline_depth_per_fanout_pair`) — combines partition=workers (4 fan-out workers w1..w4 over iter-var n with B=8 split into 2-element slices) with pipeline=D=2 on the partitioned loop. Asserts each fan-out pair's seq has pipeline_depth_for_seq[seq]=2 and the sidecar has one entry per unique seq (4 seqs from 4 fan-out pairs).

AC#2 PARTIAL: rather than rewriting `rewrite_partition_tiles_inner` to walk the enclosing Repeat stack (which would require non-trivial restructuring — the function doesn't receive stack context), this cycle:
- Added a clarifying inline comment at the bounds-construction site documenting the assumption (BTreeMap iteration = IterVar-id order = coincidentally nest-order for current single-iter-var schedules) and the latent risk for future nested-partition schedules.
- Filed TASK-0224 as the explicit follow-up for the deeper "walk enclosing Repeat stack" fix.

AC#3: forward-carry to TASK-0042.01 notes — pthreads-async ring-buffer codegen should use per-fan-out-pair (data, src_worker, dst_worker) as the unit for ring sizing, not per-data.

### Implementation
- `nucleus/compiler/tests/transfer_inject.rs`: new test (~75 lines) — synthetic ACFG with partition_worker_ranges + pipeline=D schedule directive; asserts pipeline_depth_for_seq is populated for every fan-out pair's seq.
- `nucleus/compiler/src/passes/transfer_inject.rs`: 12-line inline comment at the bounds-construction site documenting the iteration-order assumption + risk + pointer to TASK-0224 + the new test name.
- TASK-0224 filed with crisp ACs for the deeper fix.

### Gate (orchestrator re-ran)
- cargo test workspace: 549 pass / 0 fail / 2 ignored (was 548/0/2; +1).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 / 0 / 7 (baseline unchanged).
<!-- SECTION:NOTES:END -->
