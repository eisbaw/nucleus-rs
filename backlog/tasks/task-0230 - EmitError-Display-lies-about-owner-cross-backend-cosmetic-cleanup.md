---
id: TASK-0230
title: 'EmitError Display lies about owner: cross-backend cosmetic cleanup'
status: To Do
assignee: []
created_date: '2026-05-21 21:50'
labels:
  - tech-debt
  - M4
  - backend
  - cosmetic
dependencies:
  - TASK-0226
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Found in TASK-0042.01 cycle 16 skeleton work: pthreads_sync::EmitError's Display impl prepends 'pthreads-sync:' to every variant's message (KernelsReadFailed -> 'failed to read kernels.rs at ...'; ContractGap -> 'pthreads-sync: EventList/sidecar contract gap: ...'; UnsupportedFeature -> 'pthreads-sync: unsupported feature: ...').

mp-tcp-bufsync re-exports this type (lib.rs:81 `pub use pthreads_sync::EmitError`) — so a ContractGap from mp-tcp-bufsync emits a message reading 'pthreads-sync: EventList/sidecar contract gap: ...'. The driver dispatch site wraps it as 'mp-tcp-bufsync codegen error: pthreads-sync: ...' — the inner prefix is now a small cosmetic lie about WHICH backend emitted the error.

pthreads-async (TASK-0042.01 cycle 16) inherits the same situation (it also re-exports). With the third backend wired the doubled-prefix becomes visible on every codegen error from the third tier-1 backend.

This is cosmetic, not a correctness defect. Fix: change pthreads_sync::EmitError::Display to omit the 'pthreads-sync:' literal — let the driver dispatch site own the per-backend prefix (it already does). Test-asserted strings may need updates; the change is mechanical.

Defer to AFTER TASK-0226/0227/0229 land — changing emit-string Display now would churn against TASK-0226's tests during code review.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 pthreads_sync::EmitError::Display omits the 'pthreads-sync:' literal — the dispatch site owns the per-backend prefix.
- [ ] #2 Test assertions on EmitError string content updated in lockstep.
- [ ] #3 All three backends' (and the driver's) user-visible error text reads cleanly (no doubled prefix).
<!-- AC:END -->
