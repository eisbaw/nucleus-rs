---
id: TASK-0466
title: >-
  Curated e2e harness: per-phase timeouts on generated-program spawns (the
  diff-fuzz watchdog, applied to just e2e)
status: To Do
assignee: []
created_date: '2026-06-10 19:21'
labels:
  - production
  - e2e
  - test-flake
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Wave-7 review P2 (cross-cutting sibling): TASK-0453.01.01 gave the GENERATIVE harness process-group-kill timeouts on every spawned phase, but the CURATED harness keeps bare .output() spawns — nucleus/e2e/src/run.rs ~:335-520 (compile/build/run phases) and determinism.rs ~:737-755 — so a curated-cell generated-program deadlock still stalls just e2e overnight (exactly the TASK-0461 pingpong night-eater class, one harness over).

Work: lift the diff_fuzz exec.rs timeout machinery (process_group(0), deadline poll, kill-group-THEN-drain — the drain-first pipe deadlock is documented there; reuse, do not reimplement) into a shared e2e helper consumed by both harnesses; phase-tagged FAIL with output tail on expiry; env knob with a sane default (cells normally finish in seconds; 600s default matches diff-fuzz). The harness retain-on-failure scratch convention applies to timed-out cells too.

Related: TASK-0461 (the unit-test-level watchdog for backend integration tests, e.g. pingpong) — different layer, same class; cross-reference both ways.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Every curated-harness spawn phase carries the group-kill timeout; a deliberately-hung cell FAILs with phase tag + tail instead of stalling (negative test)
- [ ] #2 Machinery shared with diff_fuzz (one implementation), kill-then-drain order preserved and pinned
- [ ] #3 e2e baseline totals unchanged on a green corpus
<!-- AC:END -->
