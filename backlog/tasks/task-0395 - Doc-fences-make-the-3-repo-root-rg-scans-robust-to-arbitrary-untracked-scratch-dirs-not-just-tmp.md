---
id: TASK-0395
title: >-
  Doc fences: make the 3 repo-root rg scans robust to arbitrary untracked
  scratch dirs (not just tmp/)
status: To Do
assignee: []
created_date: '2026-06-01 01:10'
updated_date: '2026-06-01 01:10'
labels:
  - tooling
  - ci
  - doc-lie
  - robustness
dependencies:
  - TASK-0394
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-232 architect P3 #4 (review adbf154) follow-up to TASK-0394. TASK-0394 added tmp/ to .gitignore so the three repo-root rg-scanning fences (check-doc-citation-staleness, check-doc-citation-staleness-bare, check-doc-test-name-staleness) skip the conventional scratch dir. But a DIFFERENTLY-named untracked scratch dir (scratch/, foo-out/, an ad-hoc emit dir) would reintroduce the footgun: a stale citation in untracked scratch reds just ci. Robust fix options: (1) scan only git-tracked paths (e.g. feed the fences git ls-files output instead of a bare . root), or (2) restrict each fence to the known crate roots (nucleus/, driver/, etc.) like check-mega-files already does. TRADE-OFF to weigh: option (1) would let a stale citation in an untracked-but-intended-to-be-committed file ESCAPE the fence until git add -- so tracked-only scanning trades the scratch-FP-risk for a pre-commit-coverage-gap. LOW priority; the tmp/ ignore (TASK-0394) covers the conventional case. Only pursue if a non-tmp scratch dir actually trips a fence in practice.
<!-- SECTION:DESCRIPTION:END -->
