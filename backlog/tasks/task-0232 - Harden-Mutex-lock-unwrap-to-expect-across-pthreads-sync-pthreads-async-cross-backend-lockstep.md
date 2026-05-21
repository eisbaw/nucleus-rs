---
id: TASK-0232
title: >-
  Harden Mutex::lock() unwrap-to-expect across pthreads-sync + pthreads-async
  (cross-backend lockstep)
status: To Do
assignee: []
created_date: '2026-05-21 22:50'
labels:
  - tech-debt
  - M4
  - backend
  - panic-not-diagnostic
dependencies:
  - TASK-0228
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-18 review-gate B.1 finding (commit 1351c7e): the ring buffer + the existing Slot<T> in pthreads_sync::multi_worker BOTH use bare `.lock().unwrap()`. Under `panic = 'abort'` (the generated artefact's profile.release), a producer thread panic SIGABRTs the whole process before any consumer can re-enter — so PoisonError is theoretically unreachable in production. But:

(1) Under `panic = unwind` (future toggle, or different downstream compilation profile), `.unwrap()` on a PoisonError emits a useless `PoisonError { .. }` Display.
(2) CLAUDE.md fail-loud-with-context + the project's panic-not-diagnostic recurring-defect class argues for `.expect('ring mutex poisoned — producer panicked before notify')` everywhere a mutex is acquired in emitted runtime code.

Why one task and not two: the pthreads-async ring and the pthreads-sync slot share the cross-backend differential invariant. Hardening one without the other would break byte-identicality on multi-worker schedules (which is currently only enforced for the SINGLE-worker arm but will extend to multi-worker once TASK-0229 lands). Do both backends in lockstep.

Scope:
- nucleus/backends/pthreads-sync/src/multi_worker.rs: change `.lock().unwrap()` in the Slot<T> impl to `.lock().expect('slot mutex poisoned ...')`.
- nucleus/backends/pthreads-async/src/ring_buffer.rs: same change in the Ring<T> impl push/wait.
- Update all codegen-string assertion tests in lockstep.

This is a small mechanical change, but the cross-backend coordination keeps the byte-identicality contract.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 pthreads_sync::multi_worker Slot<T>::push + ::wait use .expect() with a context-bearing message, not .unwrap().
- [ ] #2 pthreads_async::ring_buffer Ring<T>::push + ::wait use .expect() with a context-bearing message, not .unwrap().
- [ ] #3 Codegen-string assertion tests in both backends updated to match the new expect-style.
- [ ] #4 Cross-backend byte-identical test on a multi-worker schedule (once TASK-0228 + TASK-0229 enable it) STILL passes after the change.
<!-- AC:END -->
