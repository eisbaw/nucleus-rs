---
id: TASK-0446
title: Standing wired-path negative arm for the --with-mpi tier coverage guard
status: Done
assignee:
  - '@mark'
created_date: '2026-06-04 10:52'
updated_date: '2026-06-04 23:59'
labels:
  - M7
  - validation
  - e2e
  - mpi
  - test-hardening
dependencies:
  - TASK-0444
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0444 added the --with-mpi tier gate to required_coverage_gaps, and a unit test (mpi_required_cell_unplanned_is_a_gap_under_with_mpi) proves the PURE function bites. But the WIRED path (run_inner -> required_coverage_gaps hard-fail) has no standing negative arm for the mpi tier — the existing required-coverage-check-negative recipe + NUC_REQUIRED_COVERAGE_NEGATIVE seam are tier-1-only by construction (the synthetic cell anchors on manifest.required.first(), a pthreads-sync cell; see the documented orthogonality note in maybe_inject_required_coverage_negative). So a future refactor could sever the mpi-tier coverage hard-fail with unit tests + e2e green. This is the SAME standing-bite gap TASK-0168 closed for tier-1. Low priority (the mpi gate is run rarely, the unit test covers the logic), but file it so it is not lost. Option: extend the injection seam to optionally anchor on an mpi backend + run under --with-mpi, or add a driver-call test seam.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a standing check proves the --with-mpi required-coverage hard-fail path still bites (not just the pure-function unit test)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED orchestrator-direct (harness/justfile infra — implementer subagents refuse these per feedback-spawned-agents-refuse-code-edits + TASK-0444/0445 precedent). DESIGN: tier-aware injection — maybe_inject_required_coverage_negative gained with_mpi; anchors the synthetic typo cell on the first required entry whose is_mpi_backend matches the active tier (tier-1 default; mpi under --with-mpi). New required-coverage-check-negative-mpi recipe (.#mpi, --with-mpi + NUC_REQUIRED_COVERAGE_NEGATIVE=1, same TASK-0188 belt+suspenders contract). CHEAP: the gap-check Err fires before any cell build (verified ordering: inject 4807 -> mpiexec probe -> plan_cells [discovery only] -> required_coverage_gaps -> Err), so no rsmpi cross-builds. VERIFIED (orchestrator-run, observed, ALL reproduced independently by qa-test-runner): (1) just required-coverage-check-negative-mpi => OK exit 0, harness errors naming backend=mpi-blocking schedule=<sentinel>, GAP_DETECTED=1 — wired fail-then-pass under the mpi tier. (2) NEGATIVE CONTROL (temp tier-1 anchor under --with-mpi): recipe RED exit 1, GAP_DETECTED=0 (synthetic silently filtered out by the mpi-tier coverage filter) — DEMONSTRATES the recipe catches a silently-skipped mpi cell, not a tautology; reverted clean (no TEMP marker, architect confirmed). (3) tier-1 recipe still OK exit 0 (pthreads-sync) — no regression. (4) cargo test -p e2e 101+1+1 pass (+1 req_cov_inject_anchors_on_mpi_tier_under_with_mpi); clippy clean. (5) default just e2e 427/364/0/63/0 exit 0 UNCHANGED (env-gated strict no-op). REVIEW GATE both GO. Architect P2 (mpi arm weaker "standing" than tier-1 since not in just ci — consistent w/ whole mpi tier being a manual gate) -> filed TASK-0447 (bundling meta-recipe). P3 trap-quoting divergence -> folded comment (commit 9cd7914). P3 docstrings verified honest. Commits 0b3bb9a + 9cd7914.

FINAL: AC#1 met — a standing wired-path negative arm (required-coverage-check-negative-mpi) proves the --with-mpi required-coverage hard-fail still bites, demonstrated via a wired fail-then-pass AND a negative control (RED on mis-anchor). Both reviewers GO; P2 residual (CI does not auto-run the mpi arm) tracked as TASK-0447. This hardens the TASK-0444/0445 9-of-9 mpi-matrix arc just shipped. Commits 0b3bb9a + 9cd7914.
<!-- SECTION:NOTES:END -->
