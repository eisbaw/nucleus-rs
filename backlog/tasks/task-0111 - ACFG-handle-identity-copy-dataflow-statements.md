---
id: TASK-0111
title: 'ACFG: handle identity-copy dataflow statements'
status: To Do
assignee: []
created_date: '2026-05-18 01:23'
labels:
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
build_acfg currently skips Dataflow statements whose RHS is not a Call (e.g. identity-copy 'd <-- e'). The link pass also files a parallel limitation (TASK-0097). The right fix for both is co-designed: identity copies should likely become an Operation with no kernel firing but a 'data move' edge, lowered to a Xfer when producer/consumer workers differ. Coordinate with TASK-0097.
<!-- SECTION:DESCRIPTION:END -->
