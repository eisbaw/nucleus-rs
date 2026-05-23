---
id: TASK-0084
title: Rename compiler crate to nucleus-compiler
status: In Progress
assignee:
  - mark
created_date: '2026-05-18 00:05'
updated_date: '2026-05-23 20:07'
labels:
  - M1
  - infra
  - tooling
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
MPED architect review of TASK-0002 flagged: crate name 'compiler' is dangerously generic in grep terms. Once codebase grows, 'grep -rn compiler' will hit standard error messages, dep names, comments. Also closes the bin/crate-name-mismatch footgun (currently 'cargo run -p compiler' but 'cargo run --bin nucleus'). Renaming now is a cheap directory move + workspace-member edit + one Cargo.toml line; renaming later costs incrementally more per-import.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Crate directory renamed: nucleus/compiler/ -> nucleus/nucleus-compiler/.
- [ ] #2 Workspace Cargo.toml members list updated accordingly.
- [ ] #3 All imports/references in the codebase updated; cargo check + clippy + test all clean.
- [ ] #4 Bin name 'nucleus' (in nucleus-compiler/Cargo.toml) unchanged — users still type 'nucleus build ...'.
- [ ] #5 Test: 'just build && just test && just clippy' all green after the rename.
- [ ] #6 Implementation notes record any imports that needed touching and any breakage encountered.
- [ ] #7 Implementation notes record honest limitations (e.g. if some external doc or follow-up task references the old name, those need a follow-pass).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. git mv nucleus/compiler -> nucleus/nucleus-compiler (preserves history).
2. nucleus/nucleus-compiler/Cargo.toml: package.name & lib.name = nucleus-compiler; KEEP [[bin]] name = nucleus.
3. nucleus/Cargo.toml workspace members: 'compiler' -> 'nucleus-compiler'.
4. 7 Cargo.toml dep updates: backend-common, driver, test-common, e2e (if any), backends/{pthreads-sync,mp-tcp-bufsync,pthreads-async,mp-tcp-event}: compiler -> nucleus-compiler with new path.
5. Rust source: every 'use compiler::' / 'compiler::' qualified path -> nucleus_compiler:: (underscore). Use careful sed.
6. Mandatory doc-lie audit: grep -rn 'compiler::|\bcompiler\b' for leftover comments. Update doc-comments, code comments naming the crate-path.
7. Example reference Cargo.toml comments ('no dependency on the compiler crate') -> 'nucleus-compiler crate'.
8. README updates: compiler/README.md (now nucleus-compiler/README.md) + 'cargo test -p compiler' -> 'cargo test -p nucleus-compiler'.
9. cargo doc --workspace --no-deps gate for stale rustdoc links.
10. Full gate: just check, just clippy, just test, just e2e (88/70/0/18 unchanged), just determinism-check + 3 negatives, port-stress 20/20.
11. Backlog/docs audit for stale references (per-file decide: update vs historical record).
12. Commit per logical unit; no AI co-author credit.
<!-- SECTION:PLAN:END -->
