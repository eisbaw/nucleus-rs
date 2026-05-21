---
id: TASK-0220
title: Decide semantic translation of check loop V when V is strip-mined by block=N
status: Done
assignee: []
created_date: '2026-05-21 16:18'
updated_date: '2026-05-21 20:41'
labels:
  - compiler
  - real-time
  - M4
  - design
dependencies:
  - TASK-0052.02
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding #3 (TASK-0052.02 cycle): when a schedule has both loop V : block=N and check loop V : latency_max=T, block_transform tiles V into outer/inner; the inner Event::Loop carries block_tag.is_some(); inject_check_frames skips strip-mined inner loops by design, which would silently drop the user's assertion. TASK-0052.02 review-gate hardening rejects the combination at sched-lower (CheckOnStripMinedLoop) — fail-loud is the honest move. This follow-up is the design question: WHAT should the assertion mean post-strip-mine? Options: (a) per-source-iteration (= inner block-tile iteration) — attach frame to inner loop with semantic note; (b) per-tile (= D source iterations, one outer-tile iteration) — attach frame to outer block loop; (c) keep rejecting at sched-lower (current state) — but rephrase the diagnostic to point at the orthogonal future decision. PRD §6.3.5 says latency_max is 'wall-clock duration of one iteration'; 'one iteration' is ambiguous after strip-mining. Pick + document + test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Document the chosen semantic in PRD §6.3.5 (or in a clarifying note); update the CheckOnStripMinedLoop diagnostic accordingly.
- [ ] #2 Implement the chosen path (attach to inner / outer / keep rejecting); add positive test.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle (resolution: PATH C confirmed status quo)

AC#1 resolved with PATH C from the task description: keep rejecting at sched-lower via `SchedLowerErrorKind::CheckOnStripMinedLoop` (already implemented in TASK-0052.02 hardening cycle).

Rationale: PRD §6.3.5 says `latency_max` measures "wall-clock duration of one iteration". After block_transform tiles V into outer/inner, "one iteration" of the source variable V is ambiguous:
- (a) per-source-iteration = per-intra-tile-iteration: the inner Event::Loop's iter_var is reused as V; check_frame on it measures one intra-tile iteration (D source iterations span D Event::Loop iterations on the inner loop). Defensible but surprising for users who think of V as the source-level loop.
- (b) per-tile = per-outer-iteration: the outer block loop's iter_var is a SYNTHESIZED tile variable (not named V); attaching check_frame to it requires the user to think of "one tile iteration = N source iterations" — semantically a different latency budget.
- (c) keep rejecting (current state): user picks ONE — drop block=N to pipeline the full loop, OR drop check loop V if the strip-mined timing isn't the budget they care about.

No defensible canonical "what should this MEAN" exists per PRD framing. Adding either (a) or (b) without PRD update would lock in a guess. PATH C is the honest answer.

AC#2: implementation is path C = no code change required (the reject already exists at sched-lower via CheckOnStripMinedLoop landed in TASK-0052.02 hardening commit 2e7c8f1). The diagnostic message already names both actionable fixes ("Either remove the `block=N` option on `loop V` or remove the `check loop V` directive."). Closed.

If a real use case for the combination ever appears, this task can be re-opened to revisit (a) vs (b) with the use case as the design forcing function.
<!-- SECTION:NOTES:END -->
