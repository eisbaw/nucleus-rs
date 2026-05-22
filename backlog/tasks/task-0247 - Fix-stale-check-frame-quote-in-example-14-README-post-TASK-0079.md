---
id: TASK-0247
title: Fix stale 'check frame' quote in example 14 README (post-TASK-0079)
status: To Do
assignee: []
created_date: '2026-05-22 13:01'
labels:
  - docs
  - M4
  - real-time
dependencies:
  - TASK-0079
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
nuc-nucleus/examples/14-hearing-aid/README.md:32 still quotes the directive as 'check frame : latency_max = 10ms;' (the pre-TASK-0079 form, no 'loop' keyword). The schedule file itself (schedules/embedded_multimcu.sched.nuc:105) was fixed in TASK-0079 to the conformant 'check loop frame : latency_max = 10ms;'. The README quotation was missed in that cycle. One-line edit. Surfaced during TASK-0052.03 (docs/check-loop-latency-max.md §7) as a stale-doc gotcha.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 README.md line 32 quotes the conformant 'check loop frame : latency_max = 10ms;' form (matches schedule file verbatim)
<!-- AC:END -->
