---
id: TASK-0391
title: >-
  check-doc-citation-staleness-bare recipe: refresh (or de-line-number) the
  MAINTENANCE-CONTRACT comment's approximate check_frame.rs cross-crate lib.rs
  cite line list
status: To Do
assignee: []
created_date: '2026-05-31 22:16'
labels:
  - tech-debt
  - doc-drift
  - justfile
  - citation-fence
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3.2 on TASK-0382.01 (1fae634). The check-doc-citation-staleness-bare recipe MAINTENANCE-CONTRACT comment block enumerates approximate line numbers (cited as ~133/148/149/179/194/200/201) of the cross-crate bare lib.rs:N citations in nucleus/backend-common/src/check_frame.rs that the WIN=3 prose guard must keep skipping. Those line numbers have DRIFTED from the actual check_frame.rs lines (architect measured ~146/162/193/208). PRE-EXISTING (not introduced by TASK-0382.01), hedged with ~, and it lives in a justfile comment that NO fence scans, so it is a soft doc-drift not a gate failure. Fix: either refresh the line list against current check_frame.rs, OR (more durable) de-line-number it — describe the cites by their stable provenance (pre-extraction historical lib.rs pointers naming pthreads-sync/mp-tcp-bufsync within WIN lines) without specific line numbers, since an approximate line list in a comment itself rots. LOW.
<!-- SECTION:DESCRIPTION:END -->
