---
id: TASK-0455
title: 'Production usefulness push: from falsification rig to usable tool'
status: To Do
assignee: []
created_date: '2026-06-09 21:28'
labels:
  - epic
  - production
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
EPIC. Decision (2026-06-09, post-thesis v0.1): push Nucleus toward production usefulness — the user intends to use it for real work across a diverse set of platforms. The thesis established correctness/falsifiability; this epic removes the walls between that and a usable tool.

Ordered by leverage:
1. Lift the communicating-case soundness-gate scaling wall (keystone: blocks compiling realistic problem sizes for ANY distributed schedule; thesis sec:res-quant-net / sec:fw-quant state the coupling).
2. Wire-level precise transfer = TASK-0453.22 (raised to HIGH under this push): stop broadcasting whole arrays on every cross-worker edge.
3. Honest runtime perf + scaling study once 1+2 land.
4. Bring first-class IO to the embedded tier where it matters most: async + buffered (double-buffered DMA) + IRQ-event notify (today embedded caps at sync/buffer=1/blocking, so the headline IO contribution is unrealized on the motivating tier).
5. One non-toy case study at realistic size = the production witness.
6. Multi-worker bounded break (S7 = TASK-0341.02.01.08, raised to MEDIUM): admits distributed iterative solvers.
7. Widen the generative differential harness beyond 1-D elementwise.
8. Onboarding + diagnostics hardening for a first external-style user.

Per-platform shim intake tasks get filed once the concrete target platform list is named by the user (AC#3).

Discipline carried over from TASK-0453: strengthen ACTUAL capability, never relabel; honest-stop over fabrication; every claim codebase-verified; just ci GREEN and e2e baseline held every cycle.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Every child task is closed or explicitly re-scoped with user sign-off
- [ ] #2 A distributed schedule at production-relevant problem size compiles and runs within commodity memory (gate included)
- [ ] #3 Target platform list collected from user; per-platform shim intake tasks filed
- [ ] #4 e2e + just ci stay green throughout; tier-1 baseline never regresses
<!-- AC:END -->
