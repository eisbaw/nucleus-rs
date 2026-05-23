---
id: TASK-0063
title: Add MPI implementation to flake for tier-2
status: To Do
assignee: []
created_date: '2026-05-17 23:24'
updated_date: '2026-05-23 14:31'
labels:
  - M7
  - infra
  - tooling
  - mpi
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-2 (M7+) per PRD §7.2 needs OpenMPI or MPICH plus rsmpi's build dependencies (clang/libclang for bindgen, libffi). nixpkgs.openmpi is the obvious pick; consider mpich as an alternative. Add MPI to the dev shell as a separate package list activated when M7 starts so M0-M6 builds stay lean. Localhost MPI CI per PRD §10.2 requires mpirun on PATH.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 openmpi (or mpich) available in nix develop
- [ ] #2 mpirun --version works inside the shell
- [ ] #3 rsmpi crate compiles against the provided MPI implementation (smoke test crate)
- [ ] #4 Decision documented in flake.nix: which MPI impl and why
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0068 cycle 68: when MPI lands in this flake, also add 'devShells.mpi' (sibling of .#renode and .#embedded) that adds the MPI packages to basePackages without touching devShells.default. Update flake.nix:71-77 comment to drop the 'future' qualifier on .#mpi. AC#2 of TASK-0068 ('M7 mpi devShell exists, default does NOT pull MPI') closes the moment this lands. Pattern to mirror: devShells.renode at flake.nix:83-87.
<!-- SECTION:NOTES:END -->
