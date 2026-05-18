---
id: TASK-0061
title: Open design questions captured in PRD margins
status: To Do
assignee: []
created_date: '2026-05-17 23:10'
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
- [ ] #1 Every PRD 'TODO', '?', 'leaning toward', and 'leans toward' is either resolved into a clear PRD statement, or has a backlog task associated with it.
- [ ] #2 PRD §12 risks list has been audited; each risk has either a mitigation already in PRD, a backlog task, or an explicit deferral.
- [ ] #3 Test: a manual grep over PRD.md for the suspect keywords returns zero unresolved entries.
- [ ] #4 Implementation notes record any deferred-to-v3 decisions explicitly.
- [ ] #5 Implementation notes record honest limitations (this is a one-shot review; new TODOs added after this task closes need their own pass).
<!-- AC:END -->
