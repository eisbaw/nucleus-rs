---
id: TASK-0118
title: Capability-matrix check on TransferPolicy at codegen time
status: Done
assignee: []
created_date: '2026-05-18 01:44'
updated_date: '2026-05-22 21:37'
labels:
  - M1
  - compiler
  - codegen
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per TASK-0018 spec, transfer_inject deliberately does NOT validate that the chosen backend can satisfy a TransferPolicy (async, buffer>1, notify=event). The backend isn't picked at the pass. Once a backend with a capabilities.toml is wired through to the codegen pass, walk every XferPlaceholder and reject combinations the backend lacks. PRD §6.3.4 says this must be a hard error, not a silent fallback. Errors must name the offending data symbol, the requested option, and the backend.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 63b tracker hygiene (2026-05-22). Structurally met by pre-session work:

- nucleus/compiler/src/capabilities.rs::check_schedule_compat is the capability-matrix check on TransferPolicy. It returns Vec<CapMismatch> — accumulates ALL mismatches (not first-error-only).

- nucleus/driver/src/main.rs:341-348 calls check_schedule_compat AFTER lowering BEFORE codegen. On mismatch: emits an error message naming the backend + each mismatch.

- check_schedule_compat covers transport, notify, supports_async, supports_buffer, max_buffer, worker_classes, memory_regions per the capabilities.toml schema (docs/capabilities-toml.md).

The structural verification: 13/pipeline_parallel × {pthreads-sync, mp-tcp-bufsync} currently SKIP with the message 'TASK-0042 / TASK-0210: async + buffer=3 + notify=event not supported by ...' — that's the capability check firing on a real cell.

PRD §6.3.4 'hard error, not silent fallback' invariant honored. No source changes; closing as tracker hygiene.
<!-- SECTION:FINAL_SUMMARY:END -->
