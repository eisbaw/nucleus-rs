---
id: TASK-0344
title: >-
  Cross-backend sweep of remaining inline backend_common::* call sites
  (cycle-195 architect P1 follow-up)
status: To Do
assignee: []
created_date: '2026-05-27 04:10'
labels:
  - tech-debt
  - hygiene
  - style
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Goal

Sweep the 6 remaining inline `backend_common::*` call sites cycle 195 (TASK-0340.01.02 + .04.02) did not touch — same stylistic-consistency rule, different files. Cycle 195 was file-scoped to mp-tcp-bufsync/plan/ and mp-tcp-event/multi_worker/ per the original briefs; this task extends the sweep cross-backend.

## Sites to fix (architect-grep cycle 195)

- `nucleus/backends/mp-tcp-bufsync/src/encode.rs:40, :56` — `backend_common::render::rust_scalar_type_pub` x2.
- `nucleus/backends/pthreads-async/src/multi_worker.rs:198` — `backend_common::elect_host_from_worker_names`.
- `nucleus/backends/pthreads-async/src/multi_worker.rs:603, :604` — `backend_common::check_frame::CountCheckLoop` x2.
- `nucleus/backends/pthreads-sync/src/multi_worker.rs:193` — `backend_common::elect_host_from_worker_names`.
- `nucleus/backends/pthreads-sync/src/lib.rs:465` — `backend_common::render::render_array_init_for`.

(Line numbers as of cycle-195 stamp; expect drift if subsequent cycles edit these files. Re-grep before editing.)

## Acceptance criteria

1. Each of the 6 sites hoisted to a file-head `use` statement; inline full-path call replaced with bare name.
2. `just build && just clippy` clean (no unused-import warnings).
3. `just e2e` preserves current baseline (210/161/0/49/0 as of cycle 194; re-check current at filing time).
4. Commit message cites current baseline + asserts no behaviour change empirically.

## Honest scope

Pure stylistic refactor. Zero behaviour change. Reading time ~5 minutes, edit time ~10 minutes. Should batch in a single small cycle once an opportunity arises.

## Forward-carried context (cycle 195)

- Per `feedback-silent-sibling-defect`: cycle 195 explicitly file-scoped to the brief-named files (mp-tcp-bufsync/plan/ + mp-tcp-event/multi_worker/) and disclosed the project-scope gap in its commit. This task closes that gap.
- Per `feedback-implementer-disclosure-mechanism-wrong` cycle 187b: if MORE inline call sites appear between filing and implementation (via intermediate cycles adding new backend_common helpers), sweep all of them — don't cite the brief literally if it's stale.
<!-- SECTION:DESCRIPTION:END -->
