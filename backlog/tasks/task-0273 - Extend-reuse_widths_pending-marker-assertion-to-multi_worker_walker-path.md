---
id: TASK-0273
title: Extend reuse_widths_pending marker assertion to multi_worker_walker path
status: To Do
assignee: []
created_date: '2026-05-24 08:46'
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
