---
id: TASK-0232
title: >-
  Harden Mutex::lock() unwrap-to-expect across pthreads-sync + pthreads-async
  (cross-backend lockstep)
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-21 22:50'
updated_date: '2026-05-22 09:56'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Edit pthreads-async/src/ring_buffer.rs lines 93 + 101: change 'self.mu.lock().unwrap()' -> 'self.mu.lock().expect("ring mutex poisoned — producer panicked before notify")' in both Ring::push and Ring::wait.
2. Edit pthreads-sync/src/multi_worker.rs lines 286 + 291: change 'self.mu.lock().unwrap()' -> 'self.mu.lock().expect("slot mutex poisoned — producer panicked before notify")' in Slot::push and Slot::wait.
3. Search for tests that pin 'lock().unwrap()' literal — none found in tests/ tree (positive ring-shape tests pin while/notify/pop_front but not the lock unwrap).
4. Verify multi-worker emit still byte-identical-up-to-substrate: 02-split × sync should show ONLY the new expect-string + header in diff vs pre snapshot; same for × async.
5. Run just test + clippy + e2e gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-32 implementation complete.

Sites edited:
- nucleus/backends/pthreads-async/src/ring_buffer.rs:93-97 — Ring::push lock unwrap -> .expect('ring mutex poisoned — producer panicked before notify').
- nucleus/backends/pthreads-async/src/ring_buffer.rs:105-109 — Ring::wait lock unwrap -> same expect message.
- nucleus/backends/pthreads-async/src/ring_buffer.rs:48-57 — docstring updated to describe the new .expect(...) precedent (lockstep with Slot<T>) and the cross-backend invariant.
- nucleus/backends/pthreads-sync/src/multi_worker.rs:285-298 — Slot::push + Slot::wait lock unwraps -> .expect('slot mutex poisoned — producer panicked before notify').

Scope check (per task spec): the task spec said 'two .lock().unwrap()' in each Ring impl and 'two .lock().unwrap()' in each Slot impl — exactly the four sites edited. Condvar wait()s still use .unwrap() but those are explicitly out of scope.

Verification:
- 02-split-add/split × pthreads-sync post-snapshot: lock unwrap -> expect change shows up in Slot::push + Slot::wait emit; nothing else moved.
- 02-split-add/split × pthreads-async post-snapshot: same shape in Ring::push + Ring::wait emit.
- Cross-backend diff sync vs async on 02-split (post-change): substrate-only diff retained (Slot vs Ring + slot_ vs ring_ + Mutex/Condvar import vs Arc/Barrier-only import). The four expect strings remain distinct ('slot mutex ...' vs 'ring mutex ...'), as expected — they document the substrate, which IS the documented cross-backend diff.
- No test in tests/ tree literally pins '.lock().unwrap()' (grep 'lock().unwrap' in tests/ outside source: zero matches; only the writeln! sites under src/).

Gate (post-change):
- just test: all suites green.
- just clippy: clean.
- just e2e: 54 / 47 / 0 / 7 stable across 3 runs.
- just determinism-check-negative: OK + 47 perturbed.
- just xbackend-check-negative: OK + 14 applied, 1 detected.

AC status:
- #1 (Slot push/wait .expect): DONE.
- #2 (Ring push/wait .expect): DONE.
- #3 (codegen-string tests updated): n/a — no tests pinned the literal.
- #4 (cross-backend bit-identical on multi-worker still holds modulo substrate): VERIFIED — single-worker cross-backend stays empty; multi-worker diff is substrate-only as documented.

Leaving in In Progress. Not committed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 32 (2026-05-22) — closed. Hardened .lock().unwrap() → .lock().expect('...mutex poisoned — producer panicked before notify') across both backends in lockstep:
- nucleus/backends/pthreads-sync/src/multi_worker.rs:285 (Slot::push), :293 (Slot::wait) — 'slot mutex poisoned...'
- nucleus/backends/pthreads-async/src/ring_buffer.rs:93 (Ring::push), :105 (Ring::wait) — 'ring mutex poisoned...'

Both backends share the cross-backend differential invariant; hardening one without the other would have broken byte-identicality on the SUBSTRATE diff. Now both emit .expect with backend-appropriate poison messages. Out of scope per task spec: Condvar.wait(g).unwrap() sites left as-is. No test pinned .lock().unwrap() — zero test updates. Architect-verified: 02-split-add/split emit via both backends shows 2 'mutex poisoned' occurrences each (push + wait), no other shape change.
<!-- SECTION:FINAL_SUMMARY:END -->
