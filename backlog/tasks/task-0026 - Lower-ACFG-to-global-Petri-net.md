---
id: TASK-0026
title: Lower ACFG to global Petri net
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 03:37'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Transform the post-injection ACFG into a global Petri net per PRD §8. Each acfg::operations becomes a transition; each xfer becomes a (push transition + place + wait transition) triple with the place carrying the buffer capacity.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 compiler exposes acfg_to_net(ACFG) -> Net.
- [ ] #2 Pipeline/reuse loop options translate to initial markings on the corresponding places.
- [x] #3 Buffer=N on a transfer translates to capacity=N on the corresponding place.
- [ ] #4 Test: each example schedule's net is dumped to DOT and snapshot-tested.
- [x] #5 Implementation notes record design questions (e.g. how to represent control-flow sync as net structure vs as a separate kind of place).
- [x] #6 Implementation notes record honest limitations (e.g. transfer aggregation may produce a coarser net than necessary).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions and decisions

**1. Encoding of `Repeat` (static-bounded loop) — unroll or parametric?**
Picked **unroll**: a `Repeat { range }` of length N emits N body-copies
of every transition inside. Reasons:
- Simpler. The walker is a tight depth-first recursion; the unroll is
  a `for _ in 0..N { walk(body) }` line.
- Lines up with per-worker control-place chain — each iteration just
  threads through the next slot. No special-case marking arithmetic.
- Matches what `EventList` projection (TASK-0027) needs: one event per
  iteration tile.

Cost: linear blow-up. Filed as TASK-0133 (parametric encoding) for
when analyses (TASK-0028/0029) reveal whether real examples scale N
high enough to bite. Examples 01/02/03 all have N <= 256; net sizes
sit comfortably in the 1000-element range.

**2. `Sync` — barrier place or barrier transition?**
PRD §8.2 mapping table gives both readings. Picked **single barrier
transition** consuming one token from each participant's current
control place and producing one for each. Reasons:
- Two structural counts smaller (no barrier place + arcs in and out;
  one transition + 2N arcs).
- Equivalent firing semantics for the v2 acyclic-firing-order
  restriction (PRD §8.4).
- Reads cleanly in DOT: "all participants gate this transition".

**3. `XferPolicy.buffer` (u64) -> `Place.capacity` (Option<NonZeroU32>)?**
Direct mapping. Clamp to u32::MAX (no example will hit this); panic
on buffer=0 (upstream invariant — the schedule grammar rejects
buffer=0). The `NonZeroU32` shape gives compile-time guarantee that
the production net is bounded.

**4. Per-worker control chain — why bother?**
PRD §8.4 demands statically determined firing order. The control
chain enforces per-worker linear order in the net's firing semantics:
worker w's k-th transition is enabled only after its (k-1)-th has
fired. Cross-worker firings (Sync, Push+Wait) layer on top to bind
inter-worker order. This is the structure the downstream
deadlock/boundedness analyses (TASK-0028/0029) walk.

**5. Distributed placements (`place k on {w0..w3}`)?**
Treated as one entity: the Operation transition has arcs to/from each
named worker's control chain. The transfer-injection pass already
collapses these the same way. The replication-per-worker story
belongs to a future partition pass (TASK-0016+).

## Honest limitations

- **Iteration unrolling explodes for large N.** A `Repeat` of length
  1_000_000 emits 1M transitions per body kernel. Filed as TASK-0133.
- **No pipeline-depth initial markings.** AC #2 unmet. The ACFG today
  carries `TransferPolicy { buffer }` but not `pipeline=D` or
  `reuse` annotations; once those propagate through the schedule IR
  into the ACFG, the buffer place's `initial_marking` should be
  pre-set to D. Filed as TASK-0134.
- **No DOT snapshot tests.** AC #4 unmet. The structural assertions
  cover place-count, transition-count, buffer-capacity, and
  determinism, but not the full graph topology. DOT snapshot wiring
  filed as TASK-0135.
- **Sync vs async distinguished only by buffer-place capacity.** The
  topology is the same; the *meaning* (does the producer block on
  push completion?) shows up in the EventList linearisation pass
  (TASK-0027). For Petri-net analysis purposes this is fine — the
  net captures the firing order, not the I/O coupling kind.
- **No cycle detection.** PRD §8.4 says \"acyclic global event DAG;
  cycle = deadlock\". This pass emits the net structure; the cycle
  check belongs to TASK-0029 (deadlock).

## Acceptance criteria

- AC #1 met: \`compiler::acfg_to_net(&ACFG) -> Net\` re-exported at the
  crate root and behind \`compiler::passes::acfg_to_petri::acfg_to_net\`.
- AC #2 NOT met (filed as TASK-0134).
- AC #3 met: \`TransferPolicy.buffer\` becomes the buffer place's
  \`Place.capacity\`. Verified by \`buffer_capacity_follows_policy_buffer_field\`.
- AC #4 NOT met (filed as TASK-0135). Structural assertions ship in
  \`tests/acfg_to_petri.rs\` instead.
- AC #5 met: design questions recorded above.
- AC #6 met: limitations recorded above.

## Follow-ups filed
- TASK-0133 — parametric repeat encoding.
- TASK-0134 — pipeline=D / reuse -> initial markings.
- TASK-0135 — DOT snapshot tests for ACFG->Petri output.
<!-- SECTION:NOTES:END -->
