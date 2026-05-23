---
id: TASK-0257
title: >-
  Lift multi_binary project skeleton from mp-tcp-bufsync + mp-tcp-event into
  backend-common::project_skeleton::multi_binary
status: To Do
assignee: []
created_date: '2026-05-23 23:36'
labels:
  - tech-debt
  - architecture
  - backend-common
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect-review F6 of TASK-0042.05 (cycle 79): mp-tcp-event's lib.rs:360-410 (render_cargo_toml + render_run_sh_single + wrap_single_worker for single-worker path) and multi_worker.rs:render_cargo_toml/render_run_sh are each near-byte-copies of mp-tcp-bufsync's homologous helpers. With mp-tcp-event Stage 3 landed, the duplication is now 3-way:
1. mp-tcp-bufsync (single source today).
2. mp-tcp-event single-worker (delegated single-binary).
3. mp-tcp-event multi-worker (multi-binary mio reactor variant).

backend-common already houses project_skeleton (single_binary). Add a sibling module project_skeleton::multi_binary with the multi-binary Cargo.toml + multi-binary run.sh (NUC_RENDEZVOUS_DIR setup + EXIT trap + per-worker launch loop) so both mp-tcp-* backends consume the same emit. Risk: any drift in the byte output of mp-tcp-bufsync would invalidate every cached SHA256 in e2e-matrix.toml — verify byte-identical re-emit before and after.

Acceptance:
1. project_skeleton::multi_binary { render_cargo_toml(bin_names), render_run_sh(workers, host) } exists in backend-common, parameterised so both backends consume.
2. mp-tcp-bufsync's inline render_run_sh + render_cargo_toml call the lift; emits byte-identical bytes (verify with a determinism-check round-trip).
3. mp-tcp-event single-worker + multi-worker arms call the lift; byte-identical to today.
4. just e2e 88/73/0/15/0 preserved (no required-fail regression).
5. just determinism-check PASS.
6. The 3-way duplication is removed; the only remaining cross-backend arrow is the deliberate render_single_worker_main delegation (semantic, not infra).

Estimated scope: ~500 LoC touch. Defer behind any work that mutates the multi-binary emit shape so the lift catches the new shape uniformly.
<!-- SECTION:DESCRIPTION:END -->
