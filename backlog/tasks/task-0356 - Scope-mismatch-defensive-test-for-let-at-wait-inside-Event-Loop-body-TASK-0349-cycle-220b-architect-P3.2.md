---
id: TASK-0356
title: >-
  Scope-mismatch defensive test for let-at-wait inside Event::Loop body
  (TASK-0349 cycle 220b architect P3.2)
status: To Do
assignee: []
created_date: '2026-05-27 23:58'
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
