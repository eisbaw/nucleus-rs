---
id: TASK-0403
title: >-
  Prove-the-silent-drop: inject_check_frames name_iter_vars.get miss silently
  continues (untested skip arm)
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 05:53'
updated_date: '2026-06-01 10:26'
labels:
  - hardening
  - testing
  - prove-the-silent-drop
  - silent-sibling
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 TASK-0402 architect-review P3c follow-out. Distinct CATEGORY from the UnknownLoopVar typed-error family TASK-0402/0400 completed: inject_check_frames.rs:~97 handles a name_iter_vars.get(name) MISS by silently continue-ing (a check directive whose name resolves to an algorithm loop but produced no IterVar -- e.g. a loop the compiler eliminated -- is skipped, the assertion having no loop to bind to). The link step is the documented gate that rejects genuinely-unknown names, so this skip is BELIEVED correct, but the skip arm has NO test (neither a positive that a real eliminated-loop check is dropped, nor a pin that the drop is intentional-not-a-defect).

SCOPE: add a prove-the-silent-drop test -- construct a checks map with a directive whose name is absent from name_iter_vars (eliminated/non-resolving loop) and assert inject_check_frames produces NO check frame for it (and does not panic / does not misbind). Mirror the white-box (LinkedIR, ACFG) poison style of TASK-0402 if a real eliminated-loop fixture is not reachable from .nuc source.

This is a SILENT-DROP guard (returns/continues), NOT a typed-error variant, so it is correctly outside the prove-the-check-bites error-enum audit. Lower value than a typed guard (a wrong silent drop loses a check assertion quietly) -- but exactly the silent-sibling class the project tracks. LOW; purely additive coverage.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0410/0411 (cycle-237): the just-ci gate does NOT build docs, so any change touching a doc-linked symbol (removing/narrowing a pub item, removing an error variant referenced by [`...`]) must run cargo doc --workspace --no-deps before/after and diff the generated-N-warning sum (baseline 10). For bite/sibling-sweep tasks that ADD tests this is usually moot, but if the work removes or renames a symbol carrying an intra-doc-link, add the cargo-doc diff to the gate.
<!-- SECTION:NOTES:END -->
