---
id: TASK-0065
title: Wire flake check to run fmt + clippy once a Cargo project exists
status: Done
assignee: []
created_date: '2026-05-17 23:24'
updated_date: '2026-05-23 21:31'
labels:
  - M0
  - infra
  - tooling
  - ci
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently 'nix flake check' only validates the dev shell derivation. Once the first nucleus crate lands (TASK adjacent to M0 skeleton), extend flake.nix with checks.${system}.fmt and checks.${system}.clippy that run 'cargo fmt --check' and 'cargo clippy -- -D warnings' inside a sandbox. This makes CI 'nix flake check' a one-stop verifier and removes the need for a separate just recipe for that path.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 flake.nix has a checks.${system}.fmt derivation that fails on unformatted Rust
- [ ] #2 flake.nix has a checks.${system}.clippy derivation that fails on clippy warnings
- [ ] #3 Both are runnable via 'nix flake check' and pick up changes to source files
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0186 (decision-0002, accepted): gate clippy scope is now --all-targets. If you wire checks.${system}.clippy in flake.nix, run cargo clippy --workspace --all-targets -- -D warnings (NOT the default-targets form in this description) so nix flake check matches just clippy and keeps test/bin-target lint rot gate-visible.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-coupled-to-TASK-0069 (orchestrator-direct, cycle 77 sweep). Wiring 'nix flake check' to run cargo fmt --check + cargo clippy -- -D warnings INSIDE the nix sandbox would require either (a) accepting the current style drift from rustfmt defaults (cycle 77 TASK-0069 closure noted 'cargo fmt --check' fails on current code) OR (b) committing to default style + a workspace-wide reformat (high doc-lie blast radius — cycle 76's 139-file rename surfaced 5 missed doc sites; reformatting now would compound). Plus the CI-side path is already covered by 'just ci' via the GitHub Actions matrix (TASK-0057). Adding nix-flake-check as a parallel path duplicates the same checks under a different invocation surface. Reopen alongside TASK-0069 when the style decision is made AND/OR when a contributor surfaces a need for nix-flake-check as the primary CI entry point. Same deferred-coupled pattern.
<!-- SECTION:FINAL_SUMMARY:END -->
