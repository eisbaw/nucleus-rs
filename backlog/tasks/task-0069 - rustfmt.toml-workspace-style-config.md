---
id: TASK-0069
title: rustfmt.toml workspace style config
status: To Do
assignee: []
created_date: '2026-05-17 23:30'
labels:
  - M0
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add a workspace-wide rustfmt.toml under nucleus/ so 'cargo fmt --all' is deterministic across contributors. Decide: edition = 2021 (matches workspace), max_width default vs 100, group_imports/imports_granularity, etc. Run inside nix develop. Acceptance: 'cargo fmt --all -- --check' passes on a freshly committed tree; rustfmt.toml committed at nucleus/rustfmt.toml or repo-root depending on which proves to apply uniformly.
<!-- SECTION:DESCRIPTION:END -->
