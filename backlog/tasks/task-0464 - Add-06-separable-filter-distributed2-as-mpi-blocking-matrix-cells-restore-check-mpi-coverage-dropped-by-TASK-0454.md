---
id: TASK-0464
title: >-
  Add 06-separable-filter/distributed{,2} as mpi-blocking matrix cells (restore
  check-mpi coverage dropped by TASK-0454)
status: Done
assignee: []
created_date: '2026-06-10 09:13'
updated_date: '2026-06-10 13:07'
labels:
  - rigour
  - validation
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0454 made check-mpi derive its cell list from e2e-matrix.toml mpi-blocking [[required]] cells. Net coverage delta: it GAINED 07/08/15/16/17 distributed (-n5, stronger) but DROPPED two genuine multi-worker cells that were ONLY in the old hardcoded recipe list and are not in the matrix: 06-separable-filter/distributed (n=5) and 06-separable-filter/distributed2 (n=5). (It also dropped 04/05/06 naive -n1 smoke, which is the weakest signal -- -n1 hides Send/Recv ordering bugs per the matrix comment -- not worth restoring.) Correct fix under the new single-source design: add the two 06-separable-filter distributed schedules as [[required]] mpi-blocking cells (milestone M7) in nuc-nucleus/e2e-matrix.toml; they then flow into BOTH just e2e-mpi (counted differential) AND just check-mpi automatically. Verify byte-exact under mpiexec --oversubscribe -n 5 first (run check-mpi after adding to confirm the derivation picks them up). e2e-matrix.toml was read-only for the TASK-0454 agent (being restructured in another wave), hence this follow-up.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Two new [[required]] cells exist in nuc-nucleus/e2e-matrix.toml with example="06-separable-filter", schedule="distributed" and "distributed2", backend="mpi-blocking" (singular string), milestone="M7" — matching the existing mpi-blocking cell block convention.
- [x] #2 Both schedules are verified sync-only (no async/buffer/notify transfer demands) so mpi-blocking can admit them; the w<->w tmp transfer in distributed2 is supported because mpi-blocking has star_topology_host_mediation=false (direct rank-to-rank Send/Recv).
- [x] #3 just e2e-mpi passes with zero failures and the two new cells counted (totals line reported verbatim), AND python3 scripts/mpi-cells.py nuc-nucleus/e2e-matrix.toml nuc-nucleus/examples mpi-blocking lists 06-separable-filter/distributed and /distributed2 each at n=5.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Onboard: verify both 06 distributed schedules are sync-only (grep transfer directives) + mpi-blocking supports w<->w (capabilities star_topology=false). Add two [[required]] mpi-blocking cells (milestone M7) for 06-separable-filter/distributed and /distributed2 to nuc-nucleus/e2e-matrix.toml, matching existing cell convention (backend SINGULAR string). PROVE: just e2e-mpi totals line (zero failures, +2 cells) and python3 scripts/mpi-cells.py confirms both new cells appear at n=5. README declares only naive/blocked as required tier-1 cells -> README NOT touched.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
VERIFIED. Added two [[required]] mpi-blocking (singular string) M7 cells for 06-separable-filter/distributed and /distributed2 to nuc-nucleus/e2e-matrix.toml, plus updated the EIGHT->TEN cell-count narrative in the matrix comment block. README.md NOT touched (it declares only naive/blocked as required tier-1 cells; the distributed schedules are not README-declared). Both schedules are sync-only (transfer ... : sync; no async/buffer/notify), so mpi-blocking admits them; distributed2 carries a genuine worker<->worker tmp transfer which mpi-blocking supports natively (capabilities.toml star_topology_host_mediation=false / host_data_relay=false -> direct rank-to-rank Send/Recv), unlike the mp-tcp star backends that SKIP it. PROOF: just e2e-mpi (4 runs) -> final CLEAN totals: "total: 14   pass: 14   fail: 0   skipped: 0   required-fail: 0"; both new cells PASS byte-exact 4/4 times (06/distributed 32.5s, 06/distributed2 19.3s last run; also PASS in 3 prior runs). Cell count 12->14 confirms the matrix-driven harness picked up both. mpi-cells.py derivation lists 06-separable-filter/distributed and /distributed2 each at n=5. GOTCHA: a concurrent agent (TASK-0455.07 WireShape refactor) repeatedly broke shared backend-common mid-run (WaitSlice undeclared -> E0433 -> E0308/E0599 across runs), failing the trailing cells in 3 of 4 runs; those are NOT my cells and NOT in my ownership. The full-green totals line above was captured in a clean window between their edits.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Two 06-separable-filter mpi-blocking [[required]] M7 cells added (distributed + distributed2); coverage the TASK-0454 derivation exposed as matrix-absent is restored and now flows into check-mpi automatically via mpi-cells.py (10 blocking cells derived). distributed2 = FIRST matrix cell exercising mpi-blocking native worker<->worker transfer on a whole-array fan-out; byte-exact. Clean-tree confirmation at the wave gate: e2e-mpi 14/14/0. False SKIP comment introduced alongside the cells reworded in fold-in 9f3434b (review P2.3). Landed 40db8f2; architect GO.
<!-- SECTION:FINAL_SUMMARY:END -->
