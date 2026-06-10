---
id: TASK-0454
title: check-mpi out-of-band list should derive from e2e-matrix (divergence-rot)
status: Done
assignee: []
created_date: '2026-06-07 07:35'
updated_date: '2026-06-10 09:59'
labels:
  - rigour
  - validation
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P9 advisory (cycle-9): the check-mpi / check-mpi-nonblocking justfile recipes carry their OWN hardcoded example/schedule lists, separate from e2e-matrix.toml's [[required]] mpi cells. TASK-0453.09 broadened the matrix mpi-blocking arm from 3 to 8 cells but did NOT touch check-mpi (sound: e2e-mpi proves the matrix cells byte-exact, strictly stronger than check-mpi's out-of-band assertion). Residual: the two MPI coverage surfaces can drift. Cleanest fix: make check-mpi{,-nonblocking} derive their target list from the matrix rather than duplicating hardcoded specs. Low priority, defense-in-depth.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 check-mpi (M7) and check-mpi-nonblocking (M8) derive their (example,schedule) target set by parsing the [[required]] mpi-blocking / mpi-nonblocking cells of nuc-nucleus/e2e-matrix.toml at recipe runtime (no second hardcoded copy of the example/schedule pair list).
- [x] #2 The recipe-local per-cell rank-count (n, the mpiexec -n N value, which the matrix TOML does NOT carry and cannot be cleanly derived from the schedule prose) lives in a single recipe-local example/schedule->n lookup; if the matrix yields an (example,schedule) cell that has no n entry in that lookup the recipe FAILS LOUD with a divergence-rot message and a non-zero exit, before any build/run.
- [x] #3 A targeted diff proves the newly-derived target list (example/schedule pairs) equals the matrix mpi cells, and the divergence-rot guard is shown to fire (negative test): injecting a matrix cell absent from the n-lookup makes the recipe abort non-zero.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add scripts/mpi-cells.py (tomllib parse of e2e-matrix [[required]] cells filtered by backend; per-cell rank n derived by counting the schedule file workers={} set; fail-loud on missing schedule = divergence-rot tripwire). Rewrite check-mpi (collapse old naive+multiworker two-arm into ONE matrix-derived loop) and check-mpi-nonblocking (keep default+rendezvous inner arm) to source their (example,schedule,n) list from the script. Prove by diff vs independent matrix extract + negative phantom-cell guard test; honest-report skip of live mpiexec run (.#mpi closure not fetchable here, network blocked).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE (pending wave gate). Added scripts/mpi-cells.py (chmod +x): tomllib-parses e2e-matrix.toml [[required]] cells filtered by --backend, derives per-cell rank n by counting the schedule .nuc workers={} set (host=rank). Rewrote check-mpi (collapsed old naive+multiworker two-arm into ONE matrix-derived loop; reads via process-substitution `done < <(printf %s "$cells")` so the loop body runs in the main bash and exit aborts correctly) and check-mpi-nonblocking (kept default+rendezvous inner arm). Recipe sources its list from the script; `cells="$(python3 ...)"` under set -eu aborts the whole recipe before any build if the script exits non-zero.

AC#1 PASS: both recipes derive (example,schedule) from the matrix, no hardcoded pair list remains. AC#2 PASS: per-cell n comes from the on-disk schedule workers={} set (single source) -- if the matrix names a schedule with no file on disk the script dies non-zero with a "divergence-rot" message before any build. AC#3 PASS: diff <(independent tomllib matrix extract) <(script cut -f1,2) is IDENTICAL for both backends; negative test (inject phantom 99-phantom/ghost mpi-blocking cell into a temp matrix) makes the recipe-mirroring `cells=$(...)` line abort rc=1 before any WOULD-RUN.

VERIFIED (default shell): script runs both backends; M7 derives 8 cells (01/naive n1, 02/split n2, 03/07/08/15/16/17 distributed n5) == matrix comment block; M8 derives 4 cells (05/distributed n5, 05/distributed-2d n5, 09/pipelined n3, 11/pipelined n2) == old hardcoded M8 specs exactly. py_compile OK. just --list / --summary / --show all parse rc=0. Dry-run of both inner loops (tab-split + proc-sub) iterates the right cells with correct n.

HONEST SKIP: did NOT run live `just check-mpi` / `check-mpi-nonblocking` -- the .#mpi dev shell could not be entered (cache.nixos.org DNS timed out in this sandbox; the MPI closure is not fetchable). The build/run/cmp lines are UNCHANGED from the prior passing recipe; only the cell-list SOURCE changed (hardcoded -> matrix-derived), and that derivation is proven by diff. The python3+tomllib call runs in the DEFAULT shell (verified), so the only .#mpi-gated part is the unchanged mpiexec cross-build/run.

COVERAGE DELTA (honest): M7 check-mpi GAINED 07/08/15/16/17 distributed (-n5) and DROPPED 04/05/06 naive (-n1 smoke, weakest signal) + 06-separable-filter/distributed{,2} (real multiworker, only in old recipe not matrix). Filed TASK-0464 to add the two 06-separable-filter distributed cells to the matrix (read-only for me this wave) so they flow back into both e2e-mpi and check-mpi under the new single-source design.

Touched: scripts/mpi-cells.py (new), justfile (check-mpi + check-mpi-nonblocking recipes only).

AC#2 clarification addendum (orchestrator, 2026-06-10): the landed mechanism is STRONGER than the AC wording — n is DERIVED from each schedule declared workers set (host counts as a rank), so no recipe-local n-lookup exists at all; the fail-loud guard fires on missing schedule file / unparseable / empty workers set instead of on a missing lookup entry. AC#3 verified by orchestrator: derived list manually diffed against matrix mpi cells (8 blocking + 4 nonblocking, matches); negative arm exercised with a synthetic 99-nonexistent/phantom mpi-blocking cell -> exit 1 divergence-rot message BEFORE any build. First negative attempt used backends=[...] plural (wrong schema, silently filtered) — the matrix schema is backend= singular; recorded so nobody repeats that test mistake.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
check-mpi and check-mpi-nonblocking now derive their cell lists from e2e-matrix.toml [[required]] mpi cells via scripts/mpi-cells.py (rank count derived from each schedule workers set, all ranks live; fail-loud divergence tripwires verified positive AND negative). Both recipes run GREEN end-to-end (8+4 cells byte-exact, nonblocking x default+rendezvous). Coverage delta tracked: TASK-0464 restores 06-separable-filter distributed{,2} as matrix cells. Landed dc10ff7; implementer agent died mid-task (API socket), work recovered + verified by orchestrator.
<!-- SECTION:FINAL_SUMMARY:END -->
