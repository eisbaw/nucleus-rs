---
id: TASK-0101
title: 'Contract pass: static purity check (likely won''t-fix in v2)'
status: To Do
assignee: []
created_date: '2026-05-18 00:52'
labels:
  - v3
  - compiler
  - research
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.2.2: 'pure' vs 'effectful' is a contract the user upholds. Rust's type checker cannot prove a function is pure (no IO, no global mutation, no panicking arithmetic). Options: (a) leave as documentation only (current); (b) ban a denylist of std::* calls in pure kernel bodies via syn walk (brittle); (c) require pure kernels to be const fn (overly restrictive); (d) custom rustc plugin / dylint lint (large scope). Recommendation: leave (a) and document explicitly in user docs. This task should either confirm 'won't fix v2' or pick a path.
<!-- SECTION:DESCRIPTION:END -->
