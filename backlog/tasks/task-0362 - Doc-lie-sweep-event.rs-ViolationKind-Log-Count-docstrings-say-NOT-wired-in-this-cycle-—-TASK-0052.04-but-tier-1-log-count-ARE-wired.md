---
id: TASK-0362
title: >-
  Doc-lie sweep: event.rs ViolationKind Log/Count docstrings say 'NOT wired in
  this cycle — TASK-0052.04' but tier-1 log/count ARE wired
status: Done
assignee:
  - '@claude'
created_date: '2026-05-28 22:50'
updated_date: '2026-05-29 00:03'
labels:
  - tech-debt
  - doc-lie
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3.2 from the TASK-0048.04 cycle-242 review gate (read-only, out of that commit's range; PRE-EXISTING). nucleus/nucleus-compiler/src/event.rs:499 and :504 — the ViolationKind::Log and ::Count doc comments still read 'NOT wired in this cycle (TASK-0052.04)'. Per the project narrative TASK-0052.02/.04 already landed tier-1 panic/log/count codegen end-to-end (and TASK-0048.04 just landed tier-3 log), so these docstrings are stale comment-lies on the canonical contract enum. Recurring defect class feedback-comment-doc-lie-recurring. Fix: update the two doc comments to reflect that Log/Count ARE wired at tier-1 (backend-common/src/check_frame.rs) and, for Log, now tier-3 embedded (embedded-pattern). LOW / docs-only; verify with 'just check-narrative-doc-lie' (note: that arm only scans e2e-matrix.toml, so the real guard is the unit suite + grep) and 'just clippy'.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle plan (docs-only, batched with TASK-0048.10). event.rs:499/:504 — both ViolationKind::Log and ::Count docstrings say 'NOT wired in this cycle — TASK-0052.04', a lie. Verified wiring: tier-1 Log = eprintln! (check_frame.rs:184), Count = AtomicU64 + Drop summary (check_frame.rs:109-119); tier-3 embedded Log = TASK-0048.04, Count = TASK-0048.08 (AtomicU32 + program-exit USART1 summary). Fix both doc comments to state they ARE wired at tier-1 (backend-common::check_frame, TASK-0052.04) + tier-3 embedded (Log TASK-0048.04 / Count TASK-0048.08); keep each variant's accurate WHAT description. No enum/logic change.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE (docs-only). Fixed both stale ViolationKind doc comments in nucleus/nucleus-compiler/src/event.rs (Log :499 region, Count :504 region) that read 'NOT wired in this cycle — TASK-0052.04'. Verified wiring before editing: tier-1 Log = eprintln! (backend-common/src/check_frame.rs:184), Count = AtomicU64 + Drop summary (check_frame.rs:109-119); tier-3 embedded Log = TASK-0048.04 (per-violation UART line), Count = TASK-0048.08 (AtomicU32 + program-exit USART1 summary). New Log docstring: 'Wired at tier-1 (`backend-common::check_frame`, TASK-0052.04) and tier-3 embedded (TASK-0048.04, per-violation UART line).' New Count docstring: 'Wired at tier-1 (`backend-common::check_frame` AtomicU64 + Drop summary, TASK-0052.04) and tier-3 embedded (TASK-0048.08, AtomicU32 + program-exit USART1 summary; AtomicU64 is absent on thumbv7em and a spinning firmware never fires a `Drop`).' Each variant's accurate WHAT description retained; no enum/logic change. Commit 1a4a6d7. Gate: build OK, clippy -D warnings clean, test 1079/0/3 unchanged, check-narrative-doc-lie OK.
<!-- SECTION:FINAL_SUMMARY:END -->
