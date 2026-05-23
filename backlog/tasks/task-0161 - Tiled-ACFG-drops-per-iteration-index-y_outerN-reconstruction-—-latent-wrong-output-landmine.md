---
id: TASK-0161
title: >-
  Tiled ACFG drops per-iteration index (y_outer+N reconstruction) — latent
  wrong-output landmine
status: Done
assignee: []
created_date: '2026-05-18 22:06'
updated_date: '2026-05-23 21:59'
labels:
  - M3
  - compiler
  - correctness
  - latent
dependencies:
  - TASK-0142
  - TASK-0159
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0142 (P2). The static trailing-remainder decomposition is iteration-count/order faithful for the unrolling consumers (acfg_to_petri/petri_to_events/boundedness/deadlock) and harmless TODAY because 05-stencil/blocked is single-host so the backend emits from LinkedIR::algo source, not the tiled ACFG. But the decomposition does NOT reconstruct the original per-iteration index value: untiled runs y=1..15; the decomposition yields inner indices 0..4 (full tiles) and 0..2 (tail) under a tile var, with no y_outer*N+inner+LO mapping and no min(y_outer+N,H) clamp (PRD §6.3.3). The moment an index-SENSITIVE consumer reads the tiled ACFG — a real EventList-driven backend (TASK-0124), or per-tile transfer codegen — this produces WRONG numeric output, silently. Make index reconstruction explicit on the tiled ACFG (carry tile origin + width so a consumer can compute the true index), OR have the EventList loop-structure contract (TASK-0159) carry it. Add a test that an index-sensitive consumer gets correct indices across the partial tile.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Tiled ACFG (or its Event projection) lets a consumer recover the true per-iteration index incl. the trailing partial tile
- [ ] #2 A test exercises an index-sensitive read across the partial tile and asserts correct indices/values
- [ ] #3 PRD §6.3.3 clamp semantics (min(y_outer+N,H)) honored or explicitly documented as the chosen representation
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as ADDRESSED-VIA-TASK-0180/0181 (orchestrator-direct, cycle 78 sweep). When this task was filed (post-TASK-0142 P2 review), the per-iteration index reconstruction was NOT done anywhere in the pipeline — the tiled ACFG dropped the y_outer*N+inner+LO mapping and 'the moment an index-SENSITIVE consumer reads the tiled ACFG... this produces WRONG numeric output, silently.' That landmine has since been DEFUSED by TASK-0180 (cycle ~50) + TASK-0181 (cycle 73): both landed per-occurrence absolute-index rebinding via Event::Loop.block_tag on EVERY backend path (single-worker pthreads-sync via TASK-0180; multi-worker walker + mp-tcp-bufsync via TASK-0181 cycle 73 + TASK-0253 cycle 75 consolidation). The rebinding emits 'LO + tile*N + inner' for the full nest and 'LO + num_full*N + inner' for the partial tile — exactly the y_outer*N+inner+LO reconstruction this task asked for. AC#1 ('Tiled ACFG OR its Event projection lets a consumer recover the true per-iteration index') is met via the Event projection (block_tag); AC#2 ('A test exercises an index-sensitive read across the partial tile') is met by the 4 tests in nucleus/backend-common/tests/multi_worker_blocked_rebind.rs + 4 tests in tests/block_tag_loop_header.rs (8 total, all pin both full-nest AND partial-tile branches); AC#3 (PRD §6.3.3 clamp semantics) is met by the precise tile decomposition (full nests × N iterations + partial tile of N - clamp_count) — explicitly documented in BlockTag's doc-comments. The fix landed at a DIFFERENT layer than the task originally envisioned (Event projection, not tiled-ACFG-layer reconstruction) but satisfies the same correctness goal that the task's AC#1 explicitly permitted as either-or.
<!-- SECTION:FINAL_SUMMARY:END -->
