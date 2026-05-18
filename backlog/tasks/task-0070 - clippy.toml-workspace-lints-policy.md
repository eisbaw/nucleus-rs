---
id: TASK-0070
title: clippy.toml + workspace lints policy
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
Define workspace-wide clippy lint policy. PRD §12.3 justfile already plans 'cargo clippy --workspace -- -D warnings'. Decide which lints to allow/deny project-wide (e.g. clippy::pedantic? clippy::nursery? deny missing_docs?), pin in clippy.toml and/or [workspace.lints.clippy] in Cargo.toml. Acceptance: clippy with the chosen policy passes on the M0 skeleton and on the first real compiler code.
<!-- SECTION:DESCRIPTION:END -->
