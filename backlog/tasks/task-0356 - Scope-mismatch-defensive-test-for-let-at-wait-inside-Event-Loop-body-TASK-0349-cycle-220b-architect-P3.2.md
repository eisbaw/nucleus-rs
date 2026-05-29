---
id: TASK-0356
title: >-
  Scope-mismatch defensive test for let-at-wait inside Event::Loop body
  (TASK-0349 cycle 220b architect P3.2)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-27 23:58'
updated_date: '2026-05-29 15:21'
labels:
  - tests
  - backend-common
  - defensive
  - cycle-220b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 architect P3.2: the let-at-wait classifier descends into Event::Loop bodies via collect_let_at_wait_inner. The pre-init drop is whole-data scoped, but the `let {name} = ...` emit happens at the Wait site's scope. If a downstream Fire kernel-arg or Push consumes `name` at an OUTER scope, the emit would not compile (Rust scope error).

Empirically the shipped schedules (09-producer-consumer/pipelined consumer.rs) only consume Wait-data within the same loop body. The Event-ordering invariant ('Wait precedes Fire of consumed data, in the same or enclosing scope') seems to prevent this from manifesting today.

## Acceptance

1. Contrived synthetic Plan with a Wait inside an Event::Loop body and a Fire-input read AFTER the loop body. The Wait-data is otherwise classified as let-at-wait.
2. Expect either:
   (a) An EmitError contract-gap surfacing at compile time (preferred), OR
   (b) Correct outer-scope let mut name = ... fallback emit.
3. Pin the resulting emit string with a sibling regression test.

## Honest scope LIMIT

Defensive; no in-tree schedule today triggers this. Low priority because the cross-scope use-before-decl risk is theoretical at present. File only when a future schedule actually constructs the at-risk shape, OR when refactoring the let-at-wait emit to be aware of scope boundaries.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Read machinery: collect.rs (let-at-wait classifier descends into Event::Loop bodies), wait.rs:122 (emits 'let {name} = {rhs};' at Wait scope for let-at-wait data), event_walker.rs (renders Loop as 'for { }', Wait emit lands inside body).
2. Construct synthetic Loop{body:[Wait(buf)]} + outer Fire reading buf; classify buf let-at-wait; drive render_worker_events; capture emit.
3. Determine producibility: inspect transfer_inject (inject_in_sequence) — does it ever place a Wait in a nested scope while consumer is at enclosing scope?
4. Map to OUTCOME MATRIX; pin actual behaviour; file fix task if latent defect; add code cross-refs.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
OUTCOME: branch (d). Emit on at-risk shape (empirically captured cycle 222):

    for t in (0_i64)..(4_i64) {
        let buf = ring_0.wait(); // recv `buf` from w0
    }
    kernels::consume(buf);

The 'let buf' lands INSIDE the for-block; the outer consumer 'kernels::consume(buf)' reads buf AFTER the loop closes -> out of scope. NOT an EmitError (walker has no scope-tracking), NOT an outer-scope let mut fallback. So AC#2(a) and AC#2(b) are BOTH unmet — the emit is the BROKEN scope. Confirmed non-compiling via standalone rustc reproducer: error[E0425]: cannot find value `buf` in this scope.

PRODUCIBILITY: the at-risk shape is NOT producible by valid lower/link output. transfer_inject's inject_in_sequence pushes every consumer-side Wait into the SAME sequence (out Vec) as its consuming Operation, immediately before it (verified against code, transfer_inject.rs inject_in_sequence ACFGNode::Operation arm + recursive inject_in_node_with_tile descent — not just the module docstring claim 'Insert the Wait immediately before O in O's enclosing sequence'). A nested-loop consumer gets its Wait inside the nested sequence; an outer-scope consumer gets its Wait at the outer scope. The protection is this UPSTREAM CO-LOCATION INVARIANT, not the emit and not an EmitError.

LANDED: tests/wait_let_at_wait_loop_scope.rs (2 tests: classifier_includes_in_loop_whole_array_wait_for_at_risk_shape pins the precondition; at_risk_shape_emits_broken_scope_no_emit_error characterizes the branch-d broken emit with an exact-string footprint that will fail loudly when the fix lands). Code cross-ref comments at wait.rs (let-at-wait emit site) + collect.rs (Loop descent). Fix filed as TASK-0364 (scope-aware classification OR typed EmitError).

STATUS DECISION: marking Done as a characterization+boundary-documentation cycle. AC#1 (synthetic shape) met; AC#3 (pin) met. AC#2's literal disjunction (a/b) is NOT met by the emit — but per the task's own OUTCOME MATRIX branch (d), the honest resolution is 'the protection is the upstream invariant'; this is documented truthfully (no claim the guard 'bites', no claim of an EmitError that does not exist). The latent emit defect is filed (TASK-0364) and cross-referenced in code. The cycle-220b P3.2 hazard is now an executable, truthful characterization.
<!-- SECTION:NOTES:END -->
