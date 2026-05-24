---
id: TASK-0269
title: M5 Stage 2 — pthreads-sync real circular-buffer codegen (TASK-0265.01)
status: To Do
assignee: []
created_date: '2026-05-24 08:31'
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
Tier 2 of TASK-0265 — forward-carried from cycle 87.

The Tier 1 marker scaffold (commit 7d03606) wired walker-side LOOKUP of reuse_widths and emits a reuse_widths_pending comment at every Event::Loop body entry. This task replaces the comment with actual delay-line / circular-buffer Rust code on pthreads-sync (the simplest single-worker variant).

## Scope
At every Event::Loop body entry where sidecar.reuse_widths.get(iter_var) is Some, per (DataId, axis, ReuseSlot):

1. Declare a Vec<T> circular buffer of length elements (T from sidecar.data_type(data_id).scalar).
2. Emit an initial-fill prologue. For min_offset=-1, length=3 (offsets {-1, 0, +1}): seed buf[0] and buf[1] with img_in[y][0] and img_in[y][1] BEFORE the first body iteration; buf[2] gets img_in[y][x+1] inside the body.
3. At the start of each iteration, load the most-distant element into the buffer.
4. Rewrite every img_in[y][x+b] DataRef inside the body's Fire args to buf[(x + b - min_offset) as usize % length]. Other axes (img_in[y-1][x] — y-axis read) NOT rewritten. The rewrite happens at the Fire-arg DataRef render site (render_fire_arg in backend-common/render.rs).

## Coordination
Single-worker path (pthreads-sync render_event Event::Loop arm; also delegated to by pthreads-async, mp-tcp-bufsync, mp-tcp-event when single-host). Multi-worker landing is TASK-0265.02.

## Forward-carry from TASK-0265 Tier 1 (cycle 87)
- Marker substring reuse_widths_pending is grep-able; AC#4 test pins both presence + absence. When real codegen lands, the marker SHOULD stay (or be subsumed by a new substring) so the e2e detection test keeps firing as a no-Stage-1-regression canary.
- New 05-stencil/reuse.sched.nuc cell currently PASSES on all 4 backends bit-identical to reference.bin. Real codegen MUST keep output bit-identical (reuse is perf rewrite, not semantic).
- Rewrite-site: render_fire_arg has access to ctx.sidecar but currently has no reuse context. Threading active reuse slots through RenderCtx/RenderCtxPub is the cleanest path; alternative is a side-channel arg-rewrite map populated in the Event::Loop arm.
<!-- SECTION:DESCRIPTION:END -->
