---
id: TASK-0450
title: >-
  Split pre-existing mega-file embedded-pattern/src/multimcu.rs (1478 LoC; just
  ci RED)
status: To Do
assignee: []
created_date: '2026-06-05 04:44'
labels:
  - compiler
  - hygiene
  - megafile
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
embedded-pattern/src/multimcu.rs is 1478 LoC, over the 1000-LoC just check-mega-files fence and NOT on the allow-list — a PRE-EXISTING RED (confirmed at the TASK-0343.01.01 baseline, unrelated to that work). just ci has been RED on this file independently of the cheap pre-commit subset (memory: feedback-cheap-subset-blind-to-structural-fences). Split along the module-level docstring seams (emit_bin / multimcu.resc generation / UART-hub shim / input-offset layout) into cohesive sub-modules, OR allow-list with a one-line rationale if it is a single coherent unit. Sibling of the mega-file split cluster TASK-0340.x / TASK-0383 / TASK-0435 / TASK-0437.
<!-- SECTION:DESCRIPTION:END -->
