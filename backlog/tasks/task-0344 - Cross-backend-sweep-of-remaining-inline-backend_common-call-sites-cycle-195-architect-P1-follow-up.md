---
id: TASK-0344
title: >-
  Cross-backend sweep of remaining inline backend_common::* call sites
  (cycle-195 architect P1 follow-up)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-27 04:10'
updated_date: '2026-05-28 01:32'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-224 implementation plan (orchestrator-direct)

Per project memory `feedback-spawned-agents-refuse-code-edits`, the orchestrator implements directly for low-risk stylistic sweeps. Plan:

1. Re-confirm 6 sites (line drift acknowledged):
   - mp-tcp-bufsync/src/encode.rs:40, :54  — rust_scalar_type_pub x2
   - pthreads-async/src/multi_worker.rs:199 — elect_host_from_worker_names
   - pthreads-async/src/multi_worker.rs:634, :635 — CountCheckLoop x2
   - pthreads-sync/src/multi_worker.rs:194 — elect_host_from_worker_names
   - pthreads-sync/src/lib.rs:469 — render_array_init_for

2. Edit policy: hoist each inline backend_common::path::Sym to file-head use; replace inline full path with bare Sym. Verify each edit's use clause already exists (most files import the parent module already) and add to the existing use block if so.

3. Gate: nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e". Baseline 280/246/0/34/0 (last recorded in 0357 closure narrative — to re-verify).

4. Skip docstring/comment references (lines like '// see backend_common::xxx') — those are doc text, not call sites.
<!-- SECTION:NOTES:END -->
