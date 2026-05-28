---
id: TASK-0362
title: >-
  Doc-lie sweep: event.rs ViolationKind Log/Count docstrings say 'NOT wired in
  this cycle — TASK-0052.04' but tier-1 log/count ARE wired
status: To Do
assignee: []
created_date: '2026-05-28 22:50'
labels:
  - tech-debt
  - doc-lie
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3.2 from the TASK-0048.04 cycle-242 review gate (read-only, out of that commit's range; PRE-EXISTING). nucleus/nucleus-compiler/src/event.rs:499 and :504 — the ViolationKind::Log and ::Count doc comments still read 'NOT wired in this cycle (TASK-0052.04)'. Per the project narrative TASK-0052.02/.04 already landed tier-1 panic/log/count codegen end-to-end (and TASK-0048.04 just landed tier-3 log), so these docstrings are stale comment-lies on the canonical contract enum. Recurring defect class feedback-comment-doc-lie-recurring. Fix: update the two doc comments to reflect that Log/Count ARE wired at tier-1 (backend-common/src/check_frame.rs) and, for Log, now tier-3 embedded (embedded-pattern). LOW / docs-only; verify with 'just check-narrative-doc-lie' (note: that arm only scans e2e-matrix.toml, so the real guard is the unit suite + grep) and 'just clippy'.
<!-- SECTION:DESCRIPTION:END -->
