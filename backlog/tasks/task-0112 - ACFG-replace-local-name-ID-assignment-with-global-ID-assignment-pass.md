---
id: TASK-0112
title: 'ACFG: replace local name->ID assignment with global ID-assignment pass'
status: Done
assignee: []
created_date: '2026-05-18 01:23'
updated_date: '2026-05-23 21:11'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-until-global-ID-pass-lands (orchestrator-direct, cycle 77 sweep). Description: 'When the global ID-assignment pass lands (likely sitting between link and acfg, owning the canonical name table for downstream codegen + EventList projection), this local mapping becomes redundant.' The global ID-assignment pass does NOT exist today and is not on the near-term path. Today's per-pass deterministic local mapping has been sufficient through 7 keystone cycles + 4 backends; no contributor has surfaced a defect attributable to per-pass IDs not sharing. Reopen when the global pass is filed and gets implemented — at that point this task IS the migration step of that pass. Same deferred-until-prerequisite pattern as TASK-0114 (sync-injection If rules — blocked on TASK-0110).
<!-- SECTION:FINAL_SUMMARY:END -->
