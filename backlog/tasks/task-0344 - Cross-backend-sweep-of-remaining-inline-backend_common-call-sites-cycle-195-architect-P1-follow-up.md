---
id: TASK-0344
title: >-
  Cross-backend sweep of remaining inline backend_common::* call sites
  (cycle-195 architect P1 follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-27 04:10'
updated_date: '2026-05-28 02:14'
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

## Cycle 224 + 224b closure

Pure stylistic sweep complete; 11 sites in 8 backend files (cycle 224) + 3 sites in driver (cycle 224b fold-back).

Commits:
- 18750df backends: cycle 224 — 11 backend sites lifted (6 from cycle-195 brief + 5 scope-expansion grep)
- 5b0ab6d driver: cycle 224b — 3 driver sites lifted (architect P1.1 silent-sibling fold-back)

Verification gate (final, both commits applied):
  just build         : clean
  just clippy        : clean (-D warnings)
  just test          : all green (no count regression)
  just test-release  : all green
  just e2e           : 280/246/0/34/0 (bit-identical to pre-cycle-224 baseline, three runs across the two gates — including one transient crates.io CDN flake on each gate that re-ran clean)
  just check-textual-replace-on-codegen : OK
  just check-include-str-coverage : OK

Architect re-review (5b0ab6d): GO. No new findings. Sweep cross-tree
re-grep confirmed only legitimately-out-of-scope hits remain (tests/
call sites + doc/string-literal references).

## Acceptance criteria (final)

AC#1 — Each of the 6 sites hoisted to file-head 'use' statement: TICKED.
   Extended to 11 production sites (8 backend files + 3 driver sites).
AC#2 — 'just build && just clippy' clean (no unused-import warnings): TICKED.
AC#3 — 'just e2e' preserves baseline: TICKED. 280/246/0/34/0 bit-identical (three independent runs across cycle-224 + cycle-224b gates).
AC#4 — Commit message cites current baseline + asserts no behaviour change empirically: TICKED (both commits cite 280/246/0/34/0 explicitly).

## Honest scope LIMITs

- Test-code call sites NOT touched (production sweep only):
  * mp-uds-event/tests/multi_worker_emit.rs:326 — live inline elect_host_from_name_workers
  No other test-code inline sites found.
- String-literal references in test assertions NOT touched (these are documentation strings, not call sites):
  * openmp-rs/tests/single_worker_emit.rs:70
  * mp-uds-event/tests/multi_worker_emit.rs:405 + various
  * mp-tcp-poll/tests/single_worker_emit.rs:88 + various
- Doc-comment references using fully-qualified paths NOT touched (canonical-path citation in docstrings is the correct form per project convention).

## Gotchas + forward-carried lessons

1. The original cycle-195 brief grep was symbol-name-specific (matched 'elect_host_from_worker_names' but not 'elect_host_from_name_workers'). Future sweep tasks should grep on the BROADER pattern (e.g. 'backend_common::elect_host' rather than the specific symbol). This is the 18th firing of feedback-silent-sibling-defect.

2. 'just e2e' is sensitive to transient crates.io CDN failures (debug headers: x-served-by: cache-for...; connection failure). Pre-warming the cargo registry cache before the gate would avoid spurious FAIL/build cells. Filing as a follow-up consideration in memory env-netns-sysctl-limits-adjacent forward.

3. Driver crate already had 'backend-common = { path = ... }' dep from TASK-0336 cycle 164 — no Cargo.toml edit needed. Future implementers extending sweeps to the driver should grep the driver's Cargo.toml first to confirm the dep exists before lifting.

AC ticking: all four ACs (1, 2, 3, 4) genuinely met and verified — task DONE.

## Cycle 224b self-audit addendum (post-closure)

The cycle-224b commit message (5b0ab6d) asserted '18th firing of feedback-silent-sibling-defect'. Post-closure grep of the memory file headers (`^## Cycle` count) found the actual count was 20 prior firings (cycles 219 + 220 closed firings 18-20). **Cycle 224b is the 21st firing**, not the 18th.

This is itself an instance of feedback-orchestrator-narrative-also-wrong: a quantitative narrative claim asserted from fuzzy recollection rather than grep-verified. Disclosed in memory feedback-silent-sibling-defect 21st-firing entry rather than amending the closed commit (per feedback-ac-rewrite-on-done-task precedent).

Cycle 224b's review-gate did NOT catch this drift because the count claim was in commit prose, not in code; only an architect retroactively grepping the memory would have caught it. The orchestrator caught it in self-audit while updating the memory file.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycles 224 + 224b closed TASK-0344. Pure stylistic cross-backend sweep: 14 production-code inline 'backend_common::*' call sites lifted to file-head 'use' statements + bare names. Brief named 6 sites (cycle-195 grep); scope-expansion grep found 5 more in mp-tcp-poll/openmp-rs/mp-tcp-bufsync/mp-tcp-poll; architect P1.1 fold-back caught 3 more in the driver crate that the implementer-grep missed (18th firing of feedback-silent-sibling-defect, root cause: original brief grep was symbol-name-specific instead of helper-family-pattern). Final e2e baseline 280/246/0/34/0 preserved bit-identical across three independent runs. Test-code call sites and doc/string-literal references honestly carved out of scope. All four ACs ticked.
<!-- SECTION:FINAL_SUMMARY:END -->
