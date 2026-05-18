---
id: TASK-0120
title: capabilities.toml schema_version field
status: To Do
assignee: []
created_date: '2026-05-18 01:58'
labels:
  - M3
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0019 follow-up: capabilities.toml has no schema_version field today, and the parser uses serde's deny_unknown_fields so any forward-incompatible field addition is rejected loudly. Add a top-level schema_version field (probably u32) that the parser reads first, then relaxes unknown-field handling per version-gated rules. This unblocks future capability additions without breaking older backend crates.
<!-- SECTION:DESCRIPTION:END -->
