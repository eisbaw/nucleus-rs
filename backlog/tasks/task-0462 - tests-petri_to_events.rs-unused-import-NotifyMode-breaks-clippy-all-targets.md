---
id: TASK-0462
title: 'tests/petri_to_events.rs: unused import NotifyMode breaks clippy --all-targets'
status: Done
assignee: []
created_date: '2026-06-10 09:05'
updated_date: '2026-06-10 10:00'
labels:
  - clippy
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
During TASK-0456 (block_transform collision fix), cargo clippy -p nucleus-compiler --all-targets failed with: error: unused import: NotifyMode at nucleus/nucleus-compiler/tests/petri_to_events.rs:21:54. The file showed git status M (another wave mid-edit) so it was left untouched per file-ownership rules. block_transform lib clippy is clean; this is a separate, pre-existing/in-flight defect in an integration test target. Fix: remove NotifyMode from the use-list at line 21 if genuinely unused, or add a use of it. Re-run cargo clippy -p nucleus-compiler --all-targets -- -D warnings to confirm green.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
RESOLVED under TASK-0457 (doc-lie sweep + petri_to_events test migration completion). The unused `NotifyMode` import at tests/petri_to_events.rs:21 was the dangling artifact of the wrapper-deletion test migration (NotifyMode was used only inside the now-deleted `petri_wrapper_agrees_with_acfg_entry_point` test). Removed only the `NotifyMode` symbol from the use-list (verified 0 remaining uses; all other imports still in use). `cargo clippy -p nucleus-compiler --all-targets -- -D warnings` now GREEN. Orchestrator: safe to close after the batch gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Resolved within wave 1: the dangling unused NotifyMode import in tests/petri_to_events.rs (left by the stall-retried TASK-0457 attempt-1 test migration) was removed in the same wave (commit b6869e7); clippy --all-targets green for nucleus-compiler. Filed by the TASK-0456 implementer when it broke the shared-tree clippy gate; closed after the batched wave gate confirmed green.
<!-- SECTION:FINAL_SUMMARY:END -->
