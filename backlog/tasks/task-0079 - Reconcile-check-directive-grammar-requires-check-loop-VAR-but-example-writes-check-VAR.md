---
id: TASK-0079
title: >-
  Reconcile check directive: grammar requires 'check loop VAR' but example
  writes 'check VAR'
status: To Do
assignee: []
created_date: '2026-05-17 23:49'
labels:
  - M0
  - docs
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
docs/grammar-sched.md §4.3 documents a divergence: PRD §6.3.5 and the EBNF specify 'check loop VAR : ...;', but examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc line 105 writes 'check frame : latency_max = 10ms;' without the 'loop' keyword.

Resolve one way:
  (a) Relax grammar: make 'loop' optional after 'check'. Cheap, matches example, but blocks future per-transfer 'check' variants from being unambiguous.
  (b) Fix example: add 'loop' keyword in embedded_multimcu.sched.nuc. Keeps grammar/PRD aligned, preserves room for future 'check transfer X : ...;' syntax.

Recommendation: (b). Future PRD §6.3.5 work (buffer_max, jitter_max) wants the 'loop'/'transfer' qualifier slot to remain distinct.

Acceptance:
- One of (a) or (b) is implemented.
- docs/grammar-sched.md §4.3 is updated to remove the KNOWN DIVERGENCE notice.
- Decision is recorded in the commit message and in the doc.
<!-- SECTION:DESCRIPTION:END -->
