---
id: TASK-0407
title: 'Hardening: dead-code / dead-reexport / limitation audit (review pass)'
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 07:35'
labels:
  - hardening
  - dead-code-audit
  - review-pass
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 endgame. The TEST-COVERAGE hardening wave is exhausted (prove-the-check-bites across all typed error enums SATURATED per TASK-0400/0401/0402/0404; serde round-trips + determinism + parser ParseError SATURATED per TASK-0406; doc-citation fences saturated; parser fuzz TASK-0399). The remaining named hardening dimension is REVIEW-PASS type: a dead-code / dead-reexport / limitation audit.

SCOPE: (1) dead pub re-exports (memory feedback-visibility-tighten-doclink-trap: backend-common pub mod re-exports are often DEAD -- remove, do not narrow; narrowing doc-linked modules breaks intra-doc-links SILENTLY, so run cargo doc on any visibility change). (2) #[allow(dead_code)] sites -- are they still load-bearing or removable? (3) structurally-dead error variants already FOUND (UnsupportedPartitionKind, InnerRepeatNotFound, BlockTransformError::NotDivisible) -- confirm each is either documented-unreachable or removable; no NEW ones. (4) cargo +nightly udeps / cargo machete for unused deps if available in the dev shell.

METHOD (load-bearing, forward-carried): coverage/inventory audits UNDER-count -- re-derive denominators structurally; grep BOTH tests/ AND inline cfg(test) mods; adversarially try to FALSIFY any saturation/dead claim, do not self-certify (memory feedback-coverage-audit-undercount-recurring; 3 firings cycle-236). Deliverable = findings -> precise follow-up tasks (and/or small dead-code removals through the normal gate+review). LOWER leverage than the test-coverage wave; best in a FRESH context.
<!-- SECTION:DESCRIPTION:END -->
