---
id: TASK-0217
title: Reject or document pipeline=D when D > iteration_count
status: To Do
assignee: []
created_date: '2026-05-21 14:36'
labels:
  - compiler
  - ir
dependencies:
  - TASK-0213
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0213 (path 2 — push elision in acfg_to_petri) introduces a corner case: when pipeline=D > N (loop iteration count), the elision logic elides only N pushes (all of them), and the buffer place ends with D-N leftover tokens. Boundedness/deadlock passes still accept, but the analysis-net's end-state is non-empty for a finite loop — semantically odd.

Today's link step (TASK-0134 AC#3) only rejects D > buffer=N (where N is capacity), not D > iteration_count. They are different N's:
- D <= buffer=N: bounds the runtime ring-buffer.
- D <= iteration_count: ensures pipelining makes sense (you can't pipeline 2 iterations through 3 stages).

Acceptance criteria:
- #1 Decide: hard-reject D > iteration_count at link-time, OR document the analysis-net leftover-tokens as intentional under D > N.
- #2 If reject: extend check_pipeline_buffer_constraints (link.rs) with the new check, plus a precise LinkError variant + test.
- #3 If document: add a fixture covering D > N and update the acfg_to_petri module doc's elision section.

Discovered while implementing TASK-0213; out of scope for that task.
<!-- SECTION:DESCRIPTION:END -->
