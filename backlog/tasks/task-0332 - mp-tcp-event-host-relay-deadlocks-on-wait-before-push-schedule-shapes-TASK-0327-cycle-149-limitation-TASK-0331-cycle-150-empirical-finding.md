---
id: TASK-0332
title: >-
  mp-tcp-event host-relay deadlocks on wait-before-push schedule shapes
  (TASK-0327 cycle-149 limitation, TASK-0331 cycle-150 empirical finding)
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-25 18:52'
updated_date: '2026-05-25 18:55'
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
## Cycle-150 architect P1 fold-back — root cause is more precise than wait-before-push

The cycle-150 architect read-only review (`mped-architect` P1) provided a more precise mechanism for the deadlock than this task's initial filing described. Folding it back here so the task captures both the immediate framing and the underlying defect:

### The architect's precise diagnosis (mechanism)

Even if `bar_0`'s sync timing were not a factor, the cycle-149 host-relay's sequential ordering creates a CIRCULAR SEQ DEPENDENCY. Empirically (`nucleus/target/e2e-matrix/run-96488-.../05-stencil__distributed-2d__mp-tcp-event/src/bin/host.rs` lines ~232-240):

```
__relay.relay_one(11u64, 1usize, 2usize); // relay `img_in` from w0 to w1
__relay.relay_one(12u64, 2usize, 2usize); // relay `img_in` from w0 to w2
__relay.relay_one(9u64,  0usize, 2usize); // relay `img_in` from w1 to w0
__relay.relay_one(14u64, 3usize, 2usize); // relay `img_in` from w1 to w3
__relay.relay_one(8u64,  0usize, 2usize); // relay `img_in` from w2 to w0
__relay.relay_one(15u64, 3usize, 2usize); // relay `img_in` from w2 to w3
__relay.relay_one(10u64, 1usize, 2usize); // relay `img_in` from w3 to w1
__relay.relay_one(13u64, 2usize, 2usize); // relay `img_in` from w3 to w2
```

Host's relay STARTS with `relay_one(11)` = wait(seq=11) + push to w1. wait(seq=11) blocks until w0 has pushed seq=11. But w0's first events are `wait(chan_4 = seq=8)` from w2 — which is host's 5th relay hop (not yet reached). w0 cannot push seq=11 until it has crossed its initial waits. Circular dependency: host waits for w0's push, w0 waits for host's 5th relay hop, host's 5th hop can only run after host's 1st hop completes.

This pattern was DISCLOSED in cycle-148's TASK-0327 implementation plan ("host cannot have data reads BETWEEN scatter and gather... complex interleaved schedules would need either threaded relay or scheduled relay events") but was not filed as a separate task until cycle 150 found the empirical trigger.

### Two-level framing (both correct, different layers)

- **Surface layer (the obvious bar_0 deadlock)**: workers are blocked at their initial w2w `chan.wait()` calls; they never reach `bar_0.wait()`; host's `bar_0.wait()` blocks until all participants (including workers) cross it; host never runs the relay phase. This is what the initial task description captured.
- **Underlying defect (the architect's precise diagnosis)**: even if `bar_0` were not a factor, the sequential-ordered relay phase has a circular seq dependency. This is the actual constraint that AC#1's threaded / interleaved / per-Push-arrival reactive relay must fix.

Both layers point at the same architectural fix set (AC#1 (A)/(B)/(C)). AC#2 (defensive ContractGap detecting wait-before-push at Plan::build) catches the surface symptom; a more thorough detection would also enumerate the seq DAG and check for cycles — likely overkill at codegen, simpler to fix-with-AC#1 and remove the detection.

### Triggers two recurring-pattern memory entries

- **feedback-implementer-disclosure-mechanism-wrong**: cycle-148's implementation plan disclosed this limitation HONESTLY but with a vague "complex interleaved schedules" wording. Cycle-150 surfaced it as the precise mechanism. Memory note candidate update: vague disclosures in implementation plans should be filed as defensive tasks AT IMPLEMENTATION TIME, not deferred to the in-tree trigger.
- **feedback-orchestrator-narrative-also-wrong**: the cycle-150 first re-attribution (TASK-0330 Loop-body) was also a wrong orchestrator-written narrative; the cycle-149 first attribution (TASK-0294 slice-paste) was also wrong. Three wrong-attributions before the right one. The empirical-verification step (read the emitted code) is the safety net; without it, narrative-rot iterates ad infinitum.

### AC#1 implementation guidance (extends original task brief)

The architect's diagnosis sharpens the AC#1 design choice:
- (A) Threaded host-relay: separate thread runs `relay_one` calls indefinitely, polling inbound for any seq that arrives, forwarding to the right dst. Solves both the bar_0 timing AND the sequential-ordering issue.
- (B) Interleaved host-relay: emit per-seq-arrival hooks in host's main; uses the reactor's pump_once loop. Less code than (A) but requires walker integration.
- (C) Pre-bar_0 relay (rejected): does NOT solve the sequential-ordering issue; only solves the bar_0 timing. Insufficient.

(A) is the recommended path; (B) is the more invasive but cleaner alternative. (C) is rejected by the architect's diagnosis.
<!-- SECTION:NOTES:END -->
