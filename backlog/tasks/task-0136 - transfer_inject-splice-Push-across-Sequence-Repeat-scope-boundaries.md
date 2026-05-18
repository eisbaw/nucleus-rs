---
id: TASK-0136
title: 'transfer_inject: splice Push across Sequence/Repeat scope boundaries'
status: To Do
assignee: []
created_date: '2026-05-18 03:50'
labels:
  - compiler
  - M2
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0027 surfaced this gap. The current `splice_pushes_for_waits` walks one Sequence's children and inserts a Push only when the Wait's producing data symbol is in the same sequence's `local_producer_idx`. Cross-scope Waits (inside a Repeat body whose enclosing Sequence holds the producer) therefore never trigger a Push insertion.

This is visible in example 02-split-add: host produces `a` and `b` via load_input/load_input_b at the top level, then enters `for i { add(a[i], b[i]) }` on w0. The injected ACFG ends up with two Waits per iteration on w0 but zero Pushes on host. The Petri net's buffer place gets only the consumer-side arc — the producer-side TtoP arc is missing.

The pthreads-sync backend currently masks this by consuming the ACFG directly with shared-memory shortcuts. TASK-0124 (backend consumes EventLists) cannot be done cleanly until this is fixed.

Fix shape: thread a `cross_scope_producer_idx: BTreeMap<DataId, (Vec<*mut ACFGNode>, usize)>` (or equivalent) through the walk so a child sequence can register a Push for the outer scope to splice on emit. Alternatively: do a two-pass — first walk records every Wait and its data symbol; second walk inserts Pushes after each Operation whose `data_out` matches a recorded Wait, irrespective of scope. The two-pass form is cleaner and matches v2's "no clever single-pass" preference.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Producer Operation in outer Sequence + consumer inside a Repeat body yields a Push placeholder on the producer side and a Wait on the consumer side.
- [ ] #2 Example 02-split-add (split.sched.nuc) projected to EventLists has matched Push/Wait pairs (one Push on host per declared transfer, N Waits on w0 inside the for loop). Test in tests/petri_to_events.rs upgraded to assert pushes.is_empty() is false.
- [ ] #3 Idempotence preserved: re-running inject_transfers does not duplicate the spliced Push.
- [ ] #4 All existing acfg_to_petri and petri_to_events tests still green.
<!-- AC:END -->
