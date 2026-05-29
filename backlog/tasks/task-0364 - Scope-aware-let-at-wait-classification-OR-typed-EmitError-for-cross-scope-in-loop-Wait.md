---
id: TASK-0364
title: >-
  Scope-aware let-at-wait classification OR typed EmitError for cross-scope
  in-loop Wait
status: To Do
assignee: []
created_date: '2026-05-29 15:13'
updated_date: '2026-05-29 15:35'
labels:
  - tests
  - backend-common
  - defensive
  - latent-defect
  - cycle-222-follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Latent emit defect characterized by TASK-0356 (cycle 222, OUTCOME-MATRIX branch d). The let-at-wait classifier descends into Event::Loop bodies, and render_wait_assign emits 'let <name> = <rhs>;' at the Wait's scope (the per-backend pre-init pass omits the outer 'let mut <name> = vec![0; N];'). If a Wait of let-at-wait data sits inside a Loop body while a consumer (Fire kernel-arg read or Push) of that data sits at the ENCLOSING outer scope, the emitted 'let <name>' lands inside the 'for { }' block and the outer consumer reads <name> out of scope -> non-compiling Rust (rustc E0425, confirmed empirically cycle 222).

NOT currently producible: transfer_inject (inject_in_sequence) co-locates every cross-worker Wait in the SAME sequence as its consuming Operation (pushed via out.push immediately before the Operation; nested-block consumers get their Wait inside the nested sequence by recursion). So today the at-risk shape cannot arise from valid lower/link output. This is filed defensively for the FUTURE case where a pass lifts a consumer out of a loop while leaving its Wait behind (e.g. a hoist), at which point the broken scope would ship as a latent miscompile rather than a fail-loud.

Fix options (pick one):
(A) Make collect_let_at_wait_data scope-aware: exclude an in-loop Wait whose data is consumed at an outer (enclosing) scope from let-at-wait classification, so the outer-scope 'let mut <name> = vec![0; N];' pre-init is retained and the in-loop Wait emits a plain 'name = rhs;' assign.
(B) Emit a typed EmitError::ContractGap from render_worker_events (or render_wait_assign with scope context threaded in) when a let-at-wait Wait sits in a strictly-inner scope relative to a consumer of the same data. Fail loud rather than emit non-compiling Rust.

Characterization pin lives in backend-common/tests/wait_let_at_wait_loop_scope.rs (at_risk_shape_emits_broken_scope_no_emit_error) — it asserts the CURRENT broken-scope emit with an exact-string footprint, so this fix landing will make that test fail loudly and force a re-characterization there.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Either (A) the let-at-wait classifier excludes an in-loop Wait whose data is consumed at an outer/enclosing scope (outer-scope let mut pre-init retained), OR (B) render_worker_events/render_wait_assign emits a typed EmitError for the cross-scope use.
- [ ] #2 The TASK-0356 characterization pin (wait_let_at_wait_loop_scope.rs) is updated to assert the new correct behaviour (well-scoped emit or EmitError) instead of the broken-scope footprint.
- [ ] #3 A regression test constructs the at-risk shape via a real lower/link path (or a synthetic ACFG run through inject_transfers) IF a producing pass is ever added; otherwise the synthetic walker-level pin from TASK-0356 suffices and is updated in place.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry (from TASK-0356 cycle-222 architect P3.1): the let-at-wait emit path is reached by FIVE backends that build a populated WalkerCtx.let_at_wait_data via collect_let_at_wait_data — pthreads-sync, pthreads-async, mp-tcp-event, mp-uds-event, openmp-rs (NOT three). mp-tcp-bufsync is the only multi-worker backend that bypasses (tcp_plan/events.rs, empty_let_at_wait_set). When fixing this hazard, audit ALL FIVE populated-set callers. Option A (scope-aware classifier exclusion in collect_let_at_wait_data) fixes all five at once (shared helper); option B (typed EmitError in render_wait_assign) also covers all five via the shared walker. The characterization test wait_let_at_wait_loop_scope.rs pins option-A bite via classifier_includes_in_loop_whole_array_wait_for_at_risk_shape and option-B bite via at_risk_shape_emits_broken_scope_no_emit_error — re-characterize both when this lands.
<!-- SECTION:NOTES:END -->
