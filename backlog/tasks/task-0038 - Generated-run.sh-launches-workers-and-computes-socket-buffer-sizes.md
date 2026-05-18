---
id: TASK-0038
title: Generated run.sh launches workers and computes socket buffer sizes
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
labels:
  - M3
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
For multi-process backends, each Nucleus build emits a run.sh that: launches one worker process per WorkerId, sets SO_SNDBUF/SO_RCVBUF via env or sysctl, waits for completion, returns non-zero on any worker failure. PRD §8.6, §12 risks.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 run.sh emitted alongside the per-worker binaries when backend is mp-tcp-*.
- [ ] #2 run.sh sets SO_SNDBUF / SO_RCVBUF (via env passed to each binary, which calls setsockopt) sized from the Petri net's per-channel buffer requirements.
- [ ] #3 run.sh exits non-zero if any worker fails or times out; reports which worker failed.
- [ ] #4 Test: run.sh launches a multi-worker example and reports correct exit status.
- [ ] #5 Test: an OS that caps SO_SNDBUF below required (forced via lowering net.core.wmem_max in a container) produces a clear error.
- [ ] #6 Implementation notes record design questions (e.g. should buffer sizing happen in run.sh or be baked into binaries; v2 picks via env).
- [ ] #7 Implementation notes record honest limitations (no per-channel granularity if OS-level cap binds; v2 uses the single highest requirement).
<!-- AC:END -->
