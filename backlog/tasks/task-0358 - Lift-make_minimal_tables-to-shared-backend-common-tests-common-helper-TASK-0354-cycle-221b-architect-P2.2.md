---
id: TASK-0358
title: >-
  Lift make_minimal_tables to shared backend-common tests/common/ helper
  (TASK-0354 cycle-221b architect P2.2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-28 00:48'
updated_date: '2026-05-28 03:57'
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

## Cycle 227 implementation plan (orchestrator-direct; spawned implementers refuse code edits in this repo per feedback-spawned-agents-refuse-code-edits)

Scope is WIDER than the cycle-221b filing: not 3 but FIVE table-fixture helpers across 5 files (4x make_minimal_tables + 1x make_tables), in TWO name-colliding families:
- Family A (data-centric, sig (DataId,&str,Vec<usize>)): collect_let_at_wait_data.rs (9 calls, no workers) + wait_assign_slice.rs (11 calls, +WorkerId(0)=w0/WorkerId(1)=host).
- Family B (iv/kernel/loop-bound builders, distinct sigs): multi_worker_reuse_marker.rs (3) + multi_worker_blocked_rebind.rs (4) + block_tag_loop_header.rs make_tables (3).
Plus tile_{1,2,3}d duplicated byte-identically in whole_array_classifier.rs + collect_let_at_wait_data.rs (+ tile_1d in collect_pair_tiles.rs) and empty_accumulate_and_indexed (collect_let_at_wait_data.rs).

Plan:
1. New tests/common/mod.rs with #![allow(dead_code)] (cargo compiles it into EVERY consuming test binary; subset-use would trip the -D warnings clippy gate otherwise). Hosts: chainable Tables builder (with_data / with_data_name / with_worker / with_default_workers / with_iter_var / with_loop_bound / with_kernel_i64 / build) = single construction primitive; make_minimal_tables(d,n,dims) [Family-A no-worker] + make_minimal_tables_with_workers(d,n,dims) [Family-A +default workers] thin delegations; tile_1d/2d/3d; empty_accumulate_and_indexed.
2. Migrate: collect_let_at_wait keeps make_minimal_tables name (byte-unchanged calls), drops local helper; wait_assign_slice renames 11 calls to make_minimal_tables_with_workers; Family-B 10 calls rewritten to the Tables builder directly (exact same entry set per site, verified against original bodies).
3. Each file: add mod common; + use, drop now-unused imports (clippy -D warnings will pin them).
4. Preserve EXACT per-site fixture state (AC#2): collect_let_at_wait gets NO worker entries; reuse_marker gets NO data_types (with_data_name, not with_data).
5. Gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e'. Test-only change -> e2e must be bit-identical; cargo test count must be unchanged.
Out of scope (intentionally separate, per forward-carry): src/multi_worker_walker/safe_push_reorder.rs:748 tile_2d (IterVar param, in-source convenience).

## Cycle 227 review gate (parallel read-only) + fold-back — GO

qa-test-runner (independent re-run): clippy clean under -D warnings; just test 1037 passed / 0 failed (all 7 originally-touched binaries preserve their counts: block_tag 4, collect_let_at_wait 9, collect_pair_tiles 5, blocked_rebind 4, reuse_marker 6, wait_assign_slice 11, whole_array 7); just test-release 0 failures (release total -1 = the documented TASK-0291 debug_assert-gated #[should_panic] compile-out, not a regression); just e2e 280/246/0/34/0 — IDENTICAL to baseline. GO.

mped-architect (read-only): GO, no P1/P2. Verified AC#2 fixture-state preservation BYTE-FOR-BYTE incl. the with_data_name (no data_types) vs with_data (i32 data_types) distinction for reuse_marker, and the wait_assign_slice 'make_minimal_tables_with_workers as make_minimal_tables' alias preserving w0/host. Ruled AC#4 HONEST (not AC-gaming): construction-logic duplication IS eliminated — every retained adapter body is a single Tables chain, so a NameTables/NameSidecar field change touches ONE place. Doc/comment spot-checks accurate; #![allow(dead_code)] justification load-bearing & genuine; builder appropriate (task asked for builder-style), not over-engineered.

Architect P3.1 (silent-sibling census incomplete) — FOLDED IN this cycle: the census was 5 helpers, but a 6th pair-fixture helper existed — wait_assign_accumulate.rs::make_histogram_tables (1 host + 4 senders, parameterized scalar I32/F32, 5 call sites). Added Tables::with_data_typed(data,name,scalar,dims) (with_data now delegates to it with i32) and converted make_histogram_tables to a thin builder-delegating adapter. Orchestrator-own grep sweep for all tests/ (NameTables/NameSidecar)-returning fns + data_types/worker/loop_bounds/kernel_sigs inserts confirms the ONLY remaining construction sites are: (a) whole_array_classifier::sidecar_with — returns NameSidecar ONLY (not the pair), narrow single-file, parameterized scalar, NO names.data — legitimately separate; (b) inline per-test sidecar mutations in reuse_marker (lines ~440-767) — test-body-specific setup layered on the make_minimal_tables base, correctly NOT helpers. safe_push_reorder.rs:748 tile_2d (IterVar param, in-source) confirmed correctly out of scope. Census now complete: 6 pair-fixture helpers all route through common::Tables.

Final tally: NEW tests/common/mod.rs (Tables builder + with_data/_typed/_name, with_worker/_default_workers, with_iter_var, with_loop_bound, with_kernel_i64, build; make_minimal_tables/_with_workers; tile_1d/2d/3d; empty_accumulate_and_indexed). 8 test files migrated. Net -111 LoC (first-pass) before the +histogram fold.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle 227. Lifted the drifted test-fixture construction logic in nucleus/backend-common/tests/ into a single shared primitive (common::Tables chainable builder) in the first repo tests/common/mod.rs. All 6 pair-fixture helpers (4x make_minimal_tables + make_tables + make_histogram_tables) now route through Tables: the 2 cross-file Family-A shapes are canonical named helpers in common (make_minimal_tables / make_minimal_tables_with_workers); the file-specific shapes (reuse_marker, blocked_rebind, block_tag, histogram) are thin file-local adapters whose bodies delegate to Tables (call sites byte-identical, per-fixture WHY-docs preserved). tile_1d/2d/3d + empty_accumulate_and_indexed also lifted. A NameTables/NameSidecar field change now touches ONE site. All 4 ACs met; parallel read-only review gate GO (qa + architect); gate green: clippy clean, just test 1037/0, just test-release 0 fail, just e2e 280/246/0/34/0 unchanged (test-only). Architect P3.1 silent-sibling (6th census site) folded in same cycle; census now provably complete.
<!-- SECTION:FINAL_SUMMARY:END -->
