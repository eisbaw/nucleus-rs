---
id: TASK-0468
title: >-
  coverage-heatmap.tex: add missing example rows 28-bin-fsum + 29-jacobi-cap-hit
  (renders 26 of 28 tier-1 examples)
status: To Do
assignee: []
created_date: '2026-06-10 23:30'
labels:
  - paper
  - figures
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Found during TASK-0467 (pre-existing, predates S7): paper/figures/coverage-heatmap.tex omits examples 28-bin-fsum and 29-jacobi-cap-hit entirely — the figure renders 26 rows while the tier-1 corpus has 28 examples with matrix cells. After S7 flipped example-21/29 to all-pass, the all-skipped legend key (s) may also be orphaned — reuse or remove it and reconcile the caption. Verify row data against e2e-matrix.toml when adding (the paper-accuracy discipline).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Heatmap renders all tier-1 examples with rows matching the matrix; legend keys all used or removed; PDF green
<!-- AC:END -->
