---
id: TASK-0075
title: Wire up nucleus-e2e matrix runner (replaces stub)
status: To Do
assignee: []
created_date: '2026-05-17 23:36'
labels:
  - M1
  - infra
  - tooling
  - testing
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
At M0 `nucleus-e2e` is a stub `fn main() {}`. `just e2e` therefore exits 0 with no work done, which is misleading once the first backend lands at M1.

This task wires up the real differential test matrix per PRD §10.1:

- Iterate every (example, required schedule, supporting backend) triple from PRD §9 / §7.
- Compile each cell via the `nucleus` pre-compiler.
- Run each compiled binary with the example's `input.bin`.
- Diff against the example's `reference.bin` and assert bit-identity (tier 1).
- Aggregate red/green status per cell, exit non-zero on any red.
- CLI flags to filter by example / schedule / backend (the anti-bloat replacement for `just run-stencil-on-pthreads-sync` style one-offs from PRD §12.3).

Cannot start until at least one backend exists (M1, TASK for pthreads-sync). Until then `just e2e` is a no-op success.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nucleus-e2e iterates a non-empty matrix and runs at least one example end-to-end
- [ ] #2 Tier-1 cells assert bit-identical output vs reference.bin
- [ ] #3 CLI flags allow filtering by example, schedule, backend
- [ ] #4 Exit non-zero if any cell is red
- [ ] #5 just e2e wraps it without arg pass-through changes
<!-- AC:END -->
