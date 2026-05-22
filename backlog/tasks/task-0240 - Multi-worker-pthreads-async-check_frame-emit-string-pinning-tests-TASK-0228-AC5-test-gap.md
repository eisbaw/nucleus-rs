---
id: TASK-0240
title: >-
  Multi-worker pthreads-async check_frame emit-string pinning tests (TASK-0228
  AC#5 test gap)
status: To Do
assignee: []
created_date: '2026-05-22 07:35'
labels:
  - M4
  - backend
  - test-coverage
dependencies:
  - TASK-0228
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0228 Wave B-2 (cycle 26) wired the multi-worker check_frame substrate (file-scope AtomicU64 statics via emit_count_static, host-thread Drop guards via emit_count_guard_local, per-worker Log/Count branches via emit_log_branch / emit_count_branch). The helpers are CALLED from Plan::emit, but no in-tree multi-worker pthreads-async fixture carries a check_loop directive, so the emit-string shape is UNTESTED for this backend.\n\nMirrors the structure that TASK-0236 set up for pthreads-sync + mp-tcp-bufsync's multi-worker check_frame: a synthetic 2-worker per_worker fixture with one Event::Loop carrying check_frame = Some(CheckFrame{loop_var, latency_max_ns, on_violation: ViolationKind::{Panic,Log,Count}}) for each of the three violation kinds, then string-pin the emitted main.rs against the expected (a) file-scope AtomicU64 static + reporter struct (Count only), (b) per-Count-loop Drop guard local in fn main, (c) per-iteration measurement + on-violation branch with the right idents.\n\nAcceptance:\n- nucleus/backends/pthreads-async/tests/check_frame_emit.rs created (mirror pthreads-sync's check_frame_codegen.rs + mp-tcp-bufsync's check_frame_emit.rs).\n- Three tests pin the Panic/Log/Count multi-worker emit shape.\n- The third-backend Final Summary on TASK-0222 (extract check_frame templates) can close once this lands: 'All three tier-1 backends consume the shared helpers AND test-pin their emit shape; drift is now structurally + test-detected for all three.'
<!-- SECTION:DESCRIPTION:END -->
