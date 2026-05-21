---
id: TASK-0225
title: >-
  Move check_frame_codegen.rs from compiler/tests/ to
  backends/pthreads-sync/tests/ (close backwards dev-dep arrow)
status: To Do
assignee: []
created_date: '2026-05-21 20:42'
labels:
  - compiler
  - backend
  - tech-debt
dependencies:
  - TASK-0221
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0221 (b) deferral: nucleus/compiler/tests/check_frame_codegen.rs uses pthreads_sync::emit(), which requires the compiler crate to have a dev-dep on pthreads-sync (backwards dep arrow: pthreads-sync already depends on compiler at the regular-dep level; the test-only reverse adds an arrow that test-locks the compiler suite to the tier-1 backend). Moving the file to backends/pthreads-sync/tests/ (where multi_worker.rs lives) eliminates the backwards arrow. Pure refactor; no behavior change.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Move check_frame_codegen.rs to backends/pthreads-sync/tests/; update imports + Cargo.toml dev-dep on pthreads-sync REMOVED from compiler/Cargo.toml.
- [ ] #2 Gate: cargo test, e2e, clippy all green; test count unchanged (same tests, different location).
<!-- AC:END -->
