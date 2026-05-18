---
id: TASK-0108
title: Exhaustive trait-surface tests for event types
status: To Do
assignee: []
created_date: '2026-05-18 01:15'
labels:
  - M1
  - events
  - tests
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0015's tests cover Debug-by-use, Clone-by-use, PartialEq, Hash, serde. They do not explicitly assert Send/Sync/Ord-on-newtypes/Default. Add compile-time assert_impl_all-style tests so a future derive deletion breaks the build. Reference: TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 static_assertions / manual trait-bound tests for Send/Sync on Event and IterTile;Ord on each newtype id;Default on IterTile
<!-- AC:END -->
