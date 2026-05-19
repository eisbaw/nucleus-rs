---
id: TASK-0074
title: Justfile recipes for build/check/test/fmt/clippy/e2e/clean
status: To Do
assignee: []
created_date: '2026-05-17 23:31'
updated_date: '2026-05-19 04:48'
labels:
  - M0
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add a top-level justfile per PRD §12.3 reference shape: build, test, check, fmt, clippy, e2e, clean. Recipes are one-liners; the e2e recipe runs 'cargo run --release --bin nucleus-e2e'. All recipes must respect the workspace at nucleus/ (use --manifest-path nucleus/Cargo.toml or place justfile under nucleus/ — pick one and document). Per CLAUDE.md, common commands belong in the justfile. Acceptance: 'just check', 'just build', 'just clippy' all succeed inside nix develop on the M0 skeleton.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0186 (decision-0002, accepted): the justfile clippy recipe already exists and is cargo clippy --workspace --all-targets -- -D warnings (not the bare -- -D warnings reference shape). If this task revisits/normalises the recipe set, preserve the --all-targets scope on clippy; do not regress to default targets. The justfile lives at repo ROOT and recipes cd into nucleus/ (nix flake is at repo root) — this resolves the "pick one and document" ask in this task description.
<!-- SECTION:NOTES:END -->
