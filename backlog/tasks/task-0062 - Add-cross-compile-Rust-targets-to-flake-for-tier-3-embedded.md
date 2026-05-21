---
id: TASK-0062
title: Add cross-compile Rust target(s) to flake for tier-3 embedded
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:24'
updated_date: '2026-05-21 17:35'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Mirror TASK-0064 pattern (commit 632d98c + e8086ad). Add ONE new opt-in shell `.#embedded`:

1. Compose `embeddedToolchain = fenix.combine [ rustToolchain  fenix.targets.thumbv7em-none-eabihf.stable.rust-std ]` (verify exact path via nix eval; fallback `.toolchainOf { channel; sha256 }.withComponents ["rust-std"]` if attr path differs).
2. `devShells.embedded = pkgs.mkShell { packages = [ embeddedToolchain  fenix.rust-analyzer  pkgs.just  pkgs.probe-rs-tools ]; shellHook = ""; }`. probe-rs binary lives in probe-rs-tools (pkgs.probe-rs is an alias; main program comes from -tools). Use `probe-rs-tools` explicitly for clarity.
3. Inline comment cites PRD §7.3 + TASK-0062, mirrors the renode shell comment style.
4. AC#2 verification: temp no_std crate under nucleus/target/cross-compile-verify/ (not committed) built with `nix develop .#embedded -c cargo build --target thumbv7em-none-eabihf`. Don't commit the temp crate; record in notes.
5. AC#3: `nix develop .#embedded -c probe-rs --version`.
6. Gate: default + renode shells unchanged (just test 539/0/2, just e2e 36/29/0/7, just ci exit 0, nix develop .#renode -c renode --version unchanged). `nix flake check` + `nix flake show` show three shells.

Scope-split rules: if probe-rs unavailable/heavy/broken -> file as TASK-0062.01. If temp-crate build needs nontrivial linker config -> TASK-0062.02. Honest PARTIAL preferred over rushed complete.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Progress 2026-05-21 (commit 9787412)

Mirrored TASK-0064 opt-in shell pattern. Added `devShells.embedded` exposing thumbv7em-none-eabihf rust-std + probe-rs-tools on top of the existing MSRV-pinned host toolchain.

### Verification matrix

| Check | Result |
|---|---|
| `nix flake check` | PASS (3 shells evaluate) |
| `nix flake show` | default + renode + embedded all visible |
| default `just test` | 539/0/2 (unregressed) |
| default `just e2e` | 36/29/0/7 (unregressed) |
| default `just ci` | exit 0 (unregressed) |
| `.#renode -c renode --version` | v1.16.1.0 (unregressed) |
| `.#embedded -c rustc --version` | rustc 1.83.0 (90b35a623 2024-11-26) |
| `.#embedded -c rustc --print target-list \| grep thumbv7em-none-eabihf` | match |
| `.#embedded -c probe-rs --version` | probe-rs 0.31.0 (git commit: crates.io) |
| AC#2 no_std cross-build | PASS (ELF 32-bit LSB executable, ARM, EABI5) |

### AC status

- [x] #1 flake exposes a Rust toolchain that builds for thumbv7em-none-eabihf (via `.#embedded`).
- [x] #2 `cargo build --target thumbv7em-none-eabihf` succeeds on a hello-world no_std crate inside `.#embedded`. Temp crate at `nucleus/target/cross-compile-verify/` (gitignored, removed after verification).
- [x] #3 probe-rs available via `pkgs.probe-rs-tools` (the package that actually ships the CLI binary; `pkgs.probe-rs` in current nixpkgs is an alias).

### Notes / gotchas

- `pkgs.probe-rs` IS available as an alias but `pkgs.probe-rs-tools` is the canonical package shipping the CLI binary in current nixpkgs. Picked the explicit form.
- `fenix.combine` is the documented union-of-toolchains primitive. Reused the same `rustChannel` + `sha256` for both the host and the per-target toolchainOf — fenix's sha256 is the manifest hash, not target-specific, so same pin = same hash.
- The AC#2 temp crate needed an empty `[workspace]` table in its Cargo.toml because nucleus/Cargo.toml is the parent workspace root (cargo searches upward by default). Also needed a minimal linker script (`dummy.x`) with `MEMORY { FLASH }` + `ENTRY(_start)` + a `.text` section to satisfy rust-lld; an empty link line failed with `error: undefined symbol: _stack_start`. The crate is not committed; record-keeping only.

### Forward-carry

- TASK-0047 (M9 embedded skeleton, In Progress / blocked-on-this) — unblocked.
- TASK-0048 (M10 STM32H7 shim) — unblocked for its toolchain prerequisite.
- TASK-0223 (.resc + UART) — unaffected (renode-side, separate concern).
- TASK-0068 (plan tiered devShells) — this task completes the embedded slice; renode + embedded together cover tier-3's two opt-in shells.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE — `nix develop .#embedded` exposes thumbv7em-none-eabihf rust-std + probe-rs-tools on top of the existing MSRV-pinned 1.83.0 host toolchain. Default + renode shells unregressed (539/0/2 unit, 36/29/0/7 e2e, ci exit 0, renode v1.16.1.0). All 3 ACs verified end-to-end inside the new shell: rustc reports the target, probe-rs --version returns, a no_std hello-world cross-builds to a real ARM ELF. Mirrors TASK-0064's opt-in pattern so the heavier closure stays out of tier-1 CI. Commit 9787412.
<!-- SECTION:FINAL_SUMMARY:END -->
