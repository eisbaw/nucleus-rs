---
id: TASK-0418
title: >-
  Audit: silent event-drop sweep of backend emit (backend-common walker +
  per-backend fire renderers)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 22:58'
updated_date: '2026-06-01 23:26'
labels:
  - hardening
  - audit
  - silent-drop
  - backend
  - cycle-239-followup
dependencies:
  - TASK-0417
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0417 (cycle-239) as the OUT-OF-SCOPE half. TASK-0417 swept the nucleus-compiler IR-construction/transformation surface for silent statement/edge drops (build_dataflow was the only one; fixed by TASK-0360). The BACKEND emit surface is the untested sibling: a backend that consumes the Event list and emits code could silently skip an event (a Fire/Push/Wait/Sync that should emit code but emits nothing) via a `_ => {}` match arm, a `continue`, or a filter — producing a deadlock or wrong-answer with no diagnostic.

SCOPE: backend-common (multi_worker_walker render_worker_events, render/fire, check_frame) + each of the 7 tier-1 backends fire/event renderers. Look for: Event match arms that fall through to `_ => {}` or skip without emitting; filter/continue over the event list. Classify each as legit (e.g. Sync handled structurally elsewhere) vs silent-drop hazard. For any hazard: fail-loud (EmitError or debug_assert) + a bite test. Mirror the TASK-0417 classification method.

NOTE: the e2e bit-identical differential is a strong backstop here (a dropped event usually breaks output), but a backend-SPECIFIC drop on a [[skip]]ped cell, or a drop that happens to be output-neutral but deadlocks under a different topology, would NOT be caught by the current e2e matrix. That gap is the value of this audit.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle-240 audit + architect verification — GO; backend emit is STRUCTURALLY safe from silent event-drops ===

Method: Explore enumerated every Event match across backend-common + all 10 backends; orchestrator verified the main render dispatches directly; mped-architect independently re-verified (read-only) incl. the MPI/openmp/event-plan paths the orchestrator could not personally confirm.

CONCLUSION (architect GO): a silent event-drop of the build_dataflow class is STRUCTURALLY IMPOSSIBLE in the backend emit surface. There are FIVE main per-event render dispatches, ALL exhaustive (zero `_ =>` catch-all), so Rust exhaustiveness checking forces every Event variant (Fire/Loop/Sync/Push/Wait/Alloc/Free) to be handled or LOUD-rejected:
1. multi_worker_walker/event_walker.rs:108 (shared multi-worker: pthreads-sync/async, openmp-rs, mp-tcp-event/mp-uds-event, BOTH mpi backends).
2. tcp_plan/events.rs:60 (mp-tcp-bufsync, mp-tcp-poll).
3. embedded-pattern/render.rs:158 (embedded single + multi-MCU; Alloc/Free LOUD UnsupportedFeature).
4. pthreads-sync/lib.rs:525 (single-worker render: Sync/Push/Wait -> ContractGap, Alloc/Free -> Ok(()) RAII no-op). SHARED as the single-worker renderer by openmp-rs, mp-*-event, and both mpi backends.
(architect counted this as 5 because the single-worker renderer is shared more widely than the orchestrator listed.)

GAPS RESOLVED by architect: MPI backends are thin shims (type Plan = mpi_plan::Plan<_, Rendezvous>) delegating to dispatch 1 + 4 — no local event dispatch. openmp-rs delegates (the `_ => "0"` at multi_worker.rs:564 is rust_scalar_zero, an expr renderer). event-plan backends delegate to the exhaustive walker VERBATIM (the worker_program "own walker for Sync" was a COMMENT LIE — fixed this cycle 24ba0e2; barriers ride a bar_<tag> CTRL-channel shim local, not a separate dispatch; host-relay splice covers the full list with no gap).

All 34 `_ =>` arms (architect re-counted: 34, not the orchestrator preliminary 32) classified NON-dispatch: selective collectors (collect_*/walk_*), expr/type renderers (rust_scalar_zero, BinOp simplify), predicates (has_check_frame), or loud guards (unreachable!/panic! in EMITTED code). Alloc/Free no-ops are correct (RAII Vec tier-1; loud on embedded). Event is NOT #[non_exhaustive], so a future variant breaks compilation at every dispatch = fail-loud BY CONSTRUCTION going forward.

KEY CONTRAST with TASK-0360/0417: the compiler used Option/`_ => None` (silent-drop affordance, build_dataflow had the bug); the backends use EXHAUSTIVE matches (no affordance). Different structural safety profile — backends are safer by construction.

P3 fold-backs: (1) comment-doc-lie in worker_program.rs:160-166 FIXED (24ba0e2). (2) orchestrator preliminary "32 _ => arms" corrected to 34 (architect re-count).

NO production behaviour change (the one code edit was a comment reword; codegen byte-identical, e2e provably inert at the 385/328/0/57/0 baseline). qa N/A (no codegen delta); build/clippy clean + 6 doc fences OK after the comment fix.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Audited the backend emit surface (backend-common + 10 backends) for silent Event-variant drops. Architect-verified GO: STRUCTURALLY IMPOSSIBLE — all 5 main per-event render dispatches are exhaustive matches (zero `_ =>`), converging on two shared exhaustive walkers (event_walker.rs multi-worker + pthreads-sync render_event single-worker); MPI/openmp/event-plan all delegate. All 34 `_ =>` arms are selective collectors / expr renderers / loud guards, not dispatches. Event is not #[non_exhaustive] so a future variant fails compilation at every dispatch (fail-loud by construction). Contrast: the compiler used Option/`_ => None` (the build_dataflow silent-drop affordance); backends use exhaustive matches (no affordance). 1 P3 comment-doc-lie fixed (24ba0e2, worker_program.rs). No codegen change; e2e inert at 385/328/0/57/0.
<!-- SECTION:FINAL_SUMMARY:END -->
