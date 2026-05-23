---
id: TASK-0067
title: Expose rustToolchain as flake output for cross-flake reuse
status: Done
assignee: []
created_date: '2026-05-17 23:28'
updated_date: '2026-05-23 20:51'
labels:
  - infra
  - tooling
  - M7
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect review of TASK-0001 flagged that rustToolchain is a let-binding, not a flake output. A parent flake importing this one cannot reuse the exact pinned toolchain without re-deriving. Cost: one line. Land when a second consumer appears (likely M7 CI when MPI builds want the same MSRV without entering nix develop).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 flake.nix exposes packages.${system}.rust-toolchain referring to the same fenix-derived toolchain currently let-bound.
- [ ] #2 Existing devShells.default keeps using the same binding (single source of truth).
- [ ] #3 Test: 'nix build .#rust-toolchain' produces the rustc binary used in 'nix develop'.
- [ ] #4 Implementation notes record design questions (e.g. should we also expose individual components like clippy as separate outputs).
- [ ] #5 Implementation notes record honest limitations (one toolchain only; if v3 needs nightly + stable side-by-side, will need restructure).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). The task description explicitly defines its trigger: 'Land when a second consumer appears (likely M7 CI when MPI builds want the same MSRV without entering nix develop).' Today rustToolchain has only one consumer (devShells.default in flake.nix). No second consumer exists. Reopen when M7/MPI CI or a sibling flake needs cross-flake reuse of the pinned toolchain — at that point the let-binding-to-flake-output promotion is a one-line change and AC#1-#5 are derivable from the new consumer's needs.
<!-- SECTION:FINAL_SUMMARY:END -->
