---
id: TASK-0372
title: >-
  Petri net soundness gate: make optional via CLI flag; split just e2e (cheap)
  vs e2e-full (all checks)
status: To Do
assignee: []
created_date: '2026-05-30 20:53'
labels: []
dependencies: []
references:
  - passes/net_soundness.rs
  - TASK-0368
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 217 wired the Petri-net analyses (check_bounded / check_deadlock_free) into the production compile path via a new pass (passes/net_soundness.rs). Empirical observation during the cycle-217 review gate: an independent just e2e run did not complete within ~25-30 minutes vs the established baseline of ~5-7 minutes — a 3-5x slowdown. The Petri analyses run once per compiled (algorithm, schedule, backend) cell; the matrix is 322 cells × 7 tier-1 backends.

Always-on gate is paying for itself on every developer build + every CI run, even though structural enforcement (TtoP-arc elision + ACFG guards) already covered the shipping path. Costs the developer feedback loop and CI throughput; benefit is defense-in-depth + literal accuracy of PRD §8 framing.

Split the gate so the always-on default is cheap and the exhaustive run is opt-in:

- nucleus CLI: --net-soundness=on|off (default off?) or --no-net-soundness flag. Default to whichever matches developer ergonomics (likely off; defense-in-depth keeps the implementation honest but doesn't gate every build).
- justfile: e2e recipe runs cheap path (no net-soundness); new e2e-full recipe runs everything (net-soundness ON for every cell + any other expensive checks that may have accumulated).
- CI: matrix-test on the cheap e2e for every commit; nightly or pre-release runs e2e-full.

Acceptance:
- nucleus build accepts the flag and net_soundness pass is gated by it (not unconditional).
- just e2e completes in <10 min on this matrix size.
- just e2e-full runs every cell with all checks active; bit-identity still holds.
- README / docs note which recipe to use when.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nucleus build CLI accepts --net-soundness (or equivalent) flag controlling the new pass
- [ ] #2 just e2e completes in <10 min on the current matrix and does NOT run the Petri net-soundness pass
- [ ] #3 just e2e-full runs every cell with net-soundness pass enabled; bit-identity holds across the matrix
- [ ] #4 README / justfile help text documents when each recipe is appropriate
<!-- AC:END -->
