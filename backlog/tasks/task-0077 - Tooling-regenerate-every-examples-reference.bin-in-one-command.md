---
id: TASK-0077
title: 'Tooling: regenerate every example''s reference.bin in one command'
status: To Do
assignee: []
created_date: '2026-05-17 23:40'
labels:
  - tooling
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add a single entry point that re-runs each examples/NN-name/reference/ regeneration command in sequence and writes the resulting reference.bin files in place. Two shapes considered: (a) a 'just regen-references' recipe that shells out per example; (b) a '--regen-refs' flag on the nucleus-e2e binary that walks examples/ and dispatches. Prefer (b) once the e2e harness is non-trivial; (a) is acceptable as a stub at M0–M1. Used by maintainers when a kernel body changes and references must move in lockstep (docs/reference-impl-policy.md §3, §4). Must fail loudly if any reference command exits non-zero, and must not commit results — that is a human review step.
<!-- SECTION:DESCRIPTION:END -->
