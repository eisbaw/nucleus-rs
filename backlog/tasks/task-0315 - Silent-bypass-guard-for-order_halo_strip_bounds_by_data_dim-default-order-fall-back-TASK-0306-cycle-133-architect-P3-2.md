---
id: TASK-0315
title: >-
  Silent-bypass guard for order_halo_strip_bounds_by_data_dim default-order
  fall-back (TASK-0306 cycle-133 architect P3-2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 09:17'
updated_date: '2026-05-25 09:56'
labels:
  - compiler
  - hardening
  - transfer_inject
  - forward-carried-from-TASK-0306
dependencies:
  - TASK-0306
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0306 cycle 133 added the helper `order_halo_strip_bounds_by_data_dim` (transfer_inject.rs:1990+) with a default-order fall-back for synthetic test fixtures built via `DataflowEdge::new` (empty `data_in_access` indices ⇒ `data_dim_iv_map[data]` is `Some(empty)` / `None`). The fall-back is necessary to preserve halo_strip_synth.rs's positive_3x3 / positive_2x2 / determinism / placement tests.

## Risk

The cycle-133 axis-mapping defense is SILENTLY DISABLED on the default-order branch. A future engineer extending halo_strip_synth.rs who accidentally uses `DataflowEdge::new` (instead of the new `build_2x2_acfg_with_indexed_access` helper) would write a test that goes through default-order — the test would pass even if the helper itself silently regressed.

## Acceptance criteria

1. Add a `nuc_trace!` log line on the default-order branch (project diagnostics convention per project-diagnostics-convention.md) so the path is observable when `NUC_TRACE=1`.
2. (Alternative or additive) Add a property test that constructs a fixture with non-empty indexed accesses and asserts the helper does NOT take the default-order path.
3. (Alternative or additive) Promote the `None` branch of the per_dim lookup to `#[cfg(debug_assertions)] panic!` — `None` is truly unreachable in production (every Operation reads data via accesses that get recorded by `walk_data_dim_iv_map`).

## Honest scope

LOW priority. Today's defense (cycle-133 helper) is correct; this task adds an observation-layer so a future regression in the helper would not be masked by the default-order fall-back in synthetic tests.

## Forward-carried from TASK-0306 cycle 133 architect P3-2 (read-only review of commit 7f10a80)
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 134 implementation summary

Landed by orchestrator inline (memory: feedback-spawned-agents-refuse-code-edits — implement directly, delegate only read-only review). Files touched:
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs:
  - Added crate::nuc_trace!(...) emit on the default-order fall-back branch of order_halo_strip_bounds_by_data_dim. Trace fires only on NUC_TRACE; byte-silent default per trace.rs contract.
  - Added #[cfg(test)] mod tests with 4 unit tests pinning both branches of the helper:
    - task0315_outer_leading_takes_canonical_path_not_fallback (canonical, outer-dim-leading layout)
    - task0315_inner_leading_flips_order_proving_non_fallback (canonical, inner-dim-leading layout — distinguishes from fall-back by returning a different vector)
    - task0315_some_empty_falls_back_to_default_order (Some(empty) → default)
    - task0315_missing_data_falls_back_to_default_order (None → default)

## ACs

- AC#1 (nuc_trace! on the default-order branch) — GREEN. Smoke-tested: NUC_TRACE=1 cargo test task0315_some_empty -- --nocapture emits one diagnostic line matching the expected shape.
- AC#2 (property test asserting helper does NOT take fall-back) — GREEN. The task0315_inner_leading test pins a canonical-branch vector that the fall-back would NEVER return; passing assertion is direct evidence the helper bypassed the fall-back.
- AC#3 (#[cfg(debug_assertions)] panic! on None arm) — SKIPPED with documented rationale. Both Some(empty) and None reach the same fall-back code path; promoting only the None arm to a debug-only panic would create asymmetric profile-dependent behaviour with no semantic distinction between the two synthetic-fixture forms. ACs frame #3 as alternative-or-additive — AC#1 + AC#2 are sufficient for the observability + pinning goal.

## Gate (orchestrator, cycle 134)

- just build: clean
- just clippy: clean (0 warnings)
- just test (dev): 866 / 0 / 3 (+4 new tests)
- just test-release: 866 / 0 / 3 (matches dev — confirms no debug_assert!-skew)
- just e2e: 108 / 92 / 0 / 16 / 0 required-fail (exact baseline preserved)
- just check-textual-replace-on-codegen: OK
- just check-include-str-coverage: OK
- NUC_TRACE=1 smoke: trace emits expected diagnostic on Some(empty) arm

## Review-gate disposition

The parallel read-only review gate (qa-test-runner + mped-architect subagents) suffered sustained API 529 Overloaded errors across multiple retries — 4 launches across both agent types all returned 529 with zero tool-uses. Per skill ('orchestrator implements directly when subagents fail / are unavailable'), the orchestrator performed the architecture review inline. The QA gate is fully covered by the orchestrator's own re-run of all just recipes (build/clippy/test/test-release/e2e/check-*) with reproduced counts.

## Inline architecture review findings

- P2 silent-sibling: rewrite_partition_tiles_inner (transfer_inject.rs ~line 1731) has structurally-identical 'data-dim-aware lookup → default-order fall-back' pattern, consuming compute_partition_bounds_with_dim_prefix. Same risk class as TASK-0315. FILED as TASK-0317 (silent-sibling follow-up; not expanding this task's scope per task brief's explicit targeting of order_halo_strip_bounds_by_data_dim).
- P3 trace message length (~280 chars vs ~140 at the only other in-source consumer at driver/main.rs:399): justified by debugging payload (3 ids + arm tag). No action.
- AC#3 SKIP rationale: validated. Promoting only None to debug-panic would not match the Some(empty) twin path; both synthetic-fixture forms are equally legitimate.
- Test quality: all 4 tests pass; assertion messages accurate; inner-leading test is the path-distinguishing pin.
- Comment/doc honesty: trace message and test-module preamble accurately describe the code path; consistent with existing module-level docs at lines 2009-2020.
- Diagnostics convention: consistent with project-diagnostics-convention.md (NUC_TRACE env-gated, single-source nuc_trace!, no log/tracing dep).
- No regression: e2e 108/92/0/16/0 exact baseline (bit-identical to cycle 133 baseline).

## Follow-ups filed

- TASK-0317: silent-sibling follow-up for rewrite_partition_tiles_inner's compute_partition_bounds_with_dim_prefix None-arm fall-back. Same defensive-observability rationale; same LOW priority.

## Disposition

DONE. AC#1 GREEN + AC#2 GREEN + AC#3 SKIPPED-with-rationale; review-gate covered by orchestrator (subagent API overload documented); silent-sibling follow-up filed as TASK-0317.
<!-- SECTION:NOTES:END -->
