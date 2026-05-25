---
id: TASK-0329
title: >-
  host-mediated barrier mediation for host-excluding barrier shapes (TASK-0327
  cycle-148 architect P3.1)
status: To Do
assignee: []
created_date: '2026-05-25 17:40'
labels:
  - M6
  - backend
  - mp-tcp-bufsync
  - host-mediation
  - forward-carried-from-TASK-0327
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0327 cycle 148 lifted mp-tcp-bufsync's worker-to-worker Push/Wait rejection via host-mediated relay (host reads from data_<src>, forwards to data_<dst>). The sibling rejection at nucleus/backends/mp-tcp-bufsync/src/lib.rs:382-393 — host-excluding barriers (e.g. barrier participants = {w0..w3} with no host) — STILL fails LOUD with EmitError::ContractGap citing TASK-0175.

Cycle-148 architect P3.1 noted: this rejection is now STRUCTURALLY ANALOGOUS to the lifted data_conn_var rejection. Both are 'star topology forbids X' rejections; both have a host-mediation lift available (host injects itself as a mediating hub: host crosses with each non-host participant on ctrl_<peer>; each non-host worker crosses with host on ctrl_host).

## Honest exposure

LOW (dormant). No in-tree schedule produces a host-excluding barrier on mp-tcp-bufsync:
- 03-reduction/distributed × mp-tcp-bufsync would (workers do a reduction barrier excluding host), but it's currently SKIPPED on TASK-0175 — the same blocker this task would lift.
- 13-cnn-inference/batch_parallel × mp-tcp-bufsync also SKIPPED with mixed TASK-0175 + TASK-0117 reasons.

## Acceptance criteria

### AC#1: lift mp-tcp-bufsync host-excluding barrier rejection

In Plan::build, replace the rejection at lib.rs:382-393 with host-mediation injection:
1. For each barrier with participants not including host: ADD host to that barrier's participant set in the sidecar barrier_participants map.
2. Synthesize a corresponding Event::Sync on host's per-worker event list at the right ordering point (analogous to the cycle-148 relay-phase splice — likely between adjacent worker barriers).
3. The host's render_events Sync emit (existing barrier_cross loop) handles the injected participation transparently.

### AC#2: e2e cell promotion

Once AC#1 lands, promote 03-reduction/distributed × mp-tcp-bufsync from [[skip]] to [[required]] in nuc-nucleus/e2e-matrix.toml. Bit-identical against reference.bin.

### AC#3: defensive test fixture

Add a fixture exercising a host-excluding barrier shape; assert the barrier_cross emit is generated correctly on both host and non-host workers.

## Dependencies

- Builds on TASK-0327 cycle 148 (the cycle-148 splice/scheduling machinery is precedent).
- mp-tcp-event sibling lift is part of TASK-0327 cycle 149.

## Cross-reference

- nucleus/backends/mp-tcp-bufsync/src/lib.rs:382-393 (the rejection site).
- nucleus/backends/mp-tcp-bufsync/src/lib.rs:render_relay_phase / relay_phase_insertion_point (the analogous mediation precedent).
- TASK-0327 cycle-148 architect parallel-review P3.1 finding.

## Honest scope

LOW priority. Dormant defect (no in-tree schedule trips it on mp-tcp-bufsync today). Filed so the asymmetry surfaced by cycle-148's lift has a tracker anchor; promote when an actual schedule needs it.
<!-- SECTION:DESCRIPTION:END -->
