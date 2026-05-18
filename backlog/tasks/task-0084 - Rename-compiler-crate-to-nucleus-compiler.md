---
id: TASK-0084
title: Rename compiler crate to nucleus-compiler
status: To Do
assignee: []
created_date: '2026-05-18 00:05'
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
