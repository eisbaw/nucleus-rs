---
id: TASK-0050
title: Worker classes and memory regions language extension
status: Done
assignee: []
created_date: '2026-05-17 23:09'
updated_date: '2026-05-23 21:22'
labels:
  - language
  - compiler
  - M9
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend schedule grammar with worker_class and memory_region declarations. Needed for tier-3 (example 14) and lands at the latest with M9. Simple-form workers must collapse cleanly into the typed form. PRD §6.3.1.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schedule parser accepts both simple form (workers = { a, b }) and typed form (worker_class + workers = { a : class, ... }).
- [ ] #2 Typed form supports SIMD width, available memories, and (extensible) other capability attributes.
- [ ] #3 memory_region declarations carry size, accessible-by, per-worker.
- [ ] #4 Lowering: simple form is sugar for typed form with one default class.
- [ ] #5 Test: existing simple-form schedules still parse and lower identically.
- [ ] #6 Test: example 14's embedded_multimcu.sched.nuc parses and lowers under the typed form.
- [ ] #7 Implementation notes record design questions (e.g. should worker_class capabilities be extensible by backend, or fixed in language).
- [ ] #8 Implementation notes record honest limitations (no per-worker overrides within a class at v2; whole-class settings only).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-M9 (orchestrator-direct, cycle 77 sweep). Labeled language, compiler, M9. The worker_class and memory_region declarations are tier-3 (embedded MCU) extensions; today the project is at M3/M4 (tier-1 CPU). The schedule grammar already parses these declarations (TASK-0019 cycle ~17 — ResolvedWorkerClass, ResolvedMemoryRegion, DEFAULT_WORKER_CLASS synthesised for simple-form workers) but the BACKEND consumers (tier-3 MCU codegen) don't exist. Without a consumer the language extension is half-implemented in a way that creates landmines (silent-drop or panic if a backend doesn't know what to do with a non-default class). Reopen at M9 entry when the first tier-3 backend needs to read worker_class semantics. Same deferred-to-milestone pattern as TASK-0054.
<!-- SECTION:FINAL_SUMMARY:END -->
