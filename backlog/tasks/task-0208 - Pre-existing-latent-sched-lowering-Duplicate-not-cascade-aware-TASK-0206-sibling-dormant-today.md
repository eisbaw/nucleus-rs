---
id: TASK-0208
title: >-
  Pre-existing latent: sched-lowering Duplicate* not cascade-aware (TASK-0206
  sibling, dormant today)
status: To Do
assignee: []
created_date: '2026-05-20 17:59'
labels:
  - compiler
  - diagnostics
  - follow-up
  - M0
  - latent
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced during TASK-0206 (cascade-aware duplicate detection at algo-lowering, commit pending). The algo-layer fix consults `Accum::failed_decls` in addition to `ir.consts/data/kernels` when checking for `DuplicateConst/Data/Kernel` — so a re-declaration of a poisoned name STILL fires `DuplicateX`.

The sched-lowering layer has structurally the same pattern: `DuplicateWorkerClass` / `DuplicateMemoryRegion` / `DuplicateWorker` are checked against the successful symbol tables only, not against `Accum::failed_decls`. Per `sched/lower.rs:39` doc, `failed_decls` STAYS EMPTY IN PRACTICE today — sched-decl evaluation can't fail (no arithmetic at the sched layer). So the TASK-0206 sibling defect does NOT BITE today.

WHEN IT WOULD BITE: if sched-decl evaluation ever gains a failure path (e.g., a future `worker_class` body with a const-expression field, an arithmetic option, a placement option that can fail-evaluate), the dormant gap would surface — a re-declaration of a poisoned worker_class/memory_region name would silently pass the duplicate check.

ACTION: pre-emptive parity. Mirror the TASK-0206 fix at sched/lower.rs: extend each `Duplicate*` check arm to also consult `Accum::failed_decls`. The fix is dormant in behaviour today (no live trigger), but pinning it now keeps the sched layer architecturally aligned with the algo layer and prevents a future sched-decl-eval addition from re-introducing the latent gap.

ALTERNATIVE: explicit disclaim in `sched/lower.rs` near the Duplicate* sites, citing the "no live trigger today" comment at line 39 and explicitly noting that if/when a sched-decl-eval failure path is added, this gap re-opens. The architectural alignment with TASK-0206 (algo layer) makes pre-emptive parity the cleaner choice.

ACS:
- [ ] #1 Decide: parity-fix vs explicit disclaim. Argue from sched/lower.rs:106-145 cascade-table + the absence of arithmetic at sched layer.
- [ ] #2 If parity-fix: add the analog of `is_failed_decl(failed_decls, name)` arm to every `Duplicate*` check site at sched/lower.rs (DuplicateWorkerClass, DuplicateMemoryRegion, DuplicateWorker; and the placement-target ones if they share the same single-set pattern). Pin with a same-shape K×M parametric fixture using a SIMULATED poisoned-decl path (since no real one exists today — the fixture stands as a structural guard).
- [ ] #3 If disclaim: update sched/lower.rs:39 (or the per-variant cascade table) to explicitly note the dormant gap and the algo-vs-sched divergence.
- [ ] #4 Full gate green; zero behaviour change for valid input (e2e 30/26/0/4/0).
<!-- SECTION:DESCRIPTION:END -->
