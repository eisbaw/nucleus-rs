---
id: TASK-0278
title: Extend TASK-0273 reuse marker coverage to multi_worker_walker strip-mine arm
status: To Do
assignee: []
created_date: '2026-05-24 12:08'
labels:
  - M5
  - test-gap
  - reuse
  - forward-carried-from-TASK-0273
dependencies:
  - TASK-0273
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0273 cycle-98. TASK-0273 closed coverage for the NON-strip-mine call site to `render_reuse_marker_comment` (`multi_worker_walker.rs:478`, exercised via Event::Loop with `block_tag: None`). The STRIP-MINE call site (`multi_worker_walker.rs:404`, inside the `if let Some(tag) = block_tag` arm) remains UNCOVERED.

A regression that drops the marker emit from ONLY the strip-mine arm — e.g. while refactoring the per-occurrence absolute-index rebinding path or while wiring the real circular-buffer codegen for inner-block tile loops — would silently pass today.

## Why this matters now

The shipped `05-stencil/distributed.sched.nuc` carries `loop x : block=64, vectorize=8, reuse;` — block+reuse on the same iv. When TASK-0267 + TASK-0268 unblock that cell, it will execute the strip-mine call site live. Until then, the strip-mine arm's reuse-marker emit is structurally exercised in NO test.

## Acceptance

1. A third test in `nucleus/backend-common/tests/multi_worker_reuse_marker.rs` (or a sibling file) that constructs an `Event::Loop` carrying `block_tag: Some(BlockTag {...})` AND a non-strip-mined enclosing tile loop, populates `sidecar.reuse_widths` for the inner iv, calls `render_worker_events`, and asserts the marker substring appears.
2. The fixture should mirror `nucleus/backend-common/tests/multi_worker_blocked_rebind.rs` for the BlockTag + tile_loop construction — that file already shows the working shape.
3. Test runs in <30s.

## Dependencies

- Forward-carried from: TASK-0273 (cycle-98 honest-limits disclosure; gap was self-identified but not filed by implementer — orchestrator filing the prerequisite the implementer-contract required).
<!-- SECTION:DESCRIPTION:END -->
