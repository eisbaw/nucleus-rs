---
id: TASK-0444
title: M7 mpi-blocking into e2e differential matrix (examples 1-3)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-04 09:44'
updated_date: '2026-06-04 10:51'
labels:
  - M7
  - backend
  - validation
  - e2e
  - mpi
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Bring the tier-2 mpi-blocking backend into the COUNTED e2e differential matrix for examples 1-3, closing the M6->M11 lacuna where M7's CI surface was skipped (strategic review 2026-06-04). Today mpi-blocking value-correctness is proven by 'just check-mpi' (byte-exact vs reference.bin) but is NOT a counted matrix cell; the e2e matrix covers 7 of 9 backends. This converts the 'we have 9 backends' claim from partial-facade toward full-coverage (8 of 9; embedded-pattern stays out-of-matrix by design via renode-multimcu-gate). Design: a separate 'mpi_backends' tier in e2e-matrix.toml + a '--with-mpi' harness flag that runs ONLY that tier (mutually exclusive with the default tier-1 run, mirroring renode-multimcu-gate), entered via a new 'just e2e-mpi' recipe under the .#mpi dev shell. The flag HARD-FAILS if mpiexec is absent (no silent skip). The 3 cells (01-elementwise-add/naive, 02-split-add/split n=2, 03-reduction/distributed n=5) are exactly what check-mpi already proves byte-exact, compared against the same reference.bin oracle the tier-1 differential uses.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 mpi_backends tier added to e2e-matrix.toml; default 'just e2e' UNCHANGED at 427 (tier-1 only); mpi cells out of scope in default shell
- [x] #2 --with-mpi harness flag scopes the run to the mpi tier ONLY; lockstep across plan_cells + required_coverage_gaps + fault_assert_orphans (no silent coverage gap)
- [x] #3 --with-mpi HARD-FAILS at startup if mpiexec absent (anti-silent-skip), with a clear message pointing to 'just e2e-mpi'
- [x] #4 3 mpi-blocking [[required]] cells (01/naive, 02/split, 03/distributed) byte-exact vs reference.bin under 'just e2e-mpi'
- [x] #5 new 'just e2e-mpi' recipe enters .#mpi and runs the tier; documented as out-of-default-ci sibling of renode-multimcu-gate
- [x] #6 existing 364 tier-1 differential passes unchanged; 02-split-add + 14-hearing-aid renode byte-exact preserved
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Slice landed (orchestrator-direct in-thread, per repo signal that implementer subagents refuse infra/harness edits; see TASK-0063/0045 precedent + memory feedback-spawned-agents-refuse-code-edits).

DESIGN: separate mpi_backends tier in e2e-matrix.toml + --with-mpi harness flag that runs that tier INSTEAD of the default backends tier (mutually exclusive, focused gate; sibling of renode-multimcu-gate). Shared Manifest::is_mpi_backend predicate used in lockstep by plan_cells + required_coverage_gaps + fault_assert_orphans. --with-mpi ALSO triggers tight-tier planning (like --milestone) so only the DECLARED cells run, not all ~60 example×schedule informational cells. Startup mpiexec probe HARD-FAILS if absent (anti-silent-skip). New just e2e-mpi recipe enters .#mpi.

VERIFIED:
- just e2e-mpi => 3/3 PASS byte-exact vs reference.bin, required-fail 0, exit 0. Cells: 01-elementwise-add/naive (-n1), 02-split-add/split (-n2 SPMD), 03-reduction/distributed (-n5 SPMD tree-reduce). ~27s/cell (rsmpi cross-build dominates).
- --with-mpi from DEFAULT shell (no mpiexec) => hard error exit 1 (anti-silent-skip guard fires).
- cargo test -p e2e => 102 pass (5 new tier-gate tests + extended manifest_actual_file_parses; fixed required_counts_strictly_grow_per_milestone to scope top-milestone to tier-1, since M7 mpi cells are on the orthogonal --with-mpi axis).
- clippy --workspace --all-targets => clean.
- Full default just e2e (427 baseline) re-run IN PROGRESS to confirm mpi cells out-of-scope.

GOTCHA (forward-carried): --with-mpi without tight-tier scoping planned 61 cells (all example×schedule on mpi-blocking as informational), ~27min. Tight-tier (OR with --milestone path) fixes it to the 3 declared. Any future mpi cell must be added as an explicit [[required]]/[[skip]] declaration to be run.

POST-REVIEW FOLD (cycle close): architect P3.1 (is_mpi_backend docstring overstated that plan_cells calls it — plan_cells uses active_backends; reworded to the two-helper/one-field framing) + P3.2 (documented the required-coverage-negative seam is tier-1-only by construction) folded in-thread (doc-only; cargo test -p e2e re-green 102 pass).

ENV NOTE: during the parallel review gate the root disk hit 100% (concurrent e2e runs + retained failed-run target/e2e-matrix scratch from the killed 61-cell exploratory mpi run); qa-test-runner returned NO-GO solely on an ENOSPC-aborted e2e (build/clippy/test 1374/test-release 1372 all PASS). Reclaimed space (rm target/e2e-matrix + target/mpi-m7 leftovers) and re-ran default e2e CLEAN post-edit: 427/364/0/63/0 exit 0. Authoritative.

AC#6 renode portion: satisfied BY CONSTRUCTION — the diff touches only e2e harness backend-selection logic + e2e-matrix.toml + justfile; zero backend/codegen files, so 02-split-add/14-hearing-aid renode output is byte-unchanged. Not re-run (would re-download the ~1-2GB renode closure + risk ENOSPC again) — flagged honestly rather than fake-verified.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE (commit c22c8f8). mpi-blocking is now a COUNTED e2e differential backend for examples 1-3 (8 of 9 backends in-matrix; embedded-pattern stays out via renode-multimcu-gate by design). New mpi_backends tier + --with-mpi harness flag (runs the mpi tier INSTEAD of tier-1, tight-tier scoped to declared cells, mpiexec hard-fail probe) + just e2e-mpi recipe (.#mpi). Shared is_mpi_backend/active_backends tier predicate in lockstep across plan_cells/required_coverage_gaps/fault_assert_orphans. VERIFIED: default e2e 427/364/0/63/0 exit 0 (UNCHANGED, mpi out of scope); e2e-mpi 3/3 byte-exact vs reference.bin (01/naive -n1, 02/split -n2, 03/distributed -n5) exit 0; --with-mpi default-shell hard-fail exit 1; cargo test -p e2e 102 pass; workspace test 1374 / test-release 1372 / clippy clean. Architect GO (P3 doc fixes folded). All 6 ACs met.
<!-- SECTION:FINAL_SUMMARY:END -->
