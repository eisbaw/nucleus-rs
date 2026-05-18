---
id: TASK-0154
title: Decide a logging/tracing facility for the compiler crate
status: To Do
assignee: []
created_date: '2026-05-18 09:22'
labels:
  - compiler
  - tooling
  - decision
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Several tasks (TASK-0151 AC#2, future diagnostics) want traceable debug output (e.g. 'cross-scope finalisation skipped for block-governed seq N'). The compiler crate currently has NO logging facade and deliberately minimal deps (chumsky/syn/serde only; MSRV 1.83; no env_logger/tracing). Adding one is a project-wide decision: log+env_logger vs tracing vs a tiny in-house cfg!(debug)-gated eprintln helper vs a structured diagnostics sink surfaced via the driver. Until decided, deferral points are documented in-code with TASK references instead of logged. Pick an approach consistent with PRD tech-stack and the no-spam ethos.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A logging/diagnostics approach is chosen and documented in PRD or a decision record
- [ ] #2 transfer_inject per-subtree skip emits a traceable message via the chosen facility (closes TASK-0151 AC#2)
<!-- AC:END -->
