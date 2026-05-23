---
id: TASK-0068
title: Plan tiered devShells (default / mpi / embedded) before M7
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-17 23:28'
updated_date: '2026-05-23 14:31'
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

Cycle 68 (2026-05-23) — AC#1 LANDED, AC#3 STRUCTURALLY MET, AC#2 PENDING TASK-0063 (MPI).

AC#1 (one-line comment near devShells.default): LANDED. flake.nix now has a 6-line preamble above devShells.default explaining 'this is DELIBERATELY MINIMAL; tier-specific heavy closures live in opt-in sibling shells (.#renode, .#embedded, future .#mpi)'. Forward-points TASK-0063 for the future .#mpi shell.

AC#3 (M10: devShells.embedded exists AND default does NOT pull Renode): STRUCTURALLY MET. devShells.embedded already lives in flake.nix; devShells.default = basePackages only (no renode, no embeddedToolchain). 'nix flake show' confirms three shells: default, embedded, renode.

AC#2 (M7: devShells.mpi exists AND default does NOT pull MPI): PENDING TASK-0063. Cannot be closed until MPI is in the flake. The new comment in flake.nix now explicitly forward-points TASK-0063 so the .#mpi shell is filed-when-implemented.

AC#4 (nix flake show lists shells; default closure does not include heavy deps): PARTIALLY MET. Today default/embedded/renode all listed; .#mpi placeholder. Verified by 'nix flake show' (3 devShells under x86_64-linux) and 'nix flake check' (all three derivations evaluate clean).

AC#5/#6 (design questions / honest limits in notes): inputsFrom vs separate mkShell calls — chose separate mkShell calls so each shell can have a distinct toolchain (embedded's toolchain REPLACES rustToolchain, not augments it). Honest limit: contributors must remember to enter the right shell; no auto-detection.

Cycle 68 is in-thread (no implementer subagent) — pure flake.nix doc edit + audit. Stays In Progress because AC#2 (MPI) is unimplementable until TASK-0063 lands.
<!-- SECTION:NOTES:END -->
