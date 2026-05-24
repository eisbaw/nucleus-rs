---
id: TASK-0270
title: M5 Stage 2 — multi-worker walker real circular-buffer codegen (TASK-0265.02)
status: To Do
assignee: []
created_date: '2026-05-24 08:32'
updated_date: '2026-05-24 11:22'
labels:
  - M5
  - codegen
  - reuse
  - stage-2
  - forward-carried-from-TASK-0265
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier 3 of TASK-0265 — forward-carried from cycle 87.

Sibling of TASK-0269 (TASK-0265.01, single-worker). Whereas TASK-0269 targets render_event in pthreads-sync (used by ALL 4 backends for single-host schedules), this task wires the real circular-buffer emit at the SHARED multi-worker walker (backend-common/src/multi_worker_walker.rs::render_worker_events_inner Event::Loop arm).

## Affected backends
Multi-worker walker is consumed by:
- pthreads-sync (multi-worker)
- pthreads-async (multi-worker)
- mp-tcp-bufsync (multi-worker)
- mp-tcp-event (multi-worker)

All four pick up the buffer emit at once because the walker is the single source of truth.

## Scope
Identical rewrite shape to TASK-0269 but at the walker site:
1. At Event::Loop entry (BOTH the strip-mined block_tag branch AND the regular branch), read sidecar.reuse_widths.get(iter_var).
2. For each (DataId, axis, ReuseSlot), declare a Vec<T> circular buffer + initial-fill prologue + per-iteration rotate.
3. Rewrite DataRefs in body Fire args.

## Coordination with TASK-0263 halo Stage 2
Halo Stage 2 (TASK-0263 already landed for transfer_inject) and reuse Stage 2 (this task) BOTH consume the same Event::Loop emit site. They are orthogonal:
- Halo widens per-tile transfer ranges (pre-loop, NOT inside the body).
- Reuse rewrites read patterns INSIDE the loop body.
A loop carrying BOTH a halo entry AND a reuse entry needs both code paths active simultaneously. The walker already handles halo via transfer_inject's sidecar; reuse adds an inner-body rewrite.

## Coordination blockers
05-stencil/distributed × {pthreads-async, mp-tcp-event} cells are currently SKIP due to TASK-0267 (host-Push drop under partitioned consumers + async transfer) and TASK-0268 (sync_inject barrier deadlock on unequal-iter partitioned bodies). Those unblock the COMBINED partition+block+reuse exercise. This task can land WITHOUT them — a synthetic 2-worker reuse fixture serves as the test target. Once 0267/0268 land, 05-stencil/distributed's  becomes the integration test.

## AC
1. multi_worker_walker emits Vec<T> circular buffer + rewrite for any Event::Loop carrying reuse_widths.
2. A new multi-worker e2e fixture (synthetic 2-worker reuse) is bit-identical to its reference.
3. just e2e + just determinism-check stay GREEN on all 4 tier-1 backends.
4. cargo test --workspace stays GREEN; unit + integration tests cover the rewrite.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Reconstructed lost fragment from create-time bash interpolation: the integration target is 05-stencil/distributed loop x with block=64, vectorize=8, reuse; once TASK-0267 and TASK-0268 land.

## Forward-carried from TASK-0275 (cycle 96, halo (B) promotion)

Multi-worker walker reuse codegen lands AFTER TASK-0269 (pthreads-sync first). Two carry items:

1. The reuse driver is STRICT (TASK-0271), not (B) partition-policy-aware like halo (TASK-0275). The reason: reuse marker is universally consumed across every recognised slot; halo's transfer_inject consumer is conditional on partition=. Do not transplant the halo (B) shape here.

2. If you extend reuse_inference's walker to thread additional context (e.g. partition= for multi-worker scope checks), introduce a type alias for the paired-Vec return EARLY — clippy::type_complexity fires on the bare `Vec<(Error, Vec<String>)>` shape. See halo_inference.rs `HaloErrorWithScope` for the pattern.
<!-- SECTION:NOTES:END -->
