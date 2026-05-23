---
id: TASK-0125
title: 'pthreads-sync: error recovery in generated code'
status: Done
assignee: []
created_date: '2026-05-18 02:13'
updated_date: '2026-05-23 20:49'
labels:
  - M4
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The generated code at TASK-0020 has no error recovery; a panic in any kernel aborts the whole binary. For tier-1 testing this is the right fail-loud behaviour (PRD §6.3.5 'on_violation = panic'). For longer-running tier-1 demos and for the eventual tier-3 'on_violation = count' surface (which is incompatible with abort), the codegen needs an opt-in error-recovery path.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Generated main returns Result; kernels are called inside a frame that translates panics or Result::Err into a logged violation.
- [ ] #2 Honours check.on_violation = count|log directives in the schedule.
- [ ] #3 Depends on check directive lowering (no current task -- file when needed).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77). The task description's AC#3 says 'Depends on check directive lowering (no current task -- file when needed)'. The check-directive lowering pipeline has since landed (TASK-0052 family, TASK-0079, on_violation=panic|log|count in sched). When tier-3 'on_violation=count' actually requires generated-code error-recovery beyond the current Drop-guard Count handler (TASK-0052.04), file a fresh task scoped to that specific need.
<!-- SECTION:FINAL_SUMMARY:END -->
