---
id: TASK-0273
title: Extend reuse_widths_pending marker assertion to multi_worker_walker path
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 08:46'
updated_date: '2026-05-24 12:00'
labels:
  - M5
  - test-gap
  - reuse
dependencies:
  - TASK-0265
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0265 cycle-87 review (architect P2 finding).

TASK-0265 Tier 1 landed `render_reuse_marker_comment` at TWO sites:
- `nucleus/backends/pthreads-sync/src/lib.rs::render_event` (single-worker emit path)
- `nucleus/backend-common/src/multi_worker_walker.rs` (multi-worker emit path)

The grep test `nucleus/nucleus-compiler/tests/e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive` only exercises the FIRST site. It runs `--backend pthreads-sync` against the single-host `reuse.sched.nuc` schedule, which routes through `render_event`. The multi_worker_walker.rs call site is NOT covered.

A regression that drops the marker emit from `multi_worker_walker.rs` (but not `render_event`) would silently pass.

## Why the gap exists today

The only shipped multi-worker reuse schedule is `nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc` which carries `loop x : block=64, vectorize=8, reuse;`. That cell is currently [[skip]]ped across all 4 backends due to TASK-0267 (host-Push synthesis drop) + TASK-0268 (sync_inject barrier deadlock). Until those land, there is no e2e cell exercising multi_worker_walker.rs with a reuse-tagged loop.

## Two paths to close

**Option A (blocked-on-siblings)**: wait for TASK-0267 + TASK-0268 to land, then add `reuse_distributed_multi_worker_marker_present` test that runs `--backend pthreads-sync` (or any backend; all 4 share the walker) against `distributed.sched.nuc` and greps for the marker on each per-worker emit.

**Option B (synthetic fixture)**: add a hand-built fixture in `nucleus/nucleus-compiler/tests/` that builds an ACFG with `reuse_widths` populated AND `partition_worker_ranges` populated, then calls the walker directly via `render_worker_events` and asserts the marker substring appears. Decouples this coverage from TASK-0267/0268 closure. Costs: hand-building the fixture + threading the `RenderCtxPub` etc.

## Acceptance

1. A test in `nucleus/nucleus-compiler/tests/` OR `nucleus/backend-common/tests/` that exercises `multi_worker_walker.rs`'s call to `render_reuse_marker_comment` with a non-empty `reuse_widths` sidecar entry AND asserts the marker substring appears in the per-worker emit.
2. Symmetric ABSENCE: same fixture with empty `reuse_widths` asserts ZERO occurrences (defensive — catches an over-eager emit).
3. Test runs in < 30s (no full cargo build cycle if possible — favour direct walker invocation).

## Dependencies

- Forward-carried from: TASK-0265 (cycle-87 architect P2 review item).
- Option A depends on: TASK-0267 + TASK-0268 (the runtime bugs blocking 05-stencil/distributed).
- Option B: standalone.
- Related: TASK-0269 (when real circular-buffer codegen lands on the walker, the marker substring may rename to `reuse_buf_decl` or similar — update test in lockstep).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Plan (cycle 98)

Option B (standalone synthetic). Clone fixture shape from `nucleus/backend-common/tests/multi_worker_blocked_rebind.rs`.

New file: `nucleus/backend-common/tests/multi_worker_reuse_marker.rs`.

Two tests:
1. `multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated`:
   - WalkerCtx fixture with sidecar.reuse_widths[iv][data][axis=0] = ReuseSlot{length=3, min_offset=-1}.
   - Event::Loop(iv, 0..16, body=[]) — non-strip-mine arm (block_tag: None), hits the line-478 call site.
   - Assert: contains 'reuse_widths_pending' substring; also iv name, data name, length, min_offset payload (catches drop-marker AND drop-payload regressions).
2. `multi_worker_walker_skips_reuse_marker_when_reuse_widths_empty`:
   - Same fixture but reuse_widths empty.
   - Assert !contains('reuse_widths_pending').

Path-verified facts:
- ReuseSlot at `nucleus_compiler::passes::reuse_inference::ReuseSlot` (re-exported from `nucleus_compiler` per lib.rs line 72: `apply_reuse_inference, apply_reuse_inference_advisory, ReuseInferenceError, ReuseSlot`).
- backend-common's only dep is nucleus-compiler — direct import works.
- Map shape: `BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64 /* axis */, ReuseSlot>>>` (axis is u64, not usize).
- WalkerCtx fields: names, sidecar, rendezvous_prefix, rendezvous_ids, pair_tiles.
- The line-478 call site fires the marker AFTER writing 'for {var} in (lo)..(hi) {', so an empty body is fine.

No production code changes.
<!-- SECTION:NOTES:END -->
