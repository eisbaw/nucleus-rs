---
id: TASK-0386
title: >-
  Emitted check-count static NUC_CHECK_COUNT_&lt;ident&gt; trips
  non_upper_case_globals when loop_var is lowercase
status: To Do
assignee: []
created_date: '2026-05-31 06:03'
labels:
  - codegen
  - check-loop
  - cosmetic
  - backend-common
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pre-existing wart surfaced (newly visible on tier-1) by TASK-0369's check_count cells. `backend-common/src/check_frame.rs::emit_count_static` emits `static NUC_CHECK_COUNT_{ident}` where `ident = sanitize_loop_var(loop_var)`. For a lowercase loop var like `i` this yields `NUC_CHECK_COUNT_i`, which trips rustc's `non_upper_case_globals` warning in the GENERATED crate (warning-only — does NOT fail the e2e `cargo build`, which has no `-D warnings`; so it is cosmetic, not a gate failure). TASK-0369 made it newly visible because it added the first TIER-1 on_violation=count cells (the embedded fixtures already had it via AtomicU32). \n\nFix options: (a) uppercase-mangle the ident in the static name (`NUC_CHECK_COUNT_I`) — CAUTION: changes emitted bytes, so the embedded_check_count golden/determinism fixtures + the new tier-1 check_count cells must be re-verified bit-identical after; or (b) emit `#[allow(non_upper_case_globals)]` on the static. (b) is lower-blast-radius. Either way: a shared check_frame.rs change touches pthreads-sync/pthreads-async/openmp-rs/mp-tcp-bufsync single-worker emit + the embedded backend; run `just e2e` + `just determinism-check` after. Found by mped-architect review of TASK-0369 (cycle-222).
<!-- SECTION:DESCRIPTION:END -->
