---
id: TASK-0254
title: >-
  Consolidate block_tag rebinding arithmetic across single-worker + multi-worker
  render paths (RenderCtx <-> RenderCtxPub unification)
status: Done
assignee: []
created_date: '2026-05-23 19:59'
updated_date: '2026-05-23 21:38'
labels:
  - backend
  - tech-debt
  - M4
dependencies:
  - TASK-0253
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0253 (cycle 75) consolidated the per-occurrence BlockTag rebinding arithmetic across the MULTI-worker backends — `backend_common::multi_worker_walker` (consumed by pthreads-sync + pthreads-async multi-worker) and `mp-tcp-bufsync` both delegate to one shared `render_block_tag_loop_header` helper. **But a sibling copy of the same arithmetic survives on the pthreads-sync SINGLE-worker render path** at `nucleus/backends/pthreads-sync/src/lib.rs:602-654` (the `Event::Loop` arm of `render_events_in`).

The two flavours diverge by which RenderCtx they consume:
- Multi-worker walker + mp-tcp-bufsync: `RenderCtxPub` (the cross-backend pub mirror in backend-common)
- Single-worker pthreads-sync: backend-private `RenderCtx` (with `render_const_expr` private to the crate)

The shared helper `render_block_tag_loop_header` takes `&RenderCtxPub` — the single-worker arm cannot call it without a RenderCtx -> RenderCtxPub bridge OR a RenderCtx-typed sibling helper OR full RenderCtx <-> RenderCtxPub unification.

## Why this matters

Surfaced by cycle-75 mped-architect review MAJOR-1: TASK-0253's claim "the arithmetic lives in exactly ONE place" is FALSE without this consolidation. The codebase-wide grep:

```
backend-common/src/multi_worker_walker.rs:219  format!("({lo_src} + ({}_i64 * {n}_i64) + {var})", tag.num_full)
backend-common/src/multi_worker_walker.rs:238  format!("({lo_src} + ({tile_name} * {n}_i64) + {var})")
pthreads-sync/src/lib.rs:614                   format!("({lo_src} + ({}_i64 * {n}_i64) + {var})", tag.num_full)
pthreads-sync/src/lib.rs:633                   format!("({lo_src} + ({tile_name} * {n}_i64) + {var})")
```

shows 2 sites still. Drift between single-worker and multi-worker rebinding is structurally possible (single-worker doesn't go through the helper). Today the two are byte-for-byte equal, but a future edit to one without the other would silently diverge.

## Recommended approach

Three options; pick during implementation:

(a) **Unify `RenderCtx` and `RenderCtxPub`** — promote the private RenderCtx to be a thin wrapper around RenderCtxPub (or vice versa), so all rendering helpers consume the same type. Largest scope, cleanest end-state.

(b) **Add a RenderCtx -> RenderCtxPub bridge** at the single-worker call site — construct a temporary `RenderCtxPub::new(ctx.names, ctx.sidecar).with_abs_subst(ctx.abs_subst.clone())`, call the shared helper, propagate the returned child back as RenderCtx. Lossy if `RenderCtx` has fields RenderCtxPub doesn't.

(c) **Lift a sibling helper `render_block_tag_loop_header_priv`** that takes `&RenderCtx` (backend-private) and shares format strings with the pub version via a sub-helper. Smallest diff, retains the duplication at the format-string level but at least the divergence becomes a compile error.

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 #1 Decision recorded (a/b/c).
- [ ] #2 #2 The 4 format-string sites in the codebase collapse to ONE site (or to a shared sub-helper called by 2 sites that cannot drift in arithmetic).
- [ ] #3 #3 All blocked single-worker e2e cells (04/blocked, 05/blocked, 06/blocked, 07/blocked) stay byte-identical-green; multi-worker blocked tests stay green.
- [ ] #4 #4 `just determinism-check` clean; `just e2e` 88/70/0/18 preserved.
- [ ] #5 #5 The TASK-0253 helper docstring's carve-out note ("a separate sibling copy survives on the single-worker render path") + the mp-tcp-bufsync call-site comment can be deleted as obsolete.

## Dependencies

- Builds on TASK-0253 (cycle 75 — the multi-worker consolidation already done).
<!-- SECTION:DESCRIPTION:END -->

<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-trigger (orchestrator-direct, cycle 77 sweep). Filed cycle 75 by mped-architect MAJOR-1 review of TASK-0253. The task asks for a RenderCtx ↔ RenderCtxPub unification so the pthreads-sync SINGLE-worker block_tag rebinding arithmetic (which uses backend-private RenderCtx) shares one implementation with the multi-worker shared helper (which uses pub RenderCtxPub). Today: the two flavours are byte-for-byte equal at the arithmetic level (cycle 75 confirmed via diff); divergence is structurally possible but has not occurred. The unification itself is a substantive RenderCtx-redesign refactor (~3 options enumerated in the task description) requiring fresh context per the loop spec's 'deep refactor' stop signal. Reopen at the first measured drift between the two flavours OR when a RenderCtx-side refactor for unrelated reasons lands and the unification becomes a natural rider. Same deferred-no-trigger pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
