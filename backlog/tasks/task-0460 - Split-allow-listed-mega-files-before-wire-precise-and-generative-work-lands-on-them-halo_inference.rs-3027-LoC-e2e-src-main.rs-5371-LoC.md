---
id: TASK-0460
title: >-
  Split allow-listed mega-files before wire-precise and generative work lands on
  them: halo_inference.rs (3027 LoC) + e2e/src/main.rs (5371 LoC)
status: To Do
assignee: []
created_date: '2026-06-09 22:01'
labels:
  - hygiene
  - refactor
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P2.8). Both files sit on the mega-file fence allow-list (justfile:1216-1237) exactly where upcoming epic work lands: TASK-0453.22 / TASK-0455.07 extend halo/tile inference, and TASK-0455.05 extends the e2e harness. Split along docstring seams BEFORE that work starts, so the carve is content-preserving rather than entangled with semantic changes.

Discipline: split-dont-allow-list (memory feedback-cheap-subset-blind-to-structural-fences — TASK-0383 precedent where an allow-listed file sat RED for cycles); content-preserving carve per the TASK-0437/.01 precedent; re-grep bare-filename references and classify per-hit (memory feedback-carve-out-bare-filename-deixis-double-classification); doc-citation fences must pass post-move.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Both files under the fence limit; their allow-list entries removed from the justfile
- [ ] #2 Carves content-preserving: pure mod moves, production behaviour unchanged (e2e + just ci green)
- [ ] #3 Doc-citation fences pass post-move; stale path/filename references swept and classified
<!-- AC:END -->
