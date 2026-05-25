---
id: TASK-0308
title: >-
  Backlog tracker md hygiene: phantom function-name + stale absolute-line
  citation sweep across task-0299/0303/0305/0307 (TASK-0307 cycle-123 architect
  P2)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 04:32'
updated_date: '2026-05-25 05:44'
labels:
  - M5
  - backlog-hygiene
  - comment-doc-lie
  - forward-carried-from-TASK-0307
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0307 cycle-123 (the structural sentinel that closed the Option B vacuous-pass arm) ALSO closed the cycle-122 phantom-function-name citation + stale absolute-line citation pattern across the SOURCE / TEST files in `nucleus/`. The orchestrator's `replace_all` was scoped to `nucleus/` only.

The cycle-123 architect review-gate flagged the same stale-citation pattern in the TRACKER markdown files — the orchestrator's durable narrative memory. The replace_all DID NOT touch them.

## What is stale (as of cycle 123, pre-fix)

12-13 stale citation sites across 5 task markdown files (task-0299, task-0303, task-0305, task-0307, task-0308 itself):

- The phantom-function-name pattern in cross-references + text-search hints.
- Stale absolute-line citations into the `halo_inference.rs` contract paragraph (the cycle-119 line range), the halo-entry sink line, and the `no_halo_bare_iv` test line.

(Exact line numbers may shift as the tracker files evolve. The defect class is the citation rot, not the specific lines.)

## Acceptance criteria

1. `grep -rn` for the phantom-function-name pattern across `backlog/tasks/` returns hits ONLY inside intentional historical-lesson-preservation records (task-0307 notes/final-summary that document the cycle-122 mistake by name as a recurrence-pattern marker). Cleanup did NOT touch those (lesson trail).
2. `grep -rn` for the cycle-119 absolute-line range citation (the contract paragraph) returns the same: hits only inside historical-lesson-preservation records.
3. `grep -rn` for the halo-entry-sink absolute-line citation returns the same.
4. `grep -rn` for the `no_halo_bare_iv` test absolute-line citation returns the same.
5. Each migrated citation uses the SYMBOLIC search-hint convention (cycle-122 lesson): symbolic anchors like `per_iv.entry(iv).or_insert(0)` text search hint, paragraph-title search hints (`search for "absent ≡ explicit-0"`), or `fn no_halo_bare_iv` symbolic anchor.
6. All edits applied via `backlog task edit` (CLAUDE.md project hygiene rule: never hand-edit task markdown files; route through the CLI).

## Honest scope

LOW priority. Tracker docs are the orchestrator's narrative memory; the citation rot will re-surface on the next backlog-driven cycle as the same recurrence-pattern (the next implementer brief that loads these tasks via `backlog task view` will inherit them). Pure hygiene; no production-code or test impact.

## Charitable AC interpretation (cycle 126)

The original AC#1-4 specified ZERO grep hits, but historical-lesson-preservation lines in task-0307 notes/final-summary literally record the cycle-122 mistake by name as a recurrence-pattern marker (the `feedback-comment-doc-lie-recurring` lesson trail). Strictly scrubbing those would erase the lesson punch. Cycle-126 followed the cycle-122 precedent (TASK-0307 AC#3 charitable interpretation that the cycle-123 architect accepted): scrub LIVE/CURRENT citations (in descriptions, ACs, cross-references, implementation plans) and preserve HISTORICAL records that name the mistake by literal symbol. Documented in cycle-126's final summary.

## Cross-references

- TASK-0307 cycle 123 architect review-gate NEW P2 #2 + NEW P2 #3 — the cycle that filed this follow-up.
- Memory: `feedback-silent-sibling-defect` — work pinning one visible arm while structurally identical sibling silently skips the fix. This task is the across-code-tracker-boundary instance of that pattern.
- Memory: `feedback-comment-doc-lie-recurring` — the phantom-function-name citation is exactly the symbol-that-doesn't-exist comment-doc-lie pattern.
<!-- SECTION:DESCRIPTION:END -->
