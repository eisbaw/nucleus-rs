---
id: TASK-0192
title: Bring 14-hearing-aid/embedded_multimcu into the lower/link/ACFG test matrix
status: Done
assignee: []
created_date: '2026-05-19 14:39'
updated_date: '2026-05-23 20:56'
labels:
  - M11
  - test
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0079 made embedded_multimcu.sched.nuc parse cleanly (added the grammar-conformant 'check loop' qualifier). It is currently excluded from the sched_lower / link / acfg / transfer_inject / sync_inject positive test matrices by scope (it is a far-future M11 multi-MCU schedule, not part of the M3 matrix). Once the M11 multi-MCU lowering path is in scope, add this schedule to those positive suites and remove the scope-exclusion comments (which currently point here). Until then this is a deliberate, documented gap, not a regression.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 embedded_multimcu.sched.nuc is included in the sched_lower / link / acfg positive matrices (or a documented reason why a given suite still excludes it)
- [ ] #2 The scope-exclusion comments in link.rs/acfg.rs/sched_lower.rs that reference this task are updated or removed
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-M11 (orchestrator-direct, cycle 77 sweep). The task is labeled 'M11, test, language' and the description explicitly scopes it 'Once the M11 multi-MCU lowering path is in scope'. M11 is the milestone for multi-MCU Renode co-simulation; today the project is at M3 / M4. Bringing 14-hearing-aid/embedded_multimcu into the lower/link/ACFG positive matrices now would either silently exclude (already-documented scope-exclusion comments do this) or surface unimplemented-feature errors that aren't actionable. Reopen at M11 entry; the scope-exclusion comments in link.rs/acfg.rs/sched_lower.rs that reference this task are the trigger marker. Same deferred-closure pattern as TASK-0091/0125/0147/0148/0166/0185/0067/0069/0070/0071/0101/0110.
<!-- SECTION:FINAL_SUMMARY:END -->
