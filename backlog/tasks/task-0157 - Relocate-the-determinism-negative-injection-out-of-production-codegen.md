---
id: TASK-0157
title: Relocate the determinism-negative injection out of production codegen
status: To Do
assignee: []
created_date: '2026-05-18 09:58'
labels:
  - M2
  - backend
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0145 (Finding 2, MAJOR-but-separable): the NUC_NONDET_TEST perturbation lives inline in pthreads-sync multi_worker.rs slot emission — test-only scaffolding compiled into every shipping build of the backend, on the codegen critical path. It is now deterministic (reverse order), value-gated (=='1'), and prints a loud stderr banner, so it is safe, but the seam is not clean. Move the perturbation to a single documented #[doc(hidden)] test hook, or perform it harness-side (post-process one emitted tree), so production codegen carries no self-corruption branch. Keep behaviour identical; just relocate the seam.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 pthreads-sync production codegen path contains no test-only nondeterminism branch
- [ ] #2 determinism-check-negative still bites 100% (reuse TASK-0145 verification: >=5 consecutive runs)
- [ ] #3 Loud-banner + value-gate safety properties preserved
<!-- AC:END -->
