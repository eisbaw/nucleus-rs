---
id: TASK-0073
title: 'Compiler crate: split into lib + bin'
status: To Do
assignee: []
created_date: '2026-05-17 23:31'
updated_date: '2026-05-18 00:05'
labels:
  - M2
  - compiler
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently nucleus/compiler/ is a single binary crate. Once real compiler code lands, the e2e harness will want to invoke compiler internals in-process (faster than shelling out, and lets the harness assert on intermediate IRs like the Petri net). Refactor compiler into a library crate (src/lib.rs exporting the public API) plus a thin src/bin/nucleus.rs that just wires argv -> lib. Trigger: do this as soon as the first non-trivial pass lands (probably M1 alongside the first parser), not before.
<!-- SECTION:DESCRIPTION:END -->
