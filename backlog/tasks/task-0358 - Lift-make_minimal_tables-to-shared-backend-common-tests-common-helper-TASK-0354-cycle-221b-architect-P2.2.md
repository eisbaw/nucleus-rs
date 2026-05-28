---
id: TASK-0358
title: >-
  Lift make_minimal_tables to shared backend-common tests/common/ helper
  (TASK-0354 cycle-221b architect P2.2)
status: To Do
assignee: []
created_date: '2026-05-28 00:48'
updated_date: '2026-05-28 01:17'
labels:
  - tests
  - backend-common
  - refactor
  - cycle-221b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-221b architect P2.2: TASK-0354 cycle-221 added a third copy of the `make_minimal_tables(data_id, name, dims) -> (NameTables, NameSidecar)` test-fixture helper. Sibling sites:

- `nucleus/backend-common/tests/wait_assign_slice.rs:143-162` — populates worker entries WorkerId(0)+"w0" and WorkerId(1)+"host".
- `nucleus/backend-common/tests/block_tag_loop_header.rs:40-..` — different signature (`make_tables`), populates KernelSig + LoopBound.
- `nucleus/backend-common/tests/collect_let_at_wait_data.rs:67-84` (NEW cycle 221) — omits worker entries (caller never reads `names.worker`).

The new file's docstring honestly notes the divergence, but per `feedback-silent-sibling-defect` + TASK-0237 precedent (which already proposed shared-test-helper crate extraction) the next iteration of this drift is a defect surface: any future test that adds a 4th fixture site, or any future change to NameSidecar / NameTables, must update all three independently.

## Acceptance

1. Create `nucleus/backend-common/tests/common/mod.rs` (or `tests/common.rs` if cargo's per-test-binary discovery prefers the flat layout — verify) housing one canonical `make_minimal_tables` with optional builder-style parameters for worker entries, KernelSig, LoopBound.

2. Migrate the 3 call sites to consume the canonical helper. Verify each migration preserves the existing semantics — diff the fixture state at the test entry before/after the migration.

3. Re-run `cargo test --workspace` and `just e2e` to confirm no regression (test-only refactor; e2e must be bit-identical to pre-cycle 280/246/0/34/0).

4. Remove the 3 site-local helpers.

## Honest scope

Test-only refactor; no production code touched. Low priority because the divergence is currently honest (each site documents its variation). Promote to MEDIUM if a 4th call site is added or if NameTables/NameSidecar grow.

## Forward-carry

- Memory: `feedback-silent-sibling-defect` (the drift pattern).
- TASK-0237 already proposed shared test-helper crate extraction at cycle-37; this task is the concrete next step.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0357 cycle-223 architect P3.2: the new tile_2d + tile_3d IterTile helpers added to tests/collect_let_at_wait_data.rs in cycle 223 (alongside the existing tile_1d carried from cycle 221) join the shared-helper migration scope of this task. When the shared tests/common/ module lands per AC#1, the canonical home should host: (a) make_minimal_tables(data_id, name, dims), (b) tile_1d/tile_2d/tile_3d, (c) empty_accumulate_and_indexed. Sibling note: nucleus/backend-common/src/multi_worker_walker/safe_push_reorder.rs:748 has its own tile_2d with IterVar (not u64) parameter type — different convenience profile, intentionally separate from the test-fixture variants. The migration should preserve both: shared test-fixture tile_{1,2,3}d (u64 → IterVar) AND in-source safe_push_reorder::tile_2d (IterVar parameter).
<!-- SECTION:NOTES:END -->
