---
id: TASK-0003
title: Set up starter justfile
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-17 23:36'
labels:
  - M0
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create justfile at repo root with the starter recipes from PRD §12.3: build, test, check, fmt, clippy, e2e, clean. Every recipe has a one-line comment. No example/schedule/backend-specific one-offs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 justfile at repo root has recipes: build, test, check, fmt, clippy, e2e, clean.
- [ ] #2 Every recipe has a one-line comment immediately above it.
- [ ] #3 No recipes for specific examples or schedule names.
- [ ] #4 Test: 'just --list' shows exactly the seven recipes.
- [ ] #5 Test: 'just check' and 'just fmt' both succeed in a fresh checkout.
- [ ] #6 Implementation notes record any recipes considered but rejected, and why.
- [ ] #7 Implementation notes record honest limitations (e.g. e2e binary is a stub at M0).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation notes for TASK-0003

### Design questions resolved

- **`cd nucleus && cargo ...` vs `cargo ... --manifest-path nucleus/Cargo.toml`.**
  Picked `cd nucleus && cargo ...`. Reasons:
    (a) Recipes read top-to-bottom as ordinary shell, no flag noise.
    (b) Most cargo subcommands resolve config / lockfile / target dir from
        the manifest's directory anyway -- `--manifest-path` works but
        feels like swimming upstream.
    (c) Just runs each recipe line in a fresh shell, so the `cd` does not
        leak between recipes.
  Trade-off: a developer running `just <recipe>` from inside `nucleus/`
  would see an extra `cd` step that no-ops because the path is relative
  to the justfile root, not cwd. Acceptable: justfile is invoked from the
  repo root by convention (where the justfile lives).

- **Default recipe.** Wired to `@just --list` (the conventional choice
  and explicitly suggested by the task brief). The `@` silences echo of
  the command itself; otherwise just prints `just --list` before its
  output, which is noise. Alternative considered: default to `build`,
  matching `make`'s convention. Rejected -- in a workspace where
  `build` takes non-trivial wall time, accidentally typing `just`
  shouldn't kick off a compile.

- **`set shell := ["bash", "-uc"]`.** Not used. The recipes are simple
  `cd && cmd` lines; just's default shell handles them fine. The PRD
  said "use set sparingly" and there is no current need.

- **Comments inside vs above recipes.** Above only, per PRD §12.3 and
  the justfile-hygiene rule. `just --list` picks up the comment line
  immediately above a recipe as its description, so this also yields
  the nice `just --list` output (verified -- see AC #4).

- **Workspace-relative paths in recipes.** Just resolves recipe cwd
  relative to the justfile's directory by default, so `cd nucleus`
  works regardless of where the user invokes `just`. Verified.

### Acceptance criteria status

- AC #1 (justfile at repo root has 7 recipes): MET. build, test, check,
  fmt, clippy, e2e, clean all present at /home/mpedersen/topics/mark_thesis/justfile.
- AC #2 (one-line comment above every recipe): MET. Each of the seven
  required recipes (and the default) has exactly one comment line above
  it. No trailing-comment forms.
- AC #3 (no example/schedule/backend-specific recipes): MET. Only the
  seven generic recipes + default. Explicit anti-bloat reminder added
  to the file header.
- AC #4 (just --list shows exactly seven recipes): MET MODULO DEFAULT.
  `just --list` shows the seven required recipes PLUS the default
  recipe -- which the task brief explicitly permits ("plus the default
  if you wired one"). The literal reading of AC #4 ("exactly the
  seven") is slightly violated by the default's presence; the brief's
  guidance takes precedence. Verified inside `nix develop`:
    Available recipes:
        build, check, clean, clippy, default, e2e, fmt, test
- AC #5 (just check and just fmt succeed): MET. Both run clean inside
  `nix develop`. Also verified: just build, just test, just clippy,
  just e2e (exits 0 on stub), just clean (removes target/, 15.7 MiB).
- AC #6 (notes record recipes considered but rejected): MET. None were
  considered. The PRD §12.3 reference shape is precisely the seven
  recipes; any tier-1/2/3 specific recipe would violate the anti-bloat
  rule. If a developer ever feels the urge to add `just run-stencil`,
  the right move is to add a `--example` flag to nucleus-e2e instead.
- AC #7 (limitations recorded): MET (see below).

### Honest limitations and scope cuts

- **`e2e` runs a stub binary.** At M0 nucleus-e2e is `fn main() {}`,
  so `just e2e` always exits 0 with no work done. The real
  (example x schedule x backend) matrix runner lands at M1+ when
  the first backend (pthreads-sync) exists. Filed as TASK-0075.

- **Clippy passed without `#[allow(...)]` patches.** The empty
  `fn main() {}` stubs trigger zero lints under
  `cargo clippy --workspace -- -D warnings` at rustc 1.83.0. No
  follow-up needed today; if a future rustc tightens lints on
  empty mains, that's caught by CI.

- **No `set shell` strictness.** Recipes inherit just's default sh.
  Acceptable for now (recipes are trivial); a future task could add
  `set shell := ["bash", "-uc"]` if pipefail behaviour matters.

- **No `just --check` or `just --fmt` in CI.** The justfile is short
  enough that hand-review is fine. Worth revisiting only if recipes
  proliferate (which the anti-bloat rule says they should not).

- **No environment variable handling.** No `CARGO_TARGET_DIR`,
  `RUSTFLAGS`, profile overrides etc. plumbed through. None of the
  current recipes need them. Adding them is trivial when a real
  reason appears.

- **The `cd && cmd` convention assumes a POSIX-ish shell.** Fine for
  the only supported dev platform (x86_64-linux per flake.nix). A
  Windows developer running just directly without WSL would need
  the manifest-path form. Out of scope.

- **The PRD.md working-tree modification is unrelated and was not
  touched by this task** (per the brief's "do not modify the PRD"
  rule). This is the same dirty state TASK-0001 already noted.

### Follow-up tasks filed

- TASK-0075 Wire up the real (example x schedule x backend) matrix
  runner in nucleus-e2e (replaces the stub binary).

### Commit

6d34289 infra(M0): add starter justfile (TASK-0003)
<!-- SECTION:NOTES:END -->
