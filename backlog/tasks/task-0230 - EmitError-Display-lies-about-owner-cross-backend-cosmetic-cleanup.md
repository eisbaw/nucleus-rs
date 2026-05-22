---
id: TASK-0230
title: 'EmitError Display lies about owner: cross-backend cosmetic cleanup'
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-21 21:50'
updated_date: '2026-05-22 09:56'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Edit pthreads-sync/src/lib.rs lines 169-177: drop 'pthreads-sync:' literal from EmitError::UnsupportedFeature + EmitError::ContractGap Display arms. Other variants (KernelsReadFailed/OutputCreateFailed/WriteFailed) already do not have the prefix — leave them.
2. Update the explanatory comment in pthreads-async/src/lib.rs:100-108 that documents the OLD doubled-prefix lie — flip it to note this was cleaned up.
3. Search for tests asserting on EmitError Display content — none found that grep on 'pthreads-sync:' as a Display literal in test bodies. Only the writeln! sites themselves.
4. Run just test + clippy + e2e gate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-32 implementation complete.

Sites edited:
- nucleus/backends/pthreads-sync/src/lib.rs:169-184 — EmitError::Display arms for UnsupportedFeature + ContractGap had 'pthreads-sync: ' literal removed; added rationale comment naming the driver dispatch site as the owner of the per-backend prefix.
- nucleus/backends/pthreads-async/src/lib.rs:100-107 — explanatory comment that documented the OLD doubled-prefix lie updated to record the TASK-0230 cleanup; references driver/src/main.rs:406/426/448 as the canonical prefix owner.

Other EmitError variants (KernelsReadFailed, OutputCreateFailed, WriteFailed) never had the 'pthreads-sync:' literal — left untouched.

Verification:
- The user-visible error text after cleanup reads: '<backend> codegen error: unsupported feature: ...' or '<backend> codegen error: EventList/sidecar contract gap: ...' — single prefix, owned by the dispatch site.
- No tests in the source tree asserted on the literal 'pthreads-sync:' prefix in Display output (grep for the literal in test bodies: zero matches outside the writeln!/docstring sites themselves).
- The pthreads-async lib.rs caveat-comment (which itself referenced 'pthreads-sync:' as a literal) was updated in lockstep so the source no longer documents a defect that's been removed.

Gate (post-change):
- just test: all suites green.
- just clippy: clean.
- just e2e: 54 / 47 / 0 / 7 stable across 3 runs.
- just determinism-check-negative: OK + 47 perturbed.
- just xbackend-check-negative: OK + 14 applied, 1 detected.

AC status:
- #1 (Display omits 'pthreads-sync:' literal): DONE.
- #2 (test assertions updated): n/a — none pinned the literal.
- #3 (clean user-visible error text from all three backends): VERIFIED structurally — driver wrappers at main.rs:406/426/448 unchanged, inner Display now neutral.

Leaving in In Progress. Not committed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 32 (2026-05-22) — closed. Edited pthreads_sync::EmitError Display impl (nucleus/backends/pthreads-sync/src/lib.rs:169-184) to drop the 'pthreads-sync:' literal prefix from UnsupportedFeature + ContractGap arms. The driver dispatch site (nucleus/driver/src/main.rs lines 406/426/448) already supplies the per-backend prefix; the inner literal was a cosmetic lie when the same EmitError type was re-exported by mp-tcp-bufsync + pthreads-async. Other Display arms never had the prefix. No test pinned the literal — zero test updates needed. pthreads-async/src/lib.rs comment updated in lockstep to drop the now-stale TASK-0230 reference. Architect-verified: grep finds no remaining 'pthreads-sync:' literal in lib.rs Display.
<!-- SECTION:FINAL_SUMMARY:END -->
