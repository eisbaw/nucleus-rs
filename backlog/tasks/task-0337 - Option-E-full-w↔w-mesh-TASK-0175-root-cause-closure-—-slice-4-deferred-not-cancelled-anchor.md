---
id: TASK-0337
title: >-
  Option E full w↔w mesh (TASK-0175 root-cause closure) — slice 4
  deferred-not-cancelled anchor
status: To Do
assignee: []
created_date: '2026-05-26 07:24'
labels:
  - M6
  - backend
  - mp-tcp-event
  - mp-tcp-bufsync
  - w2w-mesh
  - deferred
  - forward-carried-from-TASK-0329.01
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle 161b documented (D)+(B2)+(A-deferred)+(E-deferred) as the four-option menu for the host-relay redesign. Slice 1 (Option D, TASK-0329.01.01 cycle 162) + slice 2 (Option B2, TASK-0329.01.02 cycle 163 + TASK-0329.01.02.01 cycle 165) LANDED the (D)+(B2) composition, closing TASK-0329.01 + TASK-0332 cycle 166. Per the cycle-166b architect review:

> Option E (full w↔w mesh, the root-cause TASK-0175 closure) remains deferred-not-cancelled. The compensating-pass tower (cycle-149 host-relay → cycle-151 defensive guard → cycle-162 reorder → cycle-163 ACFG relay-inject → cycle-165 13-arm) IS workaround-shaped on top of TASK-0175. The honest cost signal: 4+ cycles, 3 passes, 2 backends asymmetric, multiple residual-class disclaimers. This is approaching the threshold where slice-4 SHOULD be filed even without a new trigger cell, on architectural-debt grounds alone.

Per CLAUDE.md 'NEVER implement workarounds — fix the root cause', the cumulative compensating-pass model is the de-facto root path for any future host-excluding-barrier + in-loop-w2w-Push schedule shape on a TCP backend. Option E (direct worker-to-worker TCP connection mesh, eliminating host as a forwarding hub) would obsolete: the cycle-148/149 host-relay, the cycle-151 defensive guard, the cycle-162 reorder, the cycle-163 ACFG relay-inject, AND the cycle-166-named residual classes (R-bare) and (R-singleton). Net subtract LOC, not add.

## Honest scope

LOW priority TODAY. No active trigger cell — all 3 cycle-166-target trigger cells (05/distributed-2d, 09/pipelined, 13/pipeline_parallel × mp-tcp-event) are [[required]] bit-identical via the (D)+(B2) composition. This task is the VISIBLE-DEBT ANCHOR so the deferral doesn't become invisible.

## Promotion triggers (when to escalate)

Any of:
- A new schedule shape that needs a 5th compensating pass on top of the existing tower.
- A test failure that root-causes to host-relay timing rather than a missing pass.
- A backend-2-asymmetry case where bufsync NEEDS the analogue but the per-pair FIFO constraint blocks it (i.e. mp-tcp-bufsync can't even add a slice-1-equivalent).
- An NTECH-EMEA / publication context where 'this design has 5 compensating passes for one defect' becomes a credibility hit.

## Acceptance criteria

### AC#1: design analysis

A cycle (~1 day) of design work analogous to cycle 161's four-option menu but for the slice-4 surface: enumerate the runtime + protocol options (direct w-to-w sockets via mio per-pair, or a mesh per cluster, or some hybrid); evaluate each against memory project-mp-tcp-event-vs-bufsync-safety-profile (per-pair FIFO vs per-seq-demux) and the cycle-160 host_mediation_inject pass (which would become unnecessary for w2w but is needed for non-w2w-related host-excluding barriers).

### AC#2: implementation

Per the AC#1 chosen option.

### AC#3: cleanup

Delete the now-dead compensating passes (apply_safe_push_reorder, apply_host_data_relay_inject) and the defensive guards (TASK-0330 collect_w2w_pushes, TASK-0332 detect_wait_before_push_hazard). Update the cycle-166-reframed docstrings to point to the slice-4 implementation. Verify e2e numbers UNCHANGED (the compensating-pass tower's behavior is fully replicated by the mesh).

### AC#4: paired-lift to mp-tcp-bufsync (if architecturally feasible)

If the slice-4 design ALSO unlocks bufsync's analogous capabilities (currently asymmetric per cycle 162), promote any bufsync-skip cells gated on TASK-0175 + TASK-0117 mixed reasons.

## Cross-reference

- TASK-0329.01 cycle 161b (the four-option menu).
- TASK-0329.01 cycle 166 (closing audit naming the (D)+(B2) composition).
- TASK-0175 (parent — the original w↔w mesh task).
- CLAUDE.md 'NEVER implement workarounds — fix the root cause'.
- Cycle-166b architect P3-Option-E recommendation (this filing).

## Forward-carried discipline

Per memory feedback-opacity-gate-rot: if this task SITS in To Do for &gt;6 months without being promoted, audit whether the compensating-pass tower has accumulated MORE machinery (a 5th, 6th pass) — that is the real promotion trigger, not a calendar date.
<!-- SECTION:DESCRIPTION:END -->
