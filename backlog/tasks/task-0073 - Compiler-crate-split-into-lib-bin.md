---
id: TASK-0073
title: 'Compiler crate: split into lib + bin'
status: Done
assignee: []
created_date: '2026-05-17 23:31'
updated_date: '2026-05-22 20:57'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 59 tracker hygiene (2026-05-22). The task as filed (M0/M1 era) called for splitting nucleus/compiler/ into [lib] + [[bin]] inside one crate. The project's current architecture instead has the binary in a SEPARATE crate at nucleus/driver/ (with [[bin]] name='nucleus' in nucleus/driver/Cargo.toml). Compiler/ stays pure lib. The separation is arguably cleaner than [lib]+[[bin]] in one Cargo.toml — the boundary is enforced at the crate level, not just by file paths. Effectively obsolete-by-design; closing as Done.
<!-- SECTION:FINAL_SUMMARY:END -->
