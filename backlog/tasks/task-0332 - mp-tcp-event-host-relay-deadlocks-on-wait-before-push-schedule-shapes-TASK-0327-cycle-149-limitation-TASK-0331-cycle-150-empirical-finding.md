---
id: TASK-0332
title: >-
  mp-tcp-event host-relay deadlocks on wait-before-push schedule shapes
  (TASK-0327 cycle-149 limitation, TASK-0331 cycle-150 empirical finding)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 18:52'
updated_date: '2026-05-25 19:50'
labels:
  - M6
  - backend
  - mp-tcp-event
  - host-relay
  - forward-carried-from-TASK-0327
  - deadlock
dependencies:
  - TASK-0327
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0327 cycle 149 landed mp-tcp-event's worker-to-worker `Push`/`Wait` via synchronous host-relay. The relay splice heuristic (`relay_phase_insertion_point`) places the relay block just BEFORE the LAST top-level `Event::Sync` — between pass-1 barrier and pass-2 barrier on the 06/distributed2 reproducer. This works for SCATTER-COMPUTE-GATHER schedule shapes where each worker pushes BEFORE waiting on w2w pairs.

## Cycle 150 (TASK-0331 AC#2) empirical finding

Promoting `05-stencil/distributed-2d × mp-tcp-event` to `[[required]]` and running `just e2e` produced a deadlock at 32.4s (workers exit code 0 but run.sh reports failure — timeout-shape).

Root cause (verified by inspecting cycle-150 e2e scratch dir's emitted code):

- Each worker's events begin with `chan_X.wait()` calls for cross-worker halo strips BEFORE any push:
  ```
  // w0.rs (emitted by mp-tcp-event for 05/distributed-2d):
  chan_4.wait();  // recv `img_in` from w2
  chan_5.wait();  // recv `img_in` from w1
  chan_7.push(img_in.clone()); // send `img_in` to w1
  chan_8.push(img_in.clone()); // send `img_in` to w2
  bar_0.wait();  // <-- worker reaches this only after the waits
  ```
- Cycle-149's host-relay block splices AFTER `bar_0.wait()` (host.rs line ~226-241).
- DEADLOCK CHAIN: w0 blocked at chan_4.wait() (waiting for w2's push via host relay). w2 symmetrically blocked. w0..w3 never reach bar_0. Host blocks at bar_0 forever (waiting for the 4 workers). Host never runs the relay phase. The w2w pushes never get forwarded.

## What works today

`05-stencil/distributed-2d × pthreads-async` PASSES bit-identical because pthreads-async has direct worker-to-worker ring buffers (no host mediation needed for the w2w pushes). The schedule's wait-before-push order is correct under the assumption of direct w2w channels.

## Acceptance criteria

### AC#1: relax the host-relay scheduling model

The cycle-149 design's scatter-compute-gather timing model needs to be replaced (or supplemented) by one of:

- **(A) Threaded host-relay**: spawn a thread in host's main that polls the reactor for inbound w2w frames and forwards them concurrently with host's own work. The CTRL barrier still works because it's on a separate stream.
- **(B) Interleaved host-relay**: instead of a single relay block, emit per-Push site interleaving — for each w2w Push event, host immediately runs `relay_one(seq, dst_peer, cap)` when its inbound[seq] arrives. Requires re-modeling host's event list to include relay events.
- **(C) Pre-bar_0 relay**: detect schedule shape ahead of time. If any worker's first w2w event is a Wait, splice the relay BEFORE bar_0 instead of after — or splice multiple times. Requires schedule-shape analysis.

(A) is the most general but adds a thread to the runtime; (B) is the cleanest but invasive in the emit walker; (C) is narrowly fix-but-only-for-known-shapes.

### AC#2: defensive ContractGap or auto-detect at Plan::build

Until AC#1 is landed, the cycle-149 host-relay should DETECT a wait-before-push hazard at `Plan::build` and fail-loud with EmitError::ContractGap forward-linking THIS task. The shape to detect: any non-host worker whose first top-level w2w `Event::Wait` precedes any top-level w2w `Event::Push` on the same worker's event list. Fail-loud > silent deadlock.

### AC#3: regression pin

Once AC#1 lands, promote `05-stencil/distributed-2d × mp-tcp-event` to `[[required]]` in `nuc-nucleus/e2e-matrix.toml`. Verify bit-identical against `05-stencil/reference.bin` (same oracle as the pthreads-async sibling that already passes).

## Honest scope

- **Distinct from TASK-0330**: TASK-0330 covers w2w Push events INSIDE Loop bodies. This task covers wait-before-push event order at TOP LEVEL. Both share the cycle-148/149 host-relay assumption set but are different defect classes.
- **Distinct from TASK-0329**: TASK-0329 covers CTRL barrier mediation for host-excluding barriers. This task covers DATA-arm wait-before-push under host-INCLUDING barriers.
- **Priority MEDIUM**: in-tree trigger exists (05/distributed-2d × mp-tcp-event). The cycle-150 empirical finding turned this from theoretical-defensive into active-correctness-gap. mp-tcp-bufsync has the same architecture but is currently capability-gated on this specific cell (TASK-0042 async + buffer=2 + notify=event), so the defect surface is mp-tcp-event-only today.
- **AC#2 defensive ContractGap is the minimum cycle-N closure**; AC#1 full fix is a substantial future cycle.

## Cross-reference

- TASK-0327 (cycle 148/149): the host-relay design this task identifies a hole in.
- TASK-0330: Loop-body sibling-defect (cycle-148 architect P3.2).
- TASK-0329: CTRL-arm barrier mediation.
- TASK-0331 (cycle 150): the audit task that empirically surfaced this finding.
- `nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc`: the in-tree trigger schedule.
- `nucleus/target/e2e-matrix/run-*/05-stencil__distributed-2d__mp-tcp-event/src/bin/{host,w0}.rs`: the emitted code showing the deadlock-shape event order.

## Memory cross-reference

- `feedback-orchestrator-narrative-also-wrong` — the cycle-149 narrative "remaining blocker is TASK-0294" was algebra-imprecise; cycle-150 empirical test corrected it.
- `feedback-silent-sibling-defect` — TASK-0330 was filed cycle 148 for a SIBLING defect class; this task is the in-tree-trigger defect class that cycle-150 originally MIS-attributed to TASK-0330.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Forward-carried from TASK-0330 (cycle 153)

When AC#1 lands one of the (A) threaded / (B) interleaved / (C) pre-bar_0 host-relay scheduling alternatives, the cycle-153 "RESOLVED by TASK-0330" comment in `detect_wait_before_push_hazard` (both backends, ~line 1740-1750 mp-tcp-bufsync and ~1146-1157 mp-tcp-event) needs re-verifying. The composition assumption is: a Loop-body w2w Push CANNOT silently reach codegen because `collect_w2w_pushes` runs from `render_relay_phase` which is on the only host emit path. If AC#1's new design diverges from that call graph (e.g. host emits relays piecewise inside the per-event loop), the TASK-0330 guard's downstream-fire timing may need to be moved up to `Plan::build` as a recursive walker — or the composition claim must be re-validated against the new emit path.

Audit step for the AC#1 implementer: `grep -rn render_relay_phase` and `grep -rn collect_w2w_pushes` in both backends; verify the TASK-0330 guard still fires on every code-emitting path before claiming this task closes.
<!-- SECTION:NOTES:END -->
