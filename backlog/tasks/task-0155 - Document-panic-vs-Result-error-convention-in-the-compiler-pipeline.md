---
id: TASK-0155
title: Document panic-vs-Result error convention in the compiler pipeline
status: To Do
assignee: []
created_date: '2026-05-18 09:39'
labels:
  - compiler
  - docs
  - decision
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of acf8bab/8fad5d3 (Finding 1): the driver pipeline now mixes two error conventions — apply_block_transforms returns Result<_,BlockTransformError> surfaced as a clean 'nucleus: error:' stderr line, while inject_transfers panics with a backtrace on a broken cross-pass invariant. Both are individually correct (user-diagnosable error vs compiler-invariant violation, matching the acfg.rs:612 panic precedent), but the rule for which mechanism to use is unwritten tribal knowledge. Document it durably (transfer_inject module docs or a decision record): compiler-invariant violations panic per acfg.rs precedent; user-diagnosable errors return Result and surface via the driver stderr channel. No code behaviour change.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The panic-vs-Result convention is written down in module docs or a decision record
- [ ] #2 transfer_inject + block_transform reference the convention
<!-- AC:END -->
