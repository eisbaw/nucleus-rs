---
id: TASK-0064
title: Add Renode to flake for tier-3 runtime validation
status: To Do
assignee: []
created_date: '2026-05-17 23:24'
labels:
  - M10
  - infra
  - tooling
  - embedded
  - renode
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-3 (M10+) per PRD §10.3 uses Renode as the default runtime validation harness. nixpkgs has a 'renode' package (Mono-based). Add it under a separate devShell or behind a feature flag so the heavy Mono runtime is not pulled in for tier-1 development. CI job to spin up .resc scripts and diff UART output against reference.bin is a separate task.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 renode binary available in a dev shell
- [ ] #2 renode --version works
- [ ] #3 An example .resc script in the repo loads, runs to completion, and the harness can capture UART output
<!-- AC:END -->
