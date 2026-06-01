---
id: TASK-0419
title: >-
  petri_to_events: debug_assert a partition-assigned worker never projects an
  empty Loop body (TASK-0417 architect P3)
status: To Do
assignee: []
created_date: '2026-06-01 23:06'
labels:
  - hardening
  - defense-in-depth
  - silent-drop
  - cycle-239-followup
dependencies:
  - TASK-0417
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3 from the TASK-0417 silent-drop audit. petri_to_events.rs:331 `if body_events.is_empty() { continue; }` drops a per-worker Event::Loop when a worker projects zero body events. This is INTENTIONAL and correct (petri_to_events.rs:304-308: a worker that does nothing in a loop gets no Loop, not an empty-bodied one) — it is NOT the build_dataflow silent-statement-drop class. BUT it is silent-by-design and is the one site a future regression of that class could hide behind (an upstream pass failing to populate a worker body would be swallowed silently).

PROPOSED (architect): add a debug_assert (or nuc_trace) that fires if a worker which IS present in the loop`s per_worker_override (partition_ranges[iter_var], i.e. partition=workers assigned it an exclusive slice) projects an EMPTY body — because such a worker should contribute body events. Stripped in release (no e2e/release behavior change), catches the upstream-population bug loudly in dev/test.

MANDATORY PRECONDITION before adding the assert: empirically VERIFY there is NO legitimate case where a partition-assigned worker projects an empty body (trace host-relay / halo-strip / cumulative / reuse interactions). A false-firing debug_assert is itself a panic-on-valid-input defect (the exact class this project rejects). If a legitimate empty case exists, narrow the predicate or keep it as a nuc_trace! diagnostic only. Add a bite test (a synthetic ACFG where a partition-assigned worker has an empty body must trip the assert/trace).

Pointer: nucleus/nucleus-compiler/src/passes/petri_to_events.rs ~309-345 (the walk-scratch + per_worker_override + empty-body continue).
<!-- SECTION:DESCRIPTION:END -->
