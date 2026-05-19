---
id: TASK-0154
title: Decide a logging/tracing facility for the compiler crate
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 09:22'
updated_date: '2026-05-19 04:18'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: Decide facility. Reject log+env_logger/tracing (contradicts PRD §12 "three tools, each doing one thing", minimal-dep ethos, MSRV pin). Choose a zero-dep, runtime env-var-gated (NUC_TRACE) nuc_trace! macro -> stderr, silent unless set. Mirrors existing NUC_NONDET_TEST / NUC_XBACKEND_NEGATIVE precedent. Env-gate over cfg!(debug_assertions): runtime-selectable without rebuild + matches existing NUC_* discipline.
2. Document decision: backlog decision create (decisions/ dir empty -> establish via CLI) + short transfer_inject doc note.
3. Add nucleus/compiler/src/trace.rs: nuc_trace! macro, no-op unless NUC_TRACE set; register module in lib.rs.
4. Pass A (~862): walk the opaque Repeat for deferred Wait/Push placeholders, emit one trace line per deferred (data symbol + seq), marked TASK-0149/0150 per-tile deferral (not error).
5. Pass B (~1185): emit trace for each Wait excluded by contains_block_inner (data symbol + seq).
6. Fix lying module-doc/comments that say "documented in-code instead of logged".
7. Test in transfer_inject_hoist.rs: gate-on => trace fires for mixed program, gate-off => silent. Use subprocess or BufWriter sink.
8. Gate: just test / just e2e (must stay 30/26/0/4) / determinism-check + -negative / xbackend-check-negative / clippy -D warnings / just ci. Prove default path byte-silent.
9. Commit (no AI credit). Reconcile TASK-0151 AC#2.
<!-- SECTION:PLAN:END -->
