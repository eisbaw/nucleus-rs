---
id: TASK-0161
title: >-
  Tiled ACFG drops per-iteration index (y_outer+N reconstruction) — latent
  wrong-output landmine
status: To Do
assignee: []
created_date: '2026-05-18 22:06'
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
