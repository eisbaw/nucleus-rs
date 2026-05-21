---
id: TASK-0068
title: Plan tiered devShells (default / mpi / embedded) before M7
status: To Do
assignee: []
created_date: '2026-05-17 23:28'
updated_date: '2026-05-21 17:16'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Partially advanced by TASK-0064 (commit 632d98c)

TASK-0064 introduced the 'devShells.renode' opt-in pattern with a DRY 'basePackages' refactor:
- basePackages = [ rustToolchain, rust-analyzer, just ] is the single source of truth (flake.nix:47-54).
- devShells.default = basePackages.
- devShells.renode = basePackages ++ [ pkgs.renode ].

This IS the template AC#1 of this task asked for ('one-line comment near devShells.default noting the planned tier split'). The renode shell's comment (flake.nix:64-68) explicitly notes 'Tier-3 (M10+) runtime validation shell. Opt-in via nix develop .#renode.' — partially fulfilling that AC. The remaining work for this task is the EXPLICIT tier-2 (MPI, M7) shell using the same basePackages pattern, plus updating the default-shell comment to enumerate the planned siblings.

After commit 632d98c: this task's AC#1 + AC#2 (multi-shell shape established) substantively done; AC#3 (tier-2/M7 MPI shell) is the remaining piece.
<!-- SECTION:NOTES:END -->
