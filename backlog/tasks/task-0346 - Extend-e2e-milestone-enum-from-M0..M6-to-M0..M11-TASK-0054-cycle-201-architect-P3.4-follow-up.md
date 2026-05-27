---
id: TASK-0346
title: >-
  Extend e2e milestone enum from M0..M6 to M0..M11 (TASK-0054 cycle-201
  architect P3.4 follow-up)
status: To Do
assignee: []
created_date: '2026-05-27 11:27'
labels:
  - e2e
  - validation
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect cycle-201 P3.4: nucleus/e2e/src/main.rs:194-207 clamps the [[required]]/[[skip]] entry's 'milestone' field to the M0..M6 tier-1 range. Per the PRD §11 milestone enum, M7..M11 are valid future milestones (M7 MPI blocking, M8 MPI non-blocking, M9 embedded skeleton, M10 STM32H7 Renode, M11 multi-MCU Renode).

The TASK-0054 cycle-201 [[skip]] entries for embedded_multimcu × 7 backends had to use milestone="M6" with the M11-deferred reason inline (clamp made literal "M11" tag rejected). This makes 'what's M11-deferred' non-greppable on the milestone field.

Fix: extend the parser at nucleus/e2e/src/main.rs:194-207 to accept M0..M11 (or the full PRD §11 milestone range), update the harness's --milestone filter to handle the wider range correctly, and update the e2e-matrix.toml [[skip]] entries that were workaround-tagged with M6 (currently the 7 embedded_multimcu cells) to use their real M11 tag.

Priority: LOW. Standalone follow-up, not specifically a cycle-201 fold-back but discovered during cycle-201.
<!-- SECTION:DESCRIPTION:END -->
