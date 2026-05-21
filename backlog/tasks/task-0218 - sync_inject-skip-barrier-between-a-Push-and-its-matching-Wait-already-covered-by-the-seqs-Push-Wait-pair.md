---
id: TASK-0218
title: >-
  sync_inject: skip barrier between a Push and its matching Wait already covered
  by the seq's Push/Wait pair
status: To Do
assignee: []
created_date: '2026-05-21 14:54'
labels:
  - compiler
  - sync-inject
  - M4
  - latent
dependencies:
  - TASK-0213
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0213 cycle): the root reason TASK-0213's path-2 elision was needed at all is that sync_inject currently interposes a barrier between a Push and its matching Wait. The Push/Wait pair already supplies the rendezvous — the extra barrier is over-synchronisation that creates a structural dependency cycle in the analysis net (Push -> barrier -> Wait can't fire because buffer is full -> Push must fire first -> overflow). sync_inject.rs module doc at lines 39-47 acknowledges general over-syncing but does not call out this specific case. If sync_inject elides such barriers, the marking-aware firing-order in boundedness::derive_firing_order resolves example-13 directly, and path-2 elision in acfg_to_petri becomes unnecessary IR scaffolding.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Identify all (Push, Wait) pairs whose Repeat-scope and worker-set match the barrier between them; elide such barriers in sync_inject.
- [ ] #2 Verify: with TASK-0218 landed, the path-2 elision in acfg_to_petri::emit_xfer can be reverted; example-13 pipeline_parallel still passes boundedness/deadlock via path-1 marking-aware derive_firing_order alone.
- [ ] #3 Forward-carry: if TASK-0218 lands BEFORE TASK-0042.01 ships, the backend's IR view simplifies (no analysis-vs-runtime mismatch); update acfg_to_petri.rs module doc accordingly.
<!-- AC:END -->
