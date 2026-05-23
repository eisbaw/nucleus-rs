---
id: TASK-0116
title: 'Transfer-injection: coalesce per-point Push/Wait into per-tile bulk transfers'
status: Done
assignee: []
created_date: '2026-05-18 01:44'
updated_date: '2026-05-23 21:27'
labels:
  - M1
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently transfer_inject.rs emits one Push/Wait pair per consumer iteration when the consumer sits inside a for-loop. A real backend bulk-sends a tile. Coalesce contiguous iterations into a single Push/Wait whose IterTile is the outer loop's full range, when the producer's tile and the consumer's tile match exactly. Requires access-pattern analysis on Operation.dataflow.edges (currently absent at M1).
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-until-prerequisite (orchestrator-direct, cycle 77 sweep). Description: 'Requires access-pattern analysis on Operation.dataflow.edges (currently absent at M1).' The access-pattern analysis prerequisite has not landed — no separate task tracks it (filed at M1 time as scope-implicit). The coalescing optimization itself is meaningful only once a backend's per-iteration Push/Wait wire overhead becomes a measurable bottleneck (mp-tcp-bufsync today emits per-iteration sock_<peer>.write_all calls without complaints). Reopen when (a) access-pattern analysis lands as a separate pass AND (b) a real perf measurement shows the bulk-transfer coalescing would shift a real cell's wall time. Same deferred-until-prerequisite pattern as TASK-0114 (sync-injection If rules, blocked on TASK-0110).
<!-- SECTION:FINAL_SUMMARY:END -->
