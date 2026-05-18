---
id: TASK-0152
title: 'transfer_inject: Wait with no producer must hard-error for real ACFGs'
status: To Do
assignee: []
created_date: '2026-05-18 08:32'
labels:
  - M2
  - compiler
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pass B (splice_pushes_global, TASK-0136) silently continues when producer_repeat_path returns None, tolerating partial synthetic test ACFGs. For a real LinkedIR-derived ACFG a cross-worker Wait with no producer anywhere is a compiler-invariant violation (single source of truth: the producer MUST exist). It is currently caught only implicitly downstream by check_deadlock_free. Distinguish synthetic-partial from real input and panic/hard-error with context (which symbol, which seq) for real input, per acfg.rs fail-fast precedent. Raised by mped-architect review of TASK-0136.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Real-ACFG path hard-errors with symbol+seq context on a producerless cross-worker Wait
- [ ] #2 Synthetic partial-ACFG unit tests still tolerated (explicit opt-in)
<!-- AC:END -->
