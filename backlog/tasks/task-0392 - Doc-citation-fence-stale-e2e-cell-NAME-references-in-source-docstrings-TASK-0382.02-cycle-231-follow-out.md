---
id: TASK-0392
title: >-
  Doc-citation fence: stale e2e-cell-NAME references in source docstrings
  (TASK-0382.02 cycle-231 follow-out)
status: To Do
assignee: []
created_date: '2026-06-01 00:38'
updated_date: '2026-06-01 00:38'
labels:
  - tooling
  - ci
  - doc-lie
  - cycle-221-followup
dependencies:
  - TASK-0382.02
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-231 follow-out of TASK-0382.02. The check-doc-test-name-staleness fence (cycle-231) validates back-ticked task<NNNN> UNIT-TEST name citations against defined fns. A SEPARATE high-confidence shape remains unvalidated: back-ticked e2e-CELL-NAME citations in source docstrings (e.g. the ec50108 lie 'gather_2out_loop' renamed to 18-multigather/distributed). These must validate against the cell universe in nuc-nucleus/e2e-matrix.toml, NOT against fn defs. HARD part / zero-FP: e2e cell identifiers have two shapes -- the NN-name/variant example path AND bare snake_case aliases (gather_2out_loop) -- and the latter is hard to disambiguate from an ordinary symbol mention. Design: restrict to back-ticked tokens that ALSO appear (or used to) as cell keys/paths in e2e-matrix.toml, SKIP on ambiguity. LOW; purely additive; only build if the alias-vs-symbol ambiguity can be made zero-FP, else keep deferred.
<!-- SECTION:DESCRIPTION:END -->
