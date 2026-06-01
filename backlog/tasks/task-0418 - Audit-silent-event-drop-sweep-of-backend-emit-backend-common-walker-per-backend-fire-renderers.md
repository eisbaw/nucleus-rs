---
id: TASK-0418
title: >-
  Audit: silent event-drop sweep of backend emit (backend-common walker +
  per-backend fire renderers)
status: To Do
assignee: []
created_date: '2026-06-01 22:58'
labels:
  - hardening
  - audit
  - silent-drop
  - backend
  - cycle-239-followup
dependencies:
  - TASK-0417
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0417 (cycle-239) as the OUT-OF-SCOPE half. TASK-0417 swept the nucleus-compiler IR-construction/transformation surface for silent statement/edge drops (build_dataflow was the only one; fixed by TASK-0360). The BACKEND emit surface is the untested sibling: a backend that consumes the Event list and emits code could silently skip an event (a Fire/Push/Wait/Sync that should emit code but emits nothing) via a `_ => {}` match arm, a `continue`, or a filter — producing a deadlock or wrong-answer with no diagnostic.

SCOPE: backend-common (multi_worker_walker render_worker_events, render/fire, check_frame) + each of the 7 tier-1 backends fire/event renderers. Look for: Event match arms that fall through to `_ => {}` or skip without emitting; filter/continue over the event list. Classify each as legit (e.g. Sync handled structurally elsewhere) vs silent-drop hazard. For any hazard: fail-loud (EmitError or debug_assert) + a bite test. Mirror the TASK-0417 classification method.

NOTE: the e2e bit-identical differential is a strong backstop here (a dropped event usually breaks output), but a backend-SPECIFIC drop on a [[skip]]ped cell, or a drop that happens to be output-neutral but deadlocks under a different topology, would NOT be caught by the current e2e matrix. That gap is the value of this audit.
<!-- SECTION:DESCRIPTION:END -->
