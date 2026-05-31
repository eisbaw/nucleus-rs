---
id: TASK-0388
title: >-
  check-mega-files gate scope omits nucleus/driver/src (main.rs 1242 LoC
  unchecked + over fence)
status: To Do
assignee: []
created_date: '2026-05-31 11:06'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3.2 on TASK-0049.05. nucleus/driver/src/main.rs is 1242 LoC (grew from 1225 via the 0049.05 --shim print arm) but the `check-mega-files` recipe `find` scope does NOT include nucleus/driver/src, even though the recipe header comment lists nucleus/driver as a covered sub-tree (comment/scope mismatch = latent gate-rot, feedback-cheap-subset-blind-to-structural-fences). Fix: extend the find scope to cover nucleus/driver/src AND split main.rs below the 1000-LoC fence (the arg-parse, the lowering-pipeline orchestration, and the per-backend dispatch are natural seams). Pre-existing; not introduced by 0049.05 but nudged further over.
<!-- SECTION:DESCRIPTION:END -->
