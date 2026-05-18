---
id: TASK-0068
title: Plan tiered devShells (default / mpi / embedded) before M7
status: To Do
assignee: []
created_date: '2026-05-17 23:28'
labels:
  - infra
  - tooling
  - M7
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect review of TASK-0001 flagged that PRD §12.1's 'tier-specific tools added when their tier lands' is invisible in the current flake structure. A single devShells.default will force every contributor to download MPI + Renode closures when those land. Plan the split before piling them in.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 flake.nix has a one-line comment near devShells.default noting the planned tier split, so the next contributor doesn't pile MPI+Renode in thoughtlessly.
- [ ] #2 When M7 (MPI) lands, devShells.mpi exists and devShells.default does NOT pull MPI.
- [ ] #3 When M10 (Renode) lands, devShells.embedded exists and devShells.default does NOT pull Renode.
- [ ] #4 Test: 'nix flake show' lists default/mpi/embedded at M10; closure size of default does not include heavy tier-2/3 deps.
- [ ] #5 Implementation notes record design questions (e.g. composition via inputsFrom vs separate mkShell calls; how to share the toolchain).
- [ ] #6 Implementation notes record honest limitations (contributors must remember to enter the right shell; no auto-detection).
<!-- AC:END -->
