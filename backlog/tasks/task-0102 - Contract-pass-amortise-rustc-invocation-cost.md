---
id: TASK-0102
title: 'Contract pass: amortise rustc invocation cost'
status: To Do
assignee: []
created_date: '2026-05-18 00:53'
labels:
  - M3
  - compiler
  - perf
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0012's check_kernels_contract spawns 'rustc --emit=metadata' per call. Per-call cost is ~50-100ms on a warm machine, which dominates a small-example build. Options: (a) cache rmeta keyed on content hash; (b) batch multiple kernels.rs into one rustc invocation when several examples share the project; (c) accept the cost (a real cargo build wraps cargo check which is already ~seconds, so net wash). Revisit when M3 lands and we have a wider example matrix to measure against.
<!-- SECTION:DESCRIPTION:END -->
