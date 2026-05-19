---
id: TASK-0186
title: 'e2e bin: empty-line-after-doc-comment clippy lint fails under --all-targets'
status: To Do
assignee: []
created_date: '2026-05-19 04:26'
labels:
  - compiler
  - tooling
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
cargo clippy --workspace --all-targets -- -D warnings fails on nucleus/e2e (commented-out doc comment block ~line 2253: empty line after doc comment). Pre-existing on clean master, NOT introduced by TASK-0154. The project gate (just clippy / just ci) does NOT pass --all-targets so it is currently green, but --all-targets clippy (test targets) is broken. Fix: convert the //  /// commented block to a plain // comment with no blank line, or #[allow]. Discovered during TASK-0154.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 cargo clippy --workspace --all-targets -- -D warnings is clean
- [ ] #2 just ci still exit 0
<!-- AC:END -->
