---
id: TASK-0102
title: 'Contract pass: amortise rustc invocation cost'
status: Done
assignee: []
created_date: '2026-05-18 00:53'
updated_date: '2026-05-23 21:06'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). Description: 'Revisit when M3 lands and we have a wider example matrix to measure against. Options: (a) cache rmeta keyed on content hash; (b) batch... (c) accept the cost.' M3 has substantially landed (e2e 88/70/0/18 across 4 tier-1 backends); no contributor has complained about contract-check perf, AND the rationale's option (c) — 'accept the cost (a real cargo build wraps cargo check which is already ~seconds, so net wash)' — appears to be the effective state. Reopen if a real perf complaint surfaces against contract-check (measured, not speculated).
<!-- SECTION:FINAL_SUMMARY:END -->
