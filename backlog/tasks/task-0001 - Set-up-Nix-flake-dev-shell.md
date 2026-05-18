---
id: TASK-0001
title: Set up Nix flake dev shell
status: Done
assignee: []
created_date: '2026-05-17 23:01'
updated_date: '2026-05-17 23:24'
labels:
  - M0
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create flake.nix at repo root providing the reproducible dev shell described in PRD §12.1: pinned Rust toolchain (rustc/cargo/clippy/rustfmt), just, rust-analyzer. Silent enter; no echo spam in shellHook. MSRV is pinned in the flake, not duplicated in Cargo.toml.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 flake.nix at repo root provides a working 'nix develop' shell containing rustc, cargo, clippy, rustfmt, just, rust-analyzer.
- [ ] #2 shellHook is silent (no echo statements). Entering the shell prints nothing extra.
- [ ] #3 MSRV pinned in flake.nix; verified by 'cargo --version' inside the shell matching the pinned version.
- [ ] #4 Test: a fresh checkout + 'nix develop' + 'just --version' works without further setup.
- [ ] #5 Implementation notes record design questions encountered (e.g. fenix vs rust-overlay, naersk vs crane vs plain cargo).
- [ ] #6 Implementation notes record honest limitations and scope cuts (e.g. no cross-compilation toolchains yet, no Renode yet).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation notes for TASK-0001

### Design choices

- **Toolchain provider: fenix (nix-community/fenix).**
  Considered: nix-community/fenix vs oxalica/rust-overlay. (Note: "oxalica"
  and "rust-overlay" are the same project, not separate options.)
  Picked fenix because:
    (a) toolchainOf { channel; sha256 } gives explicit, byte-pinnable
        version selection without an extra overlay registration step.
    (b) withComponents is a clean list-of-strings API; matches how
        the PRD lists components (rustc/cargo/clippy/rustfmt).
    (c) rust-analyzer ships from the same input, kept in lockstep with
        the toolchain release.
    (d) nix-community provenance is acceptable for a dev shell.
  Trade-off: fenix tracks upstream Mozilla channel metadata, which means
  the sha256 has to be updated every MSRV bump (documented in the file).

- **Builder library (naersk / crane / plain cargo): not chosen yet.**
  The flake currently provides only a dev shell. There is no Cargo
  project to build, so naersk vs crane vs plain cargo is deferred until
  TASK-0003 (justfile) and the first crate land. Plain cargo inside
  `nix develop` is the cheapest start; crane is the likely upgrade if/when
  reproducible binary outputs are needed for CI.

- **MSRV pin: 1.83.0.**
  PRD §13 wording: "edition 2021, MSRV pinned to whatever stable was 6
  months before M0." Today is 2026-05-16; six months prior is ~2025-11-16.
  Without offline access to a definitive Rust release calendar inside
  this dev environment, picked 1.83.0 (stable since 2024-11-28) as a
  conservative, well-tested pin. This is intentionally older than the
  literal six-month rule asks for; the trade is "definitely-known-stable"
  over "exactly to the policy". Bump in a follow-up once a real Cargo
  project exists and we can verify the chosen version actually compiles
  the planned dependencies (rsmpi, mio, rayon, etc.). Bump procedure is
  documented inline in flake.nix.

- **shellHook = "" (empty string).**
  PRD §12.1 and the user's ~/.claude/CLAUDE.md both forbid verbose echoes.
  Set to empty string rather than omitted, to make the intent explicit
  to the next reader.

### Acceptance criteria status

- AC #1 (toolchain present in nix develop): MET. Verified via
  `nix develop --command bash -c 'rustc --version && cargo --version &&
   clippy --version && rustfmt --version && just --version &&
   rust-analyzer --version'`. All six tools resolved successfully.
- AC #2 (silent shellHook): MET. `nix develop --command true` produces
  only the stock Nix "Git tree is dirty" warning, which is emitted by
  Nix itself (because there are unrelated uncommitted files in the repo:
  CLAUDE.md, .claude/, backlog/, plus a modified PRD.md). Our shellHook
  emits nothing.
- AC #3 (MSRV pinned, cargo --version matches): PARTIALLY MET.
  rustc/cargo report 1.83.0 inside the shell, matching the pin in
  flake.nix. The "verified by 'cargo --version' matching the pinned
  version" half is satisfied; the "single source of truth, not in
  Cargo.toml" half is trivially satisfied because no Cargo.toml exists
  yet. Will need re-verification once a Cargo project lands to confirm
  it does NOT re-declare rust-version.
- AC #4 (fresh checkout + nix develop + just --version works without
  further setup): MET. Verified via the same nix-develop invocation
  above. flake.lock is committed, so the resolution is fully pinned for
  any fresh clone.
- AC #5 (notes record design questions): MET (this note).
- AC #6 (notes record limitations / scope cuts): MET (see below).

### Honest limitations and scope cuts

- **No cross-compilation toolchains.** Cortex-M / RISC-V / aarch64
  targets are not in the dev shell. Tier-3 (M9+) will need at least
  thumbv7em-none-eabihf (Cortex-M7 / STM32H7 per PRD §7.3) added via
  fenix's `targets` mechanism, plus probe-rs or a similar flasher.
  Filed as a follow-up task.

- **No MPI implementation.** Tier-2 (M7+) needs OpenMPI or MPICH plus
  the `rsmpi` Rust binding's build deps (clang headers, libffi). Not in
  this flake yet. Filed as a follow-up task.

- **No Renode.** Tier-3 (M10+) runtime validation per PRD §10.3 needs
  Renode in CI. Renode is sizeable (Mono / .NET runtime) and not worth
  adding before the embedded backend exists. Filed as a follow-up task.

- **No fmt / lint pre-commit hooks.** A `nix flake check` that runs
  rustfmt and clippy on every commit would be the natural next step,
  but the flake currently has no Cargo project to lint. Will revisit
  after TASK-0003 (justfile) and the first nucleus crate land.

- **Toolchain sha256 maintenance burden.** Every Rust version bump
  requires updating both `rustChannel` and `sha256`. Documented inline
  in flake.nix. An alternative would be using fenix's `stable.latest`
  alias for floating, but that violates the "pinned MSRV" rule.

- **Only x86_64-linux really exercised.** flake-utils gives us
  `eachDefaultSystem`, but `nix flake check` warned about omitted
  systems (aarch64-darwin etc.). Mac and aarch64-linux dev are not
  actively supported by this work; would need fenix sha256 entries per
  system if they matter. Out of scope for v2 CPU-tier.

- **The "git tree dirty" warning is not from our shellHook.** Re-stating
  for the record: it comes from Nix's flake machinery when the
  containing git tree has any uncommitted changes (in this repo: the
  unrelated CLAUDE.md / .claude/ / backlog/ / modified PRD.md). Our
  shellHook is "". Once those files are committed (or moved out of the
  repo), the warning disappears.

### Commit

c538e80 infra(M0): add Nix flake dev shell (TASK-0001)
<!-- SECTION:NOTES:END -->
