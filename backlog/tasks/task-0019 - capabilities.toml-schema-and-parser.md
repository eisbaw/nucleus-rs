---
id: TASK-0019
title: capabilities.toml schema and parser
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §7.4: each backend declares its capabilities as a sibling text file. Define the schema and write a parser; the schedule-vs-backend compatibility check uses this.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/capabilities-toml.md describes the schema: transport, notify, supports_async, supports_buffer, max_buffer, worker_classes, memory_regions.
- [ ] #2 compiler crate parses capabilities.toml from each backend crate's root.
- [ ] #3 Mismatch between schedule demand and backend capability is a compile-time error with the offending field named.
- [ ] #4 Test: every backend's capabilities.toml round-trips through the parser.
- [ ] #5 Test: a curated set of schedule/backend pairs is checked; expected acceptances and rejections both verified.
- [ ] #6 Implementation notes record design questions (e.g. how to express forward-compatible capability extensions).
- [ ] #7 Implementation notes record honest limitations (e.g. schema cannot express conditional capabilities like 'async only when buffer>=2').
<!-- AC:END -->
