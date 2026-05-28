---
id: TASK-0355
title: >-
  Unify is_whole_array_tile + is_whole_array_recv (TASK-0349 cycle 220b
  architect P3.1)
status: To Do
assignee: []
created_date: '2026-05-27 23:58'
labels:
  - backend-common
  - refactor
  - opacity-gate-rot-adjacent
  - cycle-220b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 architect P3.1: two whole-array-vs-slice classifiers exist in backend-common with similar but not identical semantics:

- collect.rs:260 `is_whole_array_tile` — predates TASK-0349; returns bool on empty bounds, scalar ty, full-range bounds; returns false on axis-beyond-dims (silently).
- wait.rs:369 `is_whole_array_recv` — TASK-0349 cycle 220; wraps `wait_slice` which has rank-guard Err arms, both-axes-full -> None, etc.

For the common shipped cases they agree; for edge cases they may diverge. This is **opacity-gate-adjacent** per memory feedback-opacity-gate-rot — the two classifiers can drift independently as one or the other evolves to handle new tile/data shapes.

## Acceptance

1. Pick one canonical classifier (likely `is_whole_array_recv` since it routes through wait_slice's shape-error invariants).
2. Migrate the call site of the other to route through the canonical one.
3. Remove the deprecated classifier.
4. Add a sibling test that exercises the edge-case shapes (rank > 2, out-of-bounds, scalar, empty bounds) to pin the unified semantics.

## Honest scope

Refactor; no behaviour change for shipped cells. Low priority because both classifiers happen to agree on every currently-shipped tile/data shape.
<!-- SECTION:DESCRIPTION:END -->
