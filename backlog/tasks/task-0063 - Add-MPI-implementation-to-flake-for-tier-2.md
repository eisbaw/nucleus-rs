---
id: TASK-0063
title: Add MPI implementation to flake for tier-2
status: Done
assignee: []
created_date: '2026-05-17 23:24'
updated_date: '2026-05-31 06:42'
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
- [x] #1 openmpi (or mpich) available in nix develop
- [x] #2 mpirun --version works inside the shell
- [x] #3 rsmpi crate compiles against the provided MPI implementation (smoke test crate)
- [x] #4 Decision documented in flake.nix: which MPI impl and why
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

## REOPENED + LANDED at M7 entry (2026-05-31, orchestrator-direct in-thread, commit af832ec)

All 4 ACs met + verified by the read-only review gate (qa-test-runner + mped-architect, both GO):

- AC#1/#2: `devShells.mpi` added (basePackages ++ [openmpi, llvmPackages.libclang, clang]); `nix develop .#mpi -c mpirun --version` => `mpirun (Open MPI) 5.0.10`. openmpi 5.0.10 is fully in the binary cache (21 MiB download, zero source builds).

- AC#3: committed permanent smoke fixture `tests/mpi/rsmpi-smoke/` (standalone [workspace], mirrors tests/renode/*) + `just check-mpi-smoke`. rsmpi `mpi` 0.8.1 compiles 17s; `mpiexec --oversubscribe -n 2/-n 4` give correct rank dispatch + a verified blocking Send/Recv hop (sentinel 0x5EEDF00D re-checked on receive).

- AC#4: OpenMPI-over-MPICH decision documented in the flake comment (cache-populated; working mpicc + ompi pkg-config that build-probe-mpi consumes; localhost mpirun).

## Gotchas / lessons (forward-carried)

- bindgen (mpi-sys build.rs) needs libclang on NixOS: LIBCLANG_PATH + BINDGEN_EXTRA_CLANG_ARGS (-isystem clang-resource-dir + glibc.dev/include) in the .#mpi shell. Without them stddef.h/stdint.h don't resolve. It worked first try here.

- libffi (speculated in this task's DESCRIPTION as a build dep) was NOT actually required — rsmpi 0.8.1's mpi-sys bindgen needs only libclang + clang resource-dir/glibc headers; libffi-sys/libffi came in transitively and built fine without an explicit flake entry. Do NOT cargo-cult libffi into the shell from the old description text.

- Default-shell isolation verified: zero openmpi/ucx/pmix/libfabric paths in the default devShell closure. `nix flake check` green for all 4 shells (default/mpi/embedded/renode).

- e2e UNCHANGED at 350/293/0/57/0 (this change touches zero nucleus/ workspace files).

- SCOPE LIMIT carried to TASK-0045: the smoke de-risks POINT-TO-POINT only (Push/Wait -> Send/Recv). Event::Sync/Barrier (-> MPI_Barrier, or Comm_split+barrier for non-full-world participant subsets) is UNPROVEN by this foundation; the backend must add a barrier smoke arm when it lowers collectives.
<!-- SECTION:FINAL_SUMMARY:END -->
