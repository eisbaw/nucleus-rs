---
id: TASK-0101
title: 'Contract pass: static purity check (likely won''t-fix in v2)'
status: Done
assignee: []
created_date: '2026-05-18 00:52'
updated_date: '2026-05-23 20:56'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-v3 (orchestrator-direct, cycle 77 sweep). The task is labeled 'v3, compiler, research' and the description's recommendation is explicit: 'leave (a) and document explicitly in user docs. This task should either confirm won't-fix v2 or pick a path.' Confirming WON'T-FIX-v2: the pure/effectful contract remains a user-upheld convention enforced by code review and convention (PRD §6.2.2), not by static analysis. The alternatives (denylist syn walk, const fn coercion, custom rustc plugin) all carry costs disproportionate to v2's needs. Reopen for v3 if/when the project takes on production-grade purity guarantees.
<!-- SECTION:FINAL_SUMMARY:END -->
