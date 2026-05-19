---
id: TASK-0183
title: Relocate the cross-backend-negative wire injection out of production codegen
status: To Do
assignee: []
created_date: '2026-05-19 02:55'
labels:
  - M3
  - backend
  - tech-debt
dependencies:
  - TASK-0178
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect-style seam concern, parallel to TASK-0157 (which tracks the same for TASK-0145's NUC_NONDET_TEST). The TASK-0178 NUC_XBACKEND_NEGATIVE perturbation lives inline as maybe_corrupt_wire in mp-tcp-bufsync production lib.rs, called on the wire.rs emission critical path of every shipping build. It is deterministic (fixed source rewrite), value-gated (=='1'), loud-bannered and anchor-guarded (panics if wire_runtime drifts), so it is SAFE — but the seam is not clean: production codegen carries a self-corruption branch. Move it to a #[doc(hidden)] test hook or perform it harness-side (post-process the emitted mp-tcp tree) so production codegen has no test-only branch. Keep behaviour identical; just relocate the seam.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 mp-tcp-bufsync production codegen path contains no test-only corruption branch
- [ ] #2 xbackend-check-negative still bites 100% (>=3 consecutive runs, non-flaky)
- [ ] #3 Loud-banner + value-gate + anchor-drift-panic safety properties preserved
<!-- AC:END -->
