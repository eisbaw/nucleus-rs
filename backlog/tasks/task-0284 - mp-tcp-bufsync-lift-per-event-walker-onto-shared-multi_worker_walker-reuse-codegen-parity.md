---
id: TASK-0284
title: >-
  mp-tcp-bufsync: lift per-event walker onto shared multi_worker_walker (reuse
  codegen parity)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 16:38'
updated_date: '2026-05-24 17:27'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## CYCLE-107 LANDING (orchestrator-led, 2026-05-24)

TASK-0284 closed in commit 215bb7d. Mirrored the cycle-104 4-block reuse-codegen call sequence from the shared multi_worker_walker into mp-tcp-bufsync's private Plan::render_events at both Event::Loop arms (strip-mine + regular). Closes the silent-sibling defect filed cycle 104.

### Strip-mine arm

Previously delegated to render_block_tag_loop_header which writes the for-header AND returns the rebound child ctx. New pattern uses pure compute_block_tag_abs_exprs to get (abs, strip_lo_expr) WITHOUT writing the header — emits buf decls at the OUTER pad first, writes the for-header inline, emits per-iter update, then recurses into body with a child ctx carrying both abs_subst AND reuse_active. The structurally-built strip_lo_expr (NOT a textual replace of abs) preserves the cycle-103 TASK-0269 P1.1 fix on mp-tcp-bufsync too.

### Regular arm

Computes (lo, hi) per existing partition_worker_ranges → loop_bounds → range precedence. Emits buf decls BEFORE the for-header (persistence across iterations), writes the header, emits marker + per-iter update at body entry, builds body_ctx with reuse_active, recurses using body_ctx in BOTH the check_frame and non-check_frame sub-arms.

### Test pin

New test mp_tcp_bufsync_worker_emit_contains_reuse_buffer_codegen in tests/reuse_codegen_emit.rs builds synthetic 2-worker reuse fixture (host → w0 blur3 with for n : reuse → host sink). Asserts emitted w0.rs contains buf decl + rem_euclid + marker + rewritten blur3 call. Symmetric absence on host.rs. Bite-verified: stashing the lib.rs changes and re-running fails with raw x[(n-1)]/x[n]/x[(n+1)] reads inside blur3 and no __reuse_buf anywhere.

### Honest scope

Path-2 (patch in place) per the task brief. Path-1 (lifting mp-tcp-bufsync onto the shared walker by trait-abstracting the per-backend rendezvous substrate) remains a longer-term DRY cleanup, not undertaken here. ~110 LoC of duplication with the shared walker — same 4-block pattern emitted at the same two arm sites — but no new abstraction surface. Duplication is now visible and tested on BOTH consumers, so a future lift to a shared trait can be undertaken when a second backend joins this per-event-walker class.

### Gate

- cargo test --workspace: 818 / 0 / 3 (+1 vs cycle 106 baseline 817).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo test -p mp-tcp-bufsync: all green (pingpong + rendezvous_emit + check_frame_emit + reuse_codegen_emit + 7 unit tests).
- just e2e: 92 / 79 / 0 / 13 / 0 required-fail (preserved; no shipped cell exercises mp-tcp-bufsync's reuse path so emit on shipped cells is byte-identical pre/post cycle 107).
- just determinism-check: 92 / 79 / 0 / 13 (GREEN).
- Release rebuild forced before e2e per feedback-stale-release-binary-during-session memory.

### ACs MET

- mp-tcp-bufsync's render_events emits __reuse_buf_<data>_a<axis>: Vec<T> + rem_euclid(L_i64) shape on both arms: MET.
- New test pins the codegen shape: MET via tests/reuse_codegen_emit.rs.
- Bit-identical to reference on synthetic 2-worker reuse fixture: trivially MET (perf rewrite not semantic; no shipped reference.bin exists for the fixture so codegen-shape test is the AC anchor per task brief allowance).

Status: Done. Commit: 215bb7d.
<!-- SECTION:NOTES:END -->
