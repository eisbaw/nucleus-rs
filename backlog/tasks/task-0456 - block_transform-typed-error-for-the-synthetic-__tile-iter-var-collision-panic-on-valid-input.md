---
id: TASK-0456
title: >-
  block_transform: typed error for the synthetic __tile iter-var collision
  (panic on valid input)
status: To Do
assignee: []
created_date: '2026-06-09 21:59'
labels:
  - panic-not-diagnostic
  - compiler
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P1.1), verified verbatim in the working tree: passes/block_transform.rs:349-354 panics when a user algorithm declares an iteration variable literally named <var>__tile while the schedule puts block=N on loop <var> — a valid (if obscure) program, hitting panic! instead of a diagnostic. The pass already has a typed BlockTransformError surface and driver mapping, so this is a routing fix, not new machinery. Optionally mangle the synthetic name instead of erroring; either way the outcome must be deterministic and loud.

Recurring class: feedback-panic-not-diagnostic-recurring. While in the file, grep remaining panic!/unwrap/expect on user-reachable paths and justify or fix each.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Collision produces a typed BlockTransformError naming both variables (or a documented mangle policy), never a panic
- [ ] #2 Negative test compiles such a program end-to-end and pins the diagnostic
- [ ] #3 Remaining panic-class sites in block_transform.rs audited: fixed or justified-unreachable, list in notes
<!-- AC:END -->
