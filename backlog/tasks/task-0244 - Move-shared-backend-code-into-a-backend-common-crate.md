---
id: TASK-0244
title: Move shared backend code into a backend-common crate
status: To Do
assignee: []
created_date: '2026-05-22 09:42'
labels:
  - tech-debt
  - architecture
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 31 (TASK-0239) lifted the shared multi-worker event-walker into pthreads-sync/src/multi_worker_walker.rs (634 LoC, pub from pthreads-sync). pthreads-async now imports via pthreads_sync::multi_worker_walker::*. This is a deliberate trade-off — pthreads-async already depends on pthreads-sync for TASK-0222 helpers and TASK-0238 NameTables — but it leaks pthreads-sync's module structure and creates a backwards-looking dependency arrow (async -> sync) that is not semantically real (they are siblings, neither is the parent).

Same architectural smell as TASK-0238 (NameTables that semantically belonged in compiler, not pthreads-sync), now resolved by moving it. The same move should apply here.

Proper home: a backend-common (or pthreads-common) crate carrying:
- multi_worker_walker (the shared event-walker)
- RenderCtxPub + render_*_pub helpers (already pub from pthreads-sync, same arrow problem)
- The shared check_frame template helpers (emit_count_static, emit_count_guard_local, emit_log_branch, emit_count_branch, collect_count_check_frames, sanitize_loop_var, emit_count_reporter_struct, CountCheckLoop)
- rust_type_of, render_array_init_for, rust_scalar_type_pub

Then pthreads-sync, pthreads-async, mp-tcp-bufsync all depend on backend-common (no inter-backend dependencies).

Deferred from TASK-0239 — getting de-dup landed first was the priority; the crate move is bounded but mechanical. A future M5+ backend (TASK-0042.02 mp-tcp-event) would be the natural forcing function, but doing it preemptively keeps architectural clarity.

Acceptance:
- New crate nucleus/backend-common/ exists with the listed exports.
- pthreads-sync, pthreads-async, mp-tcp-bufsync depend on backend-common (no inter-backend deps).
- e2e tally + cross-backend bit-identical invariants unchanged.
<!-- SECTION:DESCRIPTION:END -->
