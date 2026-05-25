---
id: TASK-LOW.01
title: >-
  Backlog tracker md hygiene: phantom record_halo + stale
  halo_inference.rs:53-57 / :848 / :1199 citations across
  task-0299/0303/0305/0307 (TASK-0307 cycle-123 architect P2)
status: To Do
assignee: []
created_date: '2026-05-25 04:31'
labels:
  - M5
  - backlog-hygiene
  - comment-doc-lie
  - forward-carried-from-TASK-0307
dependencies: []
parent_task_id: TASK-LOW
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0307 cycle-123 (the structural sentinel that closed the Option B vacuous-pass arm) ALSO closed the phantom `record_halo` symbol + stale `halo_inference.rs:53-57` absolute-line citations across the SOURCE / TEST files in `nucleus/`. The orchestrator's `replace_all` was scoped to `nucleus/` only.

The cycle-123 architect review-gate flagged the same stale references in the TRACKER markdown files — the orchestrator's durable narrative memory. The replace_all DID NOT touch them.

## What is stale (as of cycle 123, pre-fix)

5 stale citations across 4 task markdown files:

- `backlog/tasks/task-0307 - ...md` lines 37, 38, 48, 49, 59: phantom `record_halo` + stale `halo_inference.rs:848` + `halo_inference.rs:1199` line citations.
- `backlog/tasks/task-0305 - ...md` lines 27, 49, 62, 80, 110: similar phantom citations.
- `backlog/tasks/task-0303 - ...md` lines 43, 75: phantom `record_halo` text-search hint.
- `backlog/tasks/task-0299 - ...md` line 54: phantom `record_halo` text-search hint.

(Exact line numbers may shift as the tracker files evolve. The defect class is the citation rot, not the specific lines.)

## Acceptance criteria

1. `grep -rn record_halo backlog/tasks/` returns zero hits.
2. `grep -rn 'halo_inference.rs:53-57' backlog/tasks/` returns zero hits.
3. `grep -rn 'halo_inference.rs:848' backlog/tasks/` returns zero hits.
4. `grep -rn 'halo_inference.rs:1199' backlog/tasks/` returns zero hits.
5. Each replacement uses the SYMBOLIC search-hint convention (cycle-122 lesson):
   - `record_halo` → `classify_index` (the actual function) AND/OR `per_iv.entry(iv).or_insert(0)` (textual search hint, durable across line moves).
   - `halo_inference.rs:53-57` → `the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs — search for \`absent ≡ explicit-0\``.
   - `halo_inference.rs:848` (the cycle-122 emit-site line) → `per_iv.entry(iv).or_insert(0)` search hint.
   - `halo_inference.rs:1199` (the no_halo_bare_iv test line) → `fn no_halo_bare_iv` symbolic anchor.
6. All edits applied via `backlog task edit` (CLAUDE.md project hygiene rule: never hand-edit task markdown files; route through the CLI).

## Honest scope

LOW priority. Tracker docs are the orchestrator's narrative memory; phantom-symbol citations will re-surface on the next backlog-driven cycle as the same lie (the next implementer brief that loads these tasks via `backlog task view` will inherit them). Pure hygiene; no production-code or test impact.

## Cross-references

- TASK-0307 cycle 123 architect review-gate NEW P2 #2 + NEW P2 #3.
- Memory: `feedback-silent-sibling-defect` — work pinning one visible arm while structurally identical sibling silently skips the fix. This task is the across-code-tracker-boundary instance of that pattern.
- Memory: `feedback-comment-doc-lie-recurring` — phantom `record_halo` is exactly the symbol-that-doesn't-exist comment-doc-lie pattern.
- Cycle-123 sentinel commit (commit hash to be appended on close) — the source-side close that should have been a cross-boundary close.
<!-- SECTION:DESCRIPTION:END -->
