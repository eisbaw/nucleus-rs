---
id: TASK-0300
title: >-
  Hoist pair_tiles into shared backend-common Plan-construction helper
  (TASK-0296 cycle-116 architect P1.2)
status: To Do
assignee: []
created_date: '2026-05-25 01:18'
labels:
  - backend-common
  - mp-tcp-bufsync
  - refactor
  - hardening
  - forward-carried-from-TASK-0296
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0296 cycle 116 added `pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>` to mp-tcp-bufsync `s Plan` struct, populated by re-walking `per_worker.values()` with `backend_common::multi_worker_walker::collect_xfer_pairs`. The same construction shape is already done inside `WalkerCtx` for pthreads-async + mp-tcp-event.

## Risk
State duplication — both the WalkerCtx-using backends AND mp-tcp-bufsync now collect pair_tiles independently. Two future deltas could drift:
1. A mp-tcp-bufsync-only event source bypasses `per_worker.values()` — pair_tiles becomes stale.
2. A sidecar enrichment of IterTile is consumed in the walker but not by mp-tcp-bufsync (or vice versa).

## Acceptance criteria
1. Lift pair_tiles construction into a shared `backend_common` helper (e.g. `multi_worker_walker::collect_pair_tiles(per_worker) -> BTreeMap<(DataId, SeqTag), IterTile>`).
2. mp-tcp-bufsync uses this helper from Plan::build instead of looping itself.
3. WalkerCtx-using backends pass the same construction result rather than building it inline (or document why they cannot).
4. Sibling check: pthreads-sync also uses the shared walker; verify it benefits from (or is consistent with) this helper.

## Honest scope
- LOW priority — the current duplication is correct by inspection. This is hygiene to keep it that way.
- 1 cycle when picked up. Related to the broader TASK-0284 (lift entire mp-tcp-bufsync per-event walker onto shared multi_worker_walker) — could be done first as a stepping stone or absorbed into TASK-0284.

## Forward-carry from TASK-0296 cycle 116 architect P1.2
<!-- SECTION:DESCRIPTION:END -->
