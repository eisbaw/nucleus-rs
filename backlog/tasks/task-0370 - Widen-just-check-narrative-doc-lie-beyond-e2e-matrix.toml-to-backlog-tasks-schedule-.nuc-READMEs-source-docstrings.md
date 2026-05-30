---
id: TASK-0370
title: >-
  Widen just check-narrative-doc-lie beyond e2e-matrix.toml to backlog/tasks,
  schedule .nuc, READMEs, source docstrings
status: To Do
assignee: []
created_date: '2026-05-30 11:08'
labels:
  - tooling
  - ci
  - doc-lie
  - robustness
  - cycle-213-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (R5, robustness). VERIFIED: the check-narrative-doc-lie recipe in the justfile targets only nuc-nucleus/e2e-matrix.toml, but the comment/doc-lie class is the projects #1 recurring defect (12+ firings) and fires across backlog/tasks/*.md, schedule .nuc headers, README files, and source docstrings — currently caught only by repeated MANUAL citation sweeps (open: TASK-0308/0311/0312/0313 and the cycle-213 P2 fix). Extend the recipes pattern set + file targets so the structural check covers those locations, converting recurring manual sweeps into a gate-time catch. Must stay zero-false-positive on the current tree (same bar as the other check-* recipes).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 check-narrative-doc-lie scans backlog/tasks/*.md, nuc-nucleus/examples/*/schedules/*.sched.nuc headers, README.md files, and crate source docstrings (or a justified subset) in addition to e2e-matrix.toml
- [ ] #2 The widened patterns capture at least the historically-recurring lie shapes (stale absolute-line citations, phantom function names, "every X" claims without a grep-witness, "only N backends remain" staleness) and run clean (exit 0, zero false positives) on the current tree
- [ ] #3 Wired into just ci so a future doc-lie in the covered locations fails the gate
<!-- AC:END -->
