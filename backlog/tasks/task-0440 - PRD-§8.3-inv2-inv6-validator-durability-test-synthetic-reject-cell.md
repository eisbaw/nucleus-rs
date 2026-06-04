---
id: TASK-0440
title: PRD §8.3 inv(2)/inv(6) validator durability test - synthetic reject cell
status: To Do
assignee: []
created_date: '2026-06-04 08:16'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per WIP review (2026-06-04). The PRD §8.3 invariants validator (validate_event_lists in driver/src/main.rs) has its REJECT arm exercised by no test. A refactor could silently delete the validator call without any test biting. Per memory project-e2e-gate-trust-caveats.md point 6.

Scope: One synthetic test that constructs an EventList with a duplicated Push (inv-2 violation) or empty Sync (inv-3 violation) and asserts the validator rejects it with typed error. Bypasses the normal building cells (which always produce valid EventLists by construction). ~50 LoC. Defends durability of the TASK-0422 gate work.

Why: Without a biting reject test, the validator's existence is asymptotically guaranteed to rot.

Estimated effort: LOW priority, ~50 LoC, single cycle. No design risk.
<!-- SECTION:DESCRIPTION:END -->
