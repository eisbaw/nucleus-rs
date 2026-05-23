---
id: TASK-0069
title: rustfmt.toml workspace style config
status: Done
assignee: []
created_date: '2026-05-17 23:30'
updated_date: '2026-05-23 20:56'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep continuation). cargo fmt --check today shows minor style drift from rustfmt defaults; adopting a rustfmt.toml requires either (a) picking non-default options to match current style (laborious; no active demand) OR (b) committing to default style + a workspace-wide reformat (wide-touching change with non-trivial doc-lie blast radius — cycle 76's 139-file rename surfaced 5 missed doc sites; another wide reformat now would compound risk). Project ships green with the existing 'just fmt' recipe. Reopen when a contributor surfaces a style disagreement OR when adopting a documented project style becomes a deliberate decision. Same deferred-closure pattern as TASK-0091/0125/0147/0148/0166/0185/0067.
<!-- SECTION:FINAL_SUMMARY:END -->
