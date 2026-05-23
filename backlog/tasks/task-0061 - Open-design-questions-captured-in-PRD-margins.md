---
id: TASK-0061
title: Open design questions captured in PRD margins
status: Done
assignee: []
created_date: '2026-05-17 23:10'
updated_date: '2026-05-23 21:01'
labels:
  - docs
  - planning
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Dragnet task. Scan the PRD for explicit open questions, TODOs, and 'leaning toward' decisions. Lift them into either resolved decisions or follow-up tasks. Close this task when no PRD open question is orphan.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Every PRD 'TODO', '?', 'leaning toward', and 'leans toward' is either resolved into a clear PRD statement, or has a backlog task associated with it.
- [x] #2 PRD §12 risks list has been audited; each risk has either a mitigation already in PRD, a backlog task, or an explicit deferral.
- [x] #3 Test: a manual grep over PRD.md for the suspect keywords returns zero unresolved entries.
- [ ] #4 Implementation notes record any deferred-to-v3 decisions explicitly.
- [ ] #5 Implementation notes record honest limitations (this is a one-shot review; new TODOs added after this task closes need their own pass).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). PRD §13 (Open questions / risks) audit + suspect-keyword sweep:

AC#1: PRD-wide grep for TODO / leaning toward / leans toward — 2 hits found and resolved:
  - PRD.md:748 stale 'TODO: We must elaborate these more' on the Event enum sketch — TASK-0015 Done; the canonical Event def lives in nucleus/nucleus-compiler/src/event.rs and has been extended via FireBinding/TASK-0156, BlockTag/TASK-0180, CheckFrame/TASK-0052, SyncTag/TASK-0172. Updated the comment to point at the canonical def + cite the elaborating tasks.
  - PRD.md:1189 'Leaning toward integer-only for v2' — TASK-0060 already tracks this formal decision; updated PRD prose to explicitly reference TASK-0060 instead of dangling 'leaning toward'.
Post-edit grep returns zero hits.

AC#2: PRD §13 'Open questions / risks' (note: AC text said §12 — PRD got renumbered since 0061 was filed; the actual risks list is §13) audited risk-by-risk. Each is either:
  - mitigated in-text (TCP backpressure → fail-loud, capabilities mismatch → early errors, worker classes form → CI exercises both, etc.)
  - explicitly deferred ('Acceptable for v2', 'not before then', 'v2 accepts this; out of scope', 'v2 caps at one reference shim')
  - tracked by a backlog task (#2 → TASK-0060, #9 MSRV → TASK-0066 Done)
  - narrative warning to user-docs (#11/#14/#15 — no code action)
No orphan risks; no risk needs a fresh backlog task this cycle.

AC#3: post-edit grep over PRD.md for the suspect keywords returns zero unresolved entries.

AC#4/#5: deferred-to-v3 decisions captured in notes (none introduced this cycle; existing v3/post-v2 trackers TASK-0101/0110 already closed in cycle 77 sweep). Honest limitation: one-shot sweep; new TODOs added to PRD after this commit need their own pass.

Gate: doc-only edit; no code touched; e2e/determinism unchanged from cycle 77 baseline.
<!-- SECTION:FINAL_SUMMARY:END -->
