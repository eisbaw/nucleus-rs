---
id: TASK-0394
title: >-
  Hygiene: add tmp/ to .gitignore so doc-lie/structural fences do not scan
  scratch (cycle-231 architect P3)
status: To Do
assignee: []
created_date: '2026-06-01 00:53'
labels:
  - tooling
  - ci
  - hygiene
  - gitignore
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-231 architect-review P3 (a89db02). tmp/ is untracked but NOT gitignored, so ripgrep scans it -- meaning ALL the check-* fences that scan '.' with only -g '!target/**' (check-doc-citation-staleness / -bare / check-doc-test-name-staleness / check-narrative-doc-lie etc.) also scan tmp/. A scratch .rs dropped in tmp/ can RED 'just ci' for the developer (architect empirically reproduced: tmp/fence_test/inject.rs broke the new fence; qa reproduced the bite via tmp/qa_bite.rs). Pre-existing shared footgun across the whole fence family, NOT introduced by any one fence. Fix: add 'tmp/' to .gitignore (the project's scratch dir per CLAUDE.md cruft/scratch conventions). Verify no fence or recipe relies on tmp/ being scannable first. Low risk, removes the footgun for the whole family at once.
<!-- SECTION:DESCRIPTION:END -->
