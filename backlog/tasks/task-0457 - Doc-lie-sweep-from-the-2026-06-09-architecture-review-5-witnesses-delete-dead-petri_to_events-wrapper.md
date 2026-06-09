---
id: TASK-0457
title: >-
  Doc-lie sweep from the 2026-06-09 architecture review (5 witnesses) + delete
  dead petri_to_events wrapper
status: To Do
assignee: []
created_date: '2026-06-09 21:59'
labels:
  - doc-lie
  - hygiene
  - compiler
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the review (P1.2 + P3.10 + P3.12). Five verified comment-doc lies (line numbers as of review; re-grep on pickup):
1. nucleus-compiler/src/event.rs:165-166 cites transfer_inject.rs:279-290 — that file was split into a directory (TASK-0340.13); the single-global-counter invariant is TRUE but lives at passes/transfer_inject/mod.rs:565-571. Load-bearing citation for the transfer_buffer_for_seq keying decision — fix it.
2. event.rs:786-787 AND sidecar.rs:33-34 both claim build_acfg panics on a non-const loop bound; stale since TASK-0398 made it typed (NonConstLoopBound/OverflowingLoopBound, acfg/build.rs:273-279).
3. backend-common/src/multi_worker_walker/event_walker.rs:2-5,27-31 say the walker is consumed by three backends; lines 75-77 of the SAME file say five; ground truth is seven (both MPI plans also call it, mpi_plan/plan.rs:612). Independently verified.
4. multi_worker_walker/wait.rs:286-291 carries the same stale consumer list.
5. petri.rs:4-7 claims per-worker EventLists are projections of the GlobalNet; in code projection reads the ACFG and the net parameter is unused (petri_to_events.rs:284-286). Also petri.rs module doc still cites the ~500-line budget at 790 lines.

Plus the structural half: DELETE the petri_to_events wrapper — zero production callers (tests only), and its stated later-milestones rationale (petri_to_events.rs:47-53) never materialised; make acfg_to_events the only entry point and fix the petri.rs docstring to say the EventList projects from the ACFG.

Discipline: feedback-comment-doc-lie-recurring — while in each file, spot-check the surrounding multi-claim docstrings against the code, not just the cited lines.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 All five witnesses fixed; surrounding doc claims in each touched file spot-checked (list what was checked in notes)
- [ ] #2 petri_to_events wrapper deleted, tests migrated to acfg_to_events, grep shows zero remaining references
- [ ] #3 just ci green including the doc-citation fences
<!-- AC:END -->
