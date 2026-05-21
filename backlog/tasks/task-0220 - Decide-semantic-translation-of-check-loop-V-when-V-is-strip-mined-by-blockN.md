---
id: TASK-0220
title: Decide semantic translation of check loop V when V is strip-mined by block=N
status: To Do
assignee: []
created_date: '2026-05-21 16:18'
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
