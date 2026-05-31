---
id: TASK-0372
title: >-
  Petri net soundness gate: make optional via CLI flag; split just e2e (cheap)
  vs e2e-full (all checks)
status: Done
assignee: []
created_date: '2026-05-30 20:53'
updated_date: '2026-05-31 00:11'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-218 orchestrator investigation: the premise (gate slows builds) is CONFIRMED, but the proposed solution (CLI flag + e2e cheap/full split) is a WORKAROUND for an O(T*A) performance bug, not a root-cause fix. Measured: 07-matmul/distributed8 gate-on 473ms vs gate-off 34ms (~439ms = 93% of build); root cause is Net::fire scanning ALL arcs per call (no per-transition adjacency index). Filed TASK-0377 as the root-cause near-linear fix (keeps the gate always-on per TASK-0368 defense-in-depth AND makes e2e fast). RECOMMENDATION: hold this task; if TASK-0377 brings the always-on gate cost under target, this flag/e2e-split is UNNECESSARY and 0372 should close as superseded. Only revive the flag if a residual cost remains after 0377.

CLOSED AS SUPERSEDED by TASK-0377 (cycle 218). This task proposed a CLI flag to disable the Petri gate + an e2e cheap/full split, to cure the cycle-217 gate slowdown. Investigation showed that was a WORKAROUND: the slowdown was an O(T*A) perf bug in Net::fire (all-arcs scan, no adjacency index). TASK-0377 fixed the root cause (per-transition ArcIndex -> near-linear gate, ~439ms -> ~21ms on the worst-case 8-worker net), so the gate now stays ALWAYS-ON (preserving TASK-0368 defense-in-depth) AND is cheap. The flag + e2e split are therefore UNNECESSARY. AC#1-#4 (the flag/recipe ACs) are deliberately NOT met and NOT ticked — the work was made moot, not done. Re-open only if a future residual gate cost re-appears that a root-cause fix cannot reach (none today: e2e 329/272/0/57/0 with the always-on gate).
<!-- SECTION:NOTES:END -->
