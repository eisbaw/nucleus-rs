---
id: TASK-0170
title: >-
  Guard collect_loop_bounds same-name-loop panic before EventList codegen goes
  live
status: To Do
assignee: []
created_date: '2026-05-18 23:26'
labels:
  - M2
  - compiler
  - robustness
  - fail-fast
dependencies:
  - TASK-0160
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect reviews of TASK-0160 and TASK-0169 (P2): build_sidecar/collect_loop_bounds in nucleus/compiler/src/sidecar.rs HARD-PANICS at compiler runtime if two loops share an IterVar (same loop-var name, PRD 6.2.3 one namespace) but have DIFFERENT bounds (keeps first; idempotent if identical). No current example (01/02/03/05/07) hits it, so it is a latent panic on a class of otherwise-valid input, currently tracked only as prose breadcrumbs across TASK-0124/0167 notes — not a first-class item. Per fail-fast discipline this must be a real guarded path before the EventList-only backend (TASK-0124) consumes loop_bounds. Decide: (a) make the same-name-different-bounds case a typed compile error surfaced via the driver (not a panic), or (b) prove it impossible upstream (lowering already rejects it) and add a should_panic/characterisation test pinning the contract, or (c) make Event::Loop/loop_bounds key on something that distinguishes the two loops. Add a regression/characterisation test either way.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Same-name-loop-differing-bounds is either a typed driver-surfaced error OR proven-impossible-upstream with a pinning test (no bare compiler panic on valid input)
- [ ] #2 A characterisation/regression test pins the chosen contract
- [ ] #3 TASK-0124 EventList path cannot reach the bare panic
<!-- AC:END -->
