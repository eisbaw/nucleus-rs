---
id: TASK-0065
title: Wire flake check to run fmt + clippy once a Cargo project exists
status: To Do
assignee: []
created_date: '2026-05-17 23:24'
updated_date: '2026-05-19 04:48'
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
