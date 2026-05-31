---
id: TASK-0063
title: Add MPI implementation to flake for tier-2
status: In Progress
assignee: []
created_date: '2026-05-17 23:24'
updated_date: '2026-05-31 06:33'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
M7 has started (focus: TASK-0045 mpi-blocking backend), so this prerequisite is REOPENED from its DEFERRED parking per its own Final Summary ("Reopen at M7 entry"). Orchestrator-direct, in-thread (repo signals implementer subagents refuse code edits; this is flake/justfile/fixture infra).

Plan:
1. Add devShells.mpi to flake.nix = basePackages ++ [openmpi, libclang, clang] with LIBCLANG_PATH + BINDGEN_EXTRA_CLANG_ARGS so rsmpi's mpi-sys bindgen resolves mpi.h + system headers. Mirror the .#renode/.#embedded opt-in pattern. (AC#1)
2. Verify mpirun --version inside .#mpi. (AC#2)
3. Commit a permanent smoke crate tests/mpi/rsmpi-smoke/ (standalone workspace, like tests/renode/*) that exercises Init/Finalize + Comm_rank/size + blocking Send/Recv, and a `just check-mpi-smoke` recipe that builds it under .#mpi and runs mpiexec -n 2 with a fail-loud sentinel assertion. (AC#3)
4. Document the OpenMPI-over-MPICH decision in the flake comment. (AC#4)
5. Verify default shell does NOT pull MPI (closes TASK-0068 AC#2).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0068 cycle 68: when MPI lands in this flake, also add 'devShells.mpi' (sibling of .#renode and .#embedded) that adds the MPI packages to basePackages without touching devShells.default. Update flake.nix:71-77 comment to drop the 'future' qualifier on .#mpi. AC#2 of TASK-0068 ('M7 mpi devShell exists, default does NOT pull MPI') closes the moment this lands. Pattern to mirror: devShells.renode at flake.nix:83-87.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-M7 (orchestrator-direct, cycle 77 sweep). Labeled M7, infra, tooling, mpi. Description: 'Add MPI to the dev shell as a separate package list activated when M7 starts so M0-M6 builds stay lean.' M7 (Tier-2 MPI blocking backend) not in progress. TASK-0068 cycle 68 documented the tiered devShell plan (default / mpi / embedded) and the flake.nix carries a comment reserving .#mpi as the future entry point. Adding openmpi + clang + libffi to the flake NOW would bloat every dev-shell entry across M0-M6 builds for zero current benefit. Reopen at M7 entry — at that point implement the .#mpi devShell + the rsmpi build deps + the localhost-mpirun CI hook. Same deferred-to-milestone pattern as TASK-0050/0051/0054/0164/0165/0192/0223.
<!-- SECTION:FINAL_SUMMARY:END -->
