---
id: TASK-0187
title: >-
  maybe_perturb_for_nondet_test partial-silent-neuter: mp-tcp cells skipped
  every run
status: To Do
assignee: []
created_date: '2026-05-19 05:13'
updated_date: '2026-05-19 05:13'
labels:
  - M2
  - backend
  - tech-debt
  - gate-trust
dependencies:
  - TASK-0157
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0157 review gate (mped-architect MAJOR finding, independently reproduced). The relocated harness-side determinism-negative perturbation maybe_perturb_for_nondet_test (nucleus/e2e/src/main.rs ~1514) hard-targets tree/src/main.rs. mp-tcp-bufsync emits src/bin/<worker>.rs / src/bin/nuc-generated.rs and NO src/main.rs (nucleus/backends/mp-tcp-bufsync/src/lib.rs ~125/150/172), so ALL ~13 mp-tcp cells return Err -> DetCellStatus::Skipped on EVERY run, today (empirically: NUC_NONDET_TEST=1 yields pass:0 fail:13 skipped:17). determinism-check-negative still bites ONLY because the pthreads-sync cells emit src/main.rs and Fail. The falsifier integrity therefore rests on an implicit, unasserted invariant (>=1 pthreads-sync cell emits src/main.rs and Fails). Partial-silent-neuter risk: if pthreads-sync ever moves to src/bin/ (as mp-tcp already did) while some unrelated cell Fails for another reason, the recipe prints OK while NUC_NONDET_TEST perturbed nothing. Inconsistent with the TASK-0145/0163/0167/0178 gate-trust lineage (a falsifier must PROVABLY bite). Total-drift IS loud (recipe exit 1); this is the PARTIAL case the TASK-0157 notes under-stated.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 maybe_perturb_for_nondet_test perturbs a file EVERY backend emits (e.g. Cargo.toml) OR is made backend-layout-aware so mp-tcp cells are perturbed too (no longer silently Skipped under NUC_NONDET_TEST=1)
- [ ] #2 When NUC_NONDET_TEST=1, zero successful perturbations across the whole matrix is a hard FAIL (a Failed cell / non-zero exit), never a uniform Skipped that the recipe inverts to OK
- [ ] #3 A unit/integration test asserts the perturbation actually mutated a tree (>=1 cell perturbed) so the falsifier cannot silently become a no-op; determinism-check-negative still bites 100% (>=5 runs) and bare determinism-check stays byte-identical 30/26/0/4
<!-- AC:END -->
