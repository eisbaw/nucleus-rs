---
id: TASK-0002
title: Set up Cargo workspace skeleton
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-17 23:31'
labels:
  - M0
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create a minimal Cargo workspace under nucleus/. Workspace root Cargo.toml lists initial members: compiler crate (the nucleus binary) and an e2e crate (the differential test harness). PRD §12.2.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nucleus/Cargo.toml is a workspace with at least 'compiler' and 'e2e' members.
- [ ] #2 Each member crate compiles with 'cargo check --workspace' inside nix develop.
- [ ] #3 Workspace pins edition = 2021 and the MSRV is consistent with the Nix flake pin.
- [ ] #4 Test: 'just check' from repo root succeeds.
- [ ] #5 Implementation notes record design questions (e.g. whether to put backend crates in the workspace from M0 or only at M1).
- [ ] #6 Implementation notes record honest limitations (no clippy lints yet, no doc builds, no per-crate README).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation notes for TASK-0002

### Design questions resolved

- **Workspace layout: nucleus/ at repo root, members nested as siblings.**
  PRD §12.2 says \"one Rust workspace\" with backends as future siblings.
  Settled on:
    nucleus/Cargo.toml          (workspace root)
    nucleus/compiler/           (the `nucleus` binary)
    nucleus/e2e/                (the `nucleus-e2e` differential test harness)
  Alternative considered: putting Cargo.toml at the repo root. Rejected
  because the repo already houses non-Rust assets (mark_thesis_final44.pdf,
  nuc-nucleus/ design docs, backlog/, equivalence-by-hashing/). Nesting
  Rust under nucleus/ keeps `cargo` rooted on Rust-only territory and
  matches PRD diagrams that name the project `nucleus`.

- **Backends in workspace from M0 or M1?** Deferred to M1. PRD milestone
  table (§11) puts the first backend (pthreads-sync) at M1, not M0. Adding
  empty backend crates now would just be dead code and forces premature
  decisions about the backend trait shape. Workspace members list will grow
  one entry at M1 (pthreads-sync) and again at M3 (mp-tcp-bufsync), etc.

- **Library vs binary structure for `compiler`?** Plain binary for now.
  Eventually the e2e harness will want to call compiler internals in-process
  (cheaper than shelling out, lets us assert on intermediate IRs like the
  Petri net per PRD §8). Filed as TASK-0073 to do the lib+bin split when
  the first non-trivial pass lands at M1. Doing it now would be premature
  -- no internal API exists to expose yet, and a placeholder lib.rs with
  no exports is just clutter.

- **Crate names vs binary names.** Crate name = directory name
  (`compiler`, `e2e`) per Rust conventions. Binary names overridden via
  [[bin]] sections to `nucleus` and `nucleus-e2e` -- the names the
  justfile in PRD §12.3 already references and the names users will type.
  Trade-off: the crate/bin name mismatch can confuse newcomers; mitigated
  by an inline comment in each Cargo.toml.

- **MSRV / rust-version.** Not set in any Cargo.toml. flake.nix pins
  rustc 1.83.0 and TASK-0001 explicitly documents the flake as the single
  source of truth. Per the task brief, TASK-0066 will re-verify when the
  first Cargo.toml lands -- I have NOT created another duplicate task for
  that.

- **resolver = \"2\".** Required for edition 2021 workspaces to behave
  correctly. Set explicitly even though it's the default for edition-2021
  members, to make intent obvious.

- **license = \"MIT OR Apache-2.0\", publish = false.** Set in
  workspace.package and inherited. License is the Rust ecosystem default;
  publish=false is correct because none of these crates are meant for
  crates.io. Both are easy to change later if the project picks a
  different license.

- **Cargo.lock committed.** Standard practice for a workspace whose
  primary outputs are binaries (vs. a library). Locks dependency versions
  for reproducible builds; flake.nix pinning rustc covers the toolchain
  side. Currently lock file has only the two local crates (no third-party
  deps yet), but committing it now establishes the convention before
  dependencies arrive.

### Acceptance criteria status

- AC #1 (nucleus/Cargo.toml is a workspace with compiler+e2e members):
  MET. Both members declared explicitly.
- AC #2 (cargo check --workspace succeeds inside nix develop): MET.
  Verified:
    nix develop --command cargo check --workspace \\
      --manifest-path nucleus/Cargo.toml
  Output: \"Finished `dev` profile [unoptimized + debuginfo]\"; no errors.
  Also verified cargo build, and that ./nucleus/target/debug/nucleus
  and ./nucleus/target/debug/nucleus-e2e both exist and exit 0.
- AC #3 (edition = 2021 pinned; MSRV consistent with flake): MET in
  spirit but interpreted strictly. edition = \"2021\" is set in
  [workspace.package] and inherited via edition.workspace = true.
  MSRV is intentionally NOT duplicated in Cargo.toml per the task brief
  and PRD §12.1/§13; the flake is the single source of truth. \"Consistent
  with the Nix flake pin\" is interpreted as \"not contradicting the
  flake\", which holds trivially because Cargo.toml is silent on MSRV.
- AC #4 (just check from repo root succeeds): NOT VERIFIED. No
  justfile exists yet -- TASK-0003 / TASK-0074 owns that. I ran the
  underlying `cargo check --workspace --manifest-path nucleus/Cargo.toml`
  directly, which is what `just check` will wrap. The recipe and its
  green-light verification belongs in the justfile task.
- AC #5 (design questions recorded): MET (this note).
- AC #6 (limitations recorded): MET (next section).

### Honest limitations and scope cuts

- **No rustfmt.toml.** Workspace style is whatever rustfmt defaults to.
  Filed as TASK-0069.
- **No clippy.toml / [workspace.lints].** No project-wide lint policy.
  PRD §12.3 plans `cargo clippy --workspace -- -D warnings`; that's a
  recipe wrapper, not a policy file. Filed as TASK-0070.
- **No cargo-deny / cargo-audit.** No license, advisory, or source
  allowlist enforcement. Both tools are absent from the dev shell. Filed
  as TASK-0071.
- **No per-crate README.** Each crate has a header comment in its
  Cargo.toml pointing to the relevant PRD section, which is the minimum
  navigable. Proper READMEs filed as TASK-0072.
- **Compiler is a single bin, not lib+bin.** Discussed above. Filed as
  TASK-0073.
- **No justfile recipes.** Task brief did not ask me to create them, and
  TASK-0003 / TASK-0074 owns the justfile. The PRD §12.3 reference shape
  works against this workspace as-is, modulo a --manifest-path argument.
- **No backend crates.** Deferred to M1 by design (see above).
- **No tests.** Both stub main() bodies are empty; there's nothing to
  test yet. `cargo test --workspace` succeeds with \"0 tests run\".
- **No CI configuration.** Out of scope for this task.

### Follow-up tasks filed

- TASK-0069 rustfmt.toml workspace style config
- TASK-0070 clippy.toml + workspace lints policy
- TASK-0071 cargo-deny and cargo-audit setup
- TASK-0072 Per-crate README stubs for compiler/ and e2e/
- TASK-0073 Compiler crate: split into lib + bin
- TASK-0074 Justfile recipes for build/check/test/fmt/clippy/e2e/clean

### Commit

33fcd09 infra(M0): add Cargo workspace skeleton (TASK-0002)
<!-- SECTION:NOTES:END -->
