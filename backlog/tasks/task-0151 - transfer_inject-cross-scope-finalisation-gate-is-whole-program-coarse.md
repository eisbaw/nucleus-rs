---
id: TASK-0151
title: 'transfer_inject: cross-scope finalisation gate is whole-program coarse'
status: To Do
assignee: []
created_date: '2026-05-18 08:32'
labels:
  - M2
  - compiler
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pass A/B (TASK-0136) are gated on inner_block_iter_vars.is_empty() — a whole-PROGRAM switch. A program mixing one block=N loop with an unrelated non-blocked cross-scope whole-symbol transfer gets ZERO cross-scope finalisation, silently reintroducing the original deadlock for the non-blocked part. No example hits this today (single-schedule programs). Tighten to per-subtree scoping, and add a log::debug! on the skipped branch so the deferral is traceable instead of invisible. Raised by mped-architect review of TASK-0136.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Gate decision is per-Repeat-subtree, not whole-program
- [ ] #2 Skipped-finalisation branch logs a traceable debug message naming the deferred symbol/seq
- [ ] #3 Test: mixed block + non-block program still pairs the non-block cross-scope transfer
<!-- AC:END -->
