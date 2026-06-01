---
id: TASK-0405
title: >-
  Optional: per-backend bite tests for EmitError I/O variants (cross-backend
  silent-sibling sweep)
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 06:21'
updated_date: '2026-06-01 10:26'
labels:
  - hardening
  - testing
  - silent-sibling
  - prove-the-check-bites
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 TASK-0404 follow-up. TASK-0404 added bite tests for KernelsReadFailed/OutputCreateFailed/WriteFailed on the CANONICAL backend (pthreads-sync), proving the variants at VARIANT granularity. The same 3 fs::*().map_err(EmitError::X) sites recur verbatim in all 10 backends (pthreads-async, mp-tcp-{bufsync,event,poll}, mp-uds-event, openmp-rs, mpi-{blocking,nonblocking}, embedded-pattern). A cycle-236 grep audit DISCHARGED the silent-sibling risk: every backend uses .map_err(EmitError::...) for these ops -- NONE uses unwrap/expect/?-on-raw-io -- so no backend panics on I/O failure. This task would add per-backend bite tests for completeness (SITE granularity per the TASK-0397 variant-vs-site lesson).

LOW / OPTIONAL: the sites are mechanical identical copies and the grep audit already proves consistency; per-backend tests would be duplication with low marginal yield. Only pursue if a backend diverges its I/O handling (e.g. a future backend that builds paths differently) OR to harden against a future refactor that could re-introduce an unwrap. Each backend embeds its emit entry differently (some lib-only, some bin), so the test scaffolding is not uniform -- mirror each backends existing emit/tests harness.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0410/0411 (cycle-237): the just-ci gate does NOT build docs, so any change touching a doc-linked symbol (removing/narrowing a pub item, removing an error variant referenced by [`...`]) must run cargo doc --workspace --no-deps before/after and diff the generated-N-warning sum (baseline 10). For bite/sibling-sweep tasks that ADD tests this is usually moot, but if the work removes or renames a symbol carrying an intra-doc-link, add the cargo-doc diff to the gate.
<!-- SECTION:NOTES:END -->
