---
id: TASK-0112
title: 'ACFG: replace local name->ID assignment with global ID-assignment pass'
status: To Do
assignee: []
created_date: '2026-05-18 01:23'
labels:
  - compiler
  - ir
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
build_acfg currently re-derives KernelId/DataId/WorkerId/IterVar mappings from the LinkedIR locally. When the global ID-assignment pass lands (likely sitting between link and acfg, owning the canonical name table for downstream codegen + EventList projection), this local mapping becomes redundant. Migrate to consuming the global mapping. Until then, the local mapping is deterministic but does not share IDs across passes.
<!-- SECTION:DESCRIPTION:END -->
