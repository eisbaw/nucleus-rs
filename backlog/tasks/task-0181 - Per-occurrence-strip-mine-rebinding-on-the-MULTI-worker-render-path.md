---
id: TASK-0181
title: Per-occurrence strip-mine rebinding on the MULTI-worker render path
status: To Do
assignee: []
created_date: '2026-05-19 02:04'
labels: []
dependencies:
  - TASK-0180
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0180 implemented per-occurrence absolute-index rebinding from Event::Loop.block_tag on the SHARED single-worker render path (pthreads-sync render_single_worker_main, which mp-tcp-bufsync also routes a 0/1-worker schedule through). The MULTI-worker renderers (pthreads-sync multi_worker.rs render_worker_events; mp-tcp-bufsync lib.rs multi-process loop arm) do NOT yet thread block_tag. If a strip-mined inner Event::Loop carrying a block_tag reaches them they now FAIL LOUD with a typed EmitError::ContractGap (refusing to emit the un-rebound loop, which would accumulator-double-count exactly like the TASK-0180 bug) rather than silently miscompile. No tier-1 schedule blocks a multi-worker loop so this is currently unreachable. This task threads the same tag-driven rebinding through the multi-worker path (expression renderers are already shared so it is one implementation, no drift) when a blocked multi-worker / distributed schedule lands.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A blocked MULTI-worker schedule strip-mining an inner loop rebinds each occurrence via block_tag on the multi-worker render path, byte-identical to the single-worker arithmetic
- [ ] #2 A synthetic blocked multi-worker accumulator schedule is bit-identical to its naive schedule on both backends
- [ ] #3 The multi_worker.rs / mp-tcp lib.rs block_tag.is_some() fail-loud guards are replaced by the actual rebinding; existing single-worker blocked cells 04/05/06/07 stay byte-identical-green
<!-- AC:END -->
