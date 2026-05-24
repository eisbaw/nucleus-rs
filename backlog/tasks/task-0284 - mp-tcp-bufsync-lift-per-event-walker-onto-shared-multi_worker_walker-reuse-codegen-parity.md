---
id: TASK-0284
title: >-
  mp-tcp-bufsync: lift per-event walker onto shared multi_worker_walker (reuse
  codegen parity)
status: To Do
assignee: []
created_date: '2026-05-24 16:38'
labels:
  - mp-tcp-bufsync
  - reuse
  - silent-sibling
  - TASK-0270-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P2.1 + QA P3.1 from TASK-0270 cycle-104 review: mp-tcp-bufsync has its own per-event walker (nucleus/backends/mp-tcp-bufsync/src/lib.rs::Plan::render_events lines 772-...) that emits both Event::Loop arms (strip-mine + regular) WITHOUT the new reuse circular-buffer codegen that TASK-0270 wired into backend_common::multi_worker_walker::render_worker_events_inner.

## What's missing on mp-tcp-bufsync

Compared to pthreads-async + mp-tcp-event (which consume the shared walker and get reuse codegen for free), mp-tcp-bufsync's render_events at both arms:
- strip-mine arm (line 857-873): delegates to backend_common::multi_worker_walker::render_block_tag_loop_header for the inner header only, then directly recurses into body. No render_reuse_buf_decls_pub call, no render_reuse_per_iter_update_pub call.
- regular arm (line 880-895): computes (lo, hi) via partition_worker_ranges fallback, writes for-header, recurses. No reuse codegen.

The marker comment render_reuse_marker_comment is ALSO absent from both arms (not even the Tier 1 scaffold ever fired here). This is a deeper silent-sibling defect than TASK-0270 acknowledged.

## Why it's dormant today

05-stencil/distributed × mp-tcp-bufsync is SKIPPED on a separate capability mismatch (async + buffer + notify=event not supported by mp-tcp-bufsync). 05-stencil/reuse × mp-tcp-bufsync IS exercised but routes through the single-host path (workers={host}) which delegates to pthreads-sync's render_single_worker_main — the new reuse codegen lands there. So mp-tcp-bufsync's lifeline tier-1 cells do NOT exercise the missing path.

## When the defect bites

If a future tier-1 schedule introduces multi-worker reuse on mp-tcp-bufsync (sync notify + buffer=1) — e.g. a partition_rows + reuse combination compatible with mp-tcp-bufsync's capability surface — the emit will silently lack reuse codegen, producing slower-than-spec but still bit-identical output (reuse is perf, not semantic). The lack of a marker means even grep-based regression tests will not surface the absence.

## Scope

Two paths:
1. **Lift mp-tcp-bufsync's render_events onto the shared multi_worker_walker.** Requires teaching the shared walker about mp-tcp-bufsync's per-backend substrate (TCP ctrl_<peer> / sock_<peer> barriers, host-vs-worker dispatch). High-effort: the shared walker is currently shaped around the pthreads ring/barrier model and a TCP-rendezvous variant would need a trait abstraction.
2. **Patch mp-tcp-bufsync's render_events to call render_reuse_buf_decls_pub + render_reuse_per_iter_update_pub at both arms** mirroring the shared walker's pattern. Lower-effort: copy the 4-block call sequence (compute_block_tag_abs_exprs for strip-mine, compute (lo, hi), call render_reuse_buf_decls_pub, write header, call render_reuse_per_iter_update_pub, recurse with reuse_active-extended child ctx). Adds ~30 LoC of duplication with the shared walker (DRY cost) but no new abstraction.

Recommend path 2 as the immediate parity fix, with path 1 as a separate longer-term task.

## Acceptance
- mp-tcp-bufsync's render_events emits the same __reuse_buf_<data>_a<axis>: Vec<T> + rem_euclid(L_i64) shape that the shared walker emits, on both Event::Loop arms.
- New test in mp-tcp-bufsync's tests/ pins the codegen shape (mirror multi_worker_reuse_marker.rs's two arm-specific tests).
- A synthetic 2-worker reuse fixture (or the first shipped reuse schedule that lands on mp-tcp-bufsync's capability surface) is bit-identical to reference.bin.

## Honest scope

This is hygiene + future-proofing. There is NO current shipped tier-1 cell that surfaces the defect. File and defer until the first multi-worker reuse cell on mp-tcp-bufsync's capability surface lands.

## Dependencies
- TASK-0270 (Done).
- Independent of TASK-0282 (multi-outer-coord generalisation — that lifts the narrow-rewrite-cut, which is orthogonal to the mp-tcp-bufsync parity).
<!-- SECTION:DESCRIPTION:END -->
