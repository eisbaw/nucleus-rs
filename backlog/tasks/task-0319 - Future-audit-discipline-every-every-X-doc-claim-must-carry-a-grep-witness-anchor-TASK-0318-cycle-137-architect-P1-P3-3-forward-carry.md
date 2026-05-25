---
id: TASK-0319
title: >-
  Future-audit discipline: every 'every X' doc claim must carry a grep-witness
  anchor (TASK-0318 cycle-137 architect P1 + P3-3 forward-carry)
status: To Do
assignee: []
created_date: '2026-05-25 11:11'
labels:
  - compiler
  - doc-lie
  - silent-sibling
  - audit-discipline
  - forward-carried-from-TASK-0318
dependencies:
  - TASK-0318
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0318 cycle 137 was a doc-lie audit on `nucleus/nucleus-compiler/src/passes/transfer_inject.rs` that fixed a silent-sibling defect in the `rewrite_partition_tiles_inner` audit listing. The first draft of the fix RE-INSTANTIATED the same defect class — it claimed to enumerate 'every other site that mutates x.tile or w.tile' but missed a third sibling (`build_waits_for_op` line 2483). The architect's read-only review caught the omission via grep; the cycle-137 fold-back rewrote the listing to:
1. Name all three sites by name + line.
2. INCLUDE the exact grep witness in the comment (`IterTile::new(enclosing_tile.to_vec())` returns exactly these three sites).
3. Cite the cross-checked grep scopes for structural-safety.

## Why this is a future-audit pattern, not a code task

The cycle-137 lesson generalises: any audit comment that claims 'every X is covered by Y' is a future-silent-sibling-defect candidate UNLESS the comment carries (a) the grep pattern used to enumerate X, and (b) the resulting match list / count. The pattern then becomes machine-checkable by any later reader.

## Acceptance criteria

1. When a future audit cycle authors a 'every X' enumeration in a code comment, include the exact grep pattern + expected match count (or named list of matches) in the comment text.
2. Apply this discipline retroactively to other existing silent-sibling audits in transfer_inject.rs and backend-common (look for comment fragments like 'every other site', 'every callsite', 'no other place', 'all of X'). For each found, audit whether the claim is anchored to a verifiable grep; if not, add one or file a follow-up.
3. Document the pattern in MEMORY.md under feedback-silent-sibling-defect as a meta-strengthening: audit-fixes are sibling-defect candidates themselves; the mitigation is grep-witness anchoring.

## Honest scope

- LOW priority. Discipline reminder, not a defect. Trigger: next cycle that touches a silent-sibling audit listing OR the next time feedback-silent-sibling-defect fires.
- The architect's TASK-0318 P3-3 also recommended that future audits with checkable closure criteria stage through To-Do → In-Progress → Done with explicit grep-checkable AC (rather than created-Done with prose-only narrative). This task is itself an example of that discipline.

## Cross-reference

- TASK-0318 final notes (`backlog task view TASK-0318 --plain`).
- transfer_inject.rs:1696-1746 (the cycle-137 rewritten listing, now with grep witness).
- MEMORY.md `feedback-silent-sibling-defect`.
<!-- SECTION:DESCRIPTION:END -->
