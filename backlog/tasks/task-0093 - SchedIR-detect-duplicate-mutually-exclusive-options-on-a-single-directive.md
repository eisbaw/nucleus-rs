---
id: TASK-0093
title: 'SchedIR: detect duplicate / mutually-exclusive options on a single directive'
status: To Do
assignee: []
created_date: '2026-05-18 00:33'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0010 lowers loop/transfer/check options as a Vec preserving source order, but does not detect duplicates (e.g. `block=64, block=128` on one loop) or mutually-exclusive combinations (e.g. `sync, async` on one transfer). Grammar sched.md sec.2 notes 5 and 7 call these out as linker concerns. The information is preserved on the IR (Vec, ordered); add a pass.
<!-- SECTION:DESCRIPTION:END -->
