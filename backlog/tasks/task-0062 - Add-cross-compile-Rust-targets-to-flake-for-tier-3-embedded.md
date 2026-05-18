---
id: TASK-0062
title: Add cross-compile Rust target(s) to flake for tier-3 embedded
status: To Do
assignee: []
created_date: '2026-05-17 23:24'
labels:
  - M9
  - infra
  - tooling
  - embedded
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-3 (M9+) per PRD §7.3 will need at least thumbv7em-none-eabihf (Cortex-M7 / STM32H7) in the dev shell. fenix supports per-target rust-std via the 'targets' attribute on toolchainOf, e.g. (fenix.combine [...]). Also likely need probe-rs / probe-run / flip-link for flashing, and llvm-tools-preview component for binutils. Do this when M9 starts, not before — keeps the dev shell lean for M0-M6.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 flake.nix exposes a Rust toolchain that can build for thumbv7em-none-eabihf
- [ ] #2 nix develop --command cargo build --target thumbv7em-none-eabihf succeeds on a hello-world no_std crate
- [ ] #3 probe-rs (or equivalent) available in the dev shell for flashing
<!-- AC:END -->
