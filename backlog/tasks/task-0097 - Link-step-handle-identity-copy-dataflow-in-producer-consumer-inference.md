---
id: TASK-0097
title: 'Link step: handle identity-copy dataflow in producer/consumer inference'
status: To Do
assignee: []
created_date: '2026-05-18 00:42'
labels:
  - compiler
  - link
  - M1-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0011's link step derives producer/consumer worker entities from Dataflow { lhs, rhs: Call }. An identity-copy dataflow ('D <-- E', RHS is a bare DataRef, no kernel) is currently NOT recorded as a producer edge — its data move is invisible to the cross-worker transfer existence check. None of the in-tree examples exercise this, but a real example will. Acceptance: identity copies attribute the producer to whoever wrote the source datum and the consumer to wherever the LHS is later read; cross-worker check catches the resulting flow when applicable. May require a 'data-symbol scope' or 'data-symbol last-writer worker' map.
<!-- SECTION:DESCRIPTION:END -->
