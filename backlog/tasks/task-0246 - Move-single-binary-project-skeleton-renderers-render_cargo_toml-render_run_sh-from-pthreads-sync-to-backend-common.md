---
id: TASK-0246
title: >-
  Move single-binary project-skeleton renderers (render_cargo_toml +
  render_run_sh) from pthreads-sync to backend-common
status: To Do
assignee: []
created_date: '2026-05-22 11:38'
labels:
  - tech-debt
  - architecture
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 37 (TASK-0244) extracted the backend-common crate but LEFT two project-skeleton renderers in pthreads-sync:
- render_cargo_toml: emits the single-binary Cargo.toml template (used by pthreads-sync + pthreads-async).
- render_run_sh: emits run.sh launcher (used by pthreads-sync + pthreads-async).

mp-tcp-bufsync does NOT consume these — it has its own multi-process variants (render_cargo_toml(bin_names) + render_run_sh_single) because multi-process needs different shape.

The cycle-37 narrative claimed 'the one semantic inter-backend arrow that survives is render_single_worker_main'. Architect review-gate (cycle 37) flagged this slightly oversells — there are TWO surfaces still routed through pthreads-sync: render_single_worker_main (genuinely semantic delegation — pthreads-async's emit() for used_workers <= 1 IS pthreads-sync's main.rs, byte-identical) AND render_cargo_toml/render_run_sh (project-skeleton string templates, no semantic reason to live in pthreads-sync — they could just as well live in backend-common::project_skeleton::single_binary).

Cleaner architecture would put render_cargo_toml + render_run_sh in backend-common::project_skeleton (single-binary variant). Then ONLY render_single_worker_main remains in pthreads-sync — which IS semantically pthreads-sync-owned (the straight-line emitter is a pthreads-sync-specific shape that pthreads-async happens to delegate to for the degenerate single-worker case).

Acceptance:
- backend-common::project_skeleton module created with render_cargo_toml + render_run_sh (single-binary variant).
- pthreads-sync, pthreads-async both consume from backend-common; pthreads-sync no longer owns these.
- mp-tcp-bufsync's multi-process variants stay in mp-tcp-bufsync (different shape).
- e2e tally + cross-backend bit-identical invariants unchanged.

Low priority — same architectural smell as TASK-0244 fixed for the rest of the surface; deferred because cycle 37 was already a 1654 LoC refactor and bundling more would have outsized the cycle.
<!-- SECTION:DESCRIPTION:END -->
