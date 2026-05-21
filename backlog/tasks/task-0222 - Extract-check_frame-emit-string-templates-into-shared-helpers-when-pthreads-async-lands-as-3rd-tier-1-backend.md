---
id: TASK-0222
title: >-
  Extract check_frame emit-string templates into shared helpers when
  pthreads-async lands as 3rd tier-1 backend
status: To Do
assignee: []
created_date: '2026-05-21 16:57'
labels:
  - tech-debt
  - M4
  - backend
dependencies:
  - TASK-0042.01
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0052.04 cycle): four emit-string templates are currently DUPLICATED between pthreads-sync and mp-tcp-bufsync (static AtomicU64 decl, per-loop guard local in fn main, Log eprintln branch, Count fetch_add branch). The commit-message claim 'No drift between backends' overstates structural prevention (the shared helpers cover the collector/struct-emitter/sanitizer; the four templates above are verbatim writeln! macros, drift-detection is test-as-tripwire). Tests pin both backends today so drift WOULD surface, but a third tier-1 backend (pthreads-async per TASK-0042.01) pushes past the two-readers-can-hold-it-in-their-head threshold.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Identify the 4 textually-duplicated emit-string templates (static decl, guard local, Log eprintln, Count fetch_add).
- [ ] #2 Extract into pub helpers in pthreads-sync (or a sibling 'backend-common' crate): emit_count_static, emit_count_guard_local, emit_log_branch, emit_count_branch.
- [ ] #3 Three backends (pthreads-sync + mp-tcp-bufsync + pthreads-async) consume the helpers; the existing tests in compiler/tests/check_frame_codegen.rs + backends/mp-tcp-bufsync/tests/check_frame_emit.rs continue to pin emit-string shape; one new test file covers pthreads-async.
<!-- AC:END -->
