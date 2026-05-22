---
id: TASK-0208
title: >-
  Pre-existing latent: sched-lowering Duplicate* not cascade-aware (TASK-0206
  sibling, dormant today)
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-20 17:59'
updated_date: '2026-05-22 13:16'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## cycle-44 implementation (mped-architect-impl, 2026-05-22)

**Action chosen: (A) parity-fix.** The TASK-0206 algo precedent maps
1:1 onto the three pass-1 sched Duplicate* arms (DuplicateWorkerClass,
DuplicateMemoryRegion, DuplicateWorker). Pass-2 Duplicate* variants
(DuplicatePlace/PlaceData/Loop/Transfer/Check) check against pass-2
tables and are structurally a different problem (statement-level
dupes, not decl-level cascade); scope is the three pass-1 sched-decl
Duplicates per task description and the per-variant cascade table.

### Changes (nucleus/compiler/src/sched/lower.rs)

1. New helper `is_failed_sched_decl(failed_decls, name) -> bool`
   (line ~196) — mirrors algo `is_failed_decl`.
2. Three pass-1 Duplicate* sites consult `failed_decls`:
   - `Directive::WorkerClass` arm (line ~232): `|| is_failed_sched_decl(...)`
   - `Directive::MemoryRegion` arm (line ~268): same
   - Cross-decl `DuplicateWorker` (line ~337): same
   The intra-decl `seen_in_this_decl` check (per-decl twin guard) is
   intentionally NOT wrapped — it's a single-sweep local set with no
   cascade interaction.
3. Test seam `lower_sched_with_accum(ast, acc)` — extracted from the
   body so the unit-test fixture can PRE-SEED `failed_decls`. Public
   `lower_sched` calls it with `Accum::default()` — production
   behaviour byte-identical.
4. Cascade-table at sched/lower.rs:128-130 updated: three rows now
   marked `(cascade-aware: TASK-0208)` with "FIRES on re-decl of
   poisoned name (dormant)" qualifier. Old "dormant gap" caveat at
   lines 113-114 augmented with the parity-paragraph explicitly
   citing TASK-0208 + the structural-fixture name.

### Tests added

`#[cfg(test)] mod tests` at end of file:
- `cascade_aware_duplicate_fires_when_failed_decls_populated` —
  parametric K×M (K∈{1,2,3} × M∈{1,2} × kind∈{WorkerClass,
  MemoryRegion, Worker}) structural pin: pre-seeds K poisoned names
  into Accum::failed_decls, asserts every re-declaration fires the
  expected Duplicate* AND every seeded name is hit. Will trip if any
  of the three is_failed_sched_decl clauses is removed.
- `cascade_aware_duplicate_does_not_overfire_with_empty_failed_decls`
  — negative-control: same surface area with empty Accum lowers
  cleanly. Pins the cascade-aware clause is purely additive.

### Honest limits

- The simulated fixture cannot be in tests/sched_lower.rs because
  `Accum` is private (intentional encapsulation). Lives as a unit
  test in src/sched/lower.rs — uses parse_sched directly, not the
  integration lower_str helper. Same approach as if/when sched-decl
  gains a real failure path: the simulated fixture stays AND a
  companion real-source fixture joins it.
- The Worker kind for M>=2 includes some intra-decl
  `seen_in_this_decl` duplicate firing alongside the cascade-aware
  path. The assertion uses `>= K*M` (not `==`) and a per-name
  must-fire structural guard — so the cascade-aware path is
  load-bearing, and the extra intra-decl signal doesn't pollute the
  pin direction.
- Zero behaviour change for valid input — confirmed by the e2e gate
  (88/70/0/18 unchanged) AND the negative-control unit test.

### Gate

- `nix develop -c just test`: all green, 2 new tests pass
  (`sched::lower::tests::cascade_aware_duplicate_*`).
- `nix develop -c just clippy`: clean (one initial
  `single_char_add_str` lint hit and was fixed in same edit).
- `nix develop -c just e2e`: 88/70/0/18 unchanged.

### Status

READY FOR REVIEW + COMMIT (not committed — workflow rule 6 honoured).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 44 (2026-05-22) — closed. Action (A) parity-fix chosen. The TASK-0206 algo-layer cascade-aware pattern is now mirrored at nucleus/compiler/src/sched/lower.rs for the 3 pass-1 Duplicate* arms (DuplicateWorkerClass, DuplicateMemoryRegion, cross-decl DuplicateWorker).

Concretely: new helper is_failed_sched_decl(failed_decls, name) structurally identical to algo's is_failed_decl; each Duplicate* check site now consults failed_decls in addition to the successful-symbol table. New test-seam lower_sched_with_accum(ast, acc) extracted (private fn — production lower_sched(ast) calls it with Accum::default(), byte-identical production behaviour).

Two new tests in src/sched/lower.rs's tests mod:
- cascade_aware_duplicate_fires_when_failed_decls_populated — K×M parametric (K∈{1,2,3}, M∈{1,2}, kind∈{WorkerClass, MemoryRegion, Worker}); pre-seeds K poisoned names; asserts dup_count >= k*m + every seeded name appears in a Duplicate of the expected kind.
- cascade_aware_duplicate_does_not_overfire_with_empty_failed_decls — negative control proving empty Accum lowers cleanly.

Cascade table at sched/lower.rs:113-128 + per-row notation updated to mark the 3 cascade-aware variants with TASK-0208. Module doc upgraded from 'dormant gap' to 'dormant but structurally pinned'.

Behavior change for valid input: ZERO. The new clause is unreachable on today's variant set (sched-decl evaluation has no failure path). If/when a sched-decl-eval failure path is added, the cascade-aware check fires correctly + the existing K×M fixture stays as the structural regression pin.

Gate (cycle 44): just test 0 FAILED (2 new tests pass); just clippy clean (single_char_add_str lint hit + fixed in same edit); just e2e 88/70/0/18 unchanged.

Review-gate (parallel read-only): both qa-test-runner + mped-architect GO. No HIGH/MEDIUM findings; 3 LOWs (lower-bound assertion tightened by per-name guard; cascade dormancy honestly disclosed; intra-decl overlap explicitly documented) — all already addressed by the implementer's own honest-limits writeup.
<!-- SECTION:FINAL_SUMMARY:END -->
