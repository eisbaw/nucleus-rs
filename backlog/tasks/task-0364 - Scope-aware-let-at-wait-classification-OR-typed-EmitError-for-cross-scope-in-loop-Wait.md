---
id: TASK-0364
title: >-
  Scope-aware let-at-wait classification OR typed EmitError for cross-scope
  in-loop Wait
status: In Progress
assignee:
  - '@claude'
created_date: '2026-05-29 15:13'
updated_date: '2026-05-29 19:02'
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

Implementation plan (cycle 222 follow-up) — OPTION B chosen (typed EmitError, fail-loud).

Decision: B over A. Shape is non-producible today (transfer_inject co-locates Wait+consumer). Failing loud with EmitError::ContractGap is the project panic-not-diagnostic response to contract gaps; near-zero regression risk vs a scope-aware silent classifier transform. Do NOT change classifier behaviour; do NOT change emitted code for any shipped schedule.

Steps:
1. collect.rs: add pub fn check_let_at_wait_scope_safety(events, let_at_wait: &BTreeSet<DataId>, names) -> Result<(), EmitError>. No-op on empty set. Walk events maintaining a lexical scope-path = stack of per-occurrence loop identities (fresh pre-order occurrence index pushed on entering a Loop body, popped on exit). For each D in let_at_wait record scope-paths of every Wait of D and every consumer of D (Fire input ArgBinding::Data{data:D} OR Push{data:D}; Fire OUTPUT not a consumer). D unsafe iff some consumer path c has NO Wait whose path is a non-strict prefix of c. Return Err on FIRST unsafe D, message names data (names.data.get), states let-at-wait Wait nested in loop consumed at enclosing scope, references TASK-0364.
2. event_walker.rs render_worker_events: call check_let_at_wait_scope_safety(events, ctx.let_at_wait_data, ctx.names)? at top (single chokepoint for all 5 populated-set backends).
3. mod.rs: export the new fn.
4. Re-characterize tests/wait_let_at_wait_loop_scope.rs: keep classifier_includes_* green; flip at_risk test to assert Err(ContractGap); add a SAFE-shape boundary test (Wait+consumer both in same loop body => Ok + in-loop let). Update module docstring branch-d section.
5. Update cross-ref comments in collect.rs + wait.rs to "guard LANDED" (honest: classifier still descends unchanged; guard fails loud at render entry).

Verified pre-impl: all 5 backends (pthreads-sync/async, mp-tcp-event, mp-uds-event, openmp-rs) route through render_worker_events (grep nucleus/backends/*/src). mp-tcp-bufsync calls render_wait_assign directly via backend-common/src/tcp_plan/events.rs with empty_let_at_wait_set() — structurally immune.
<!-- SECTION:NOTES:END -->
