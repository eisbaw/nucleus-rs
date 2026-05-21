---
id: TASK-0225
title: >-
  Move check_frame_codegen.rs from compiler/tests/ to
  backends/pthreads-sync/tests/ (close backwards dev-dep arrow)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-21 20:42'
updated_date: '2026-05-21 21:12'
labels:
  - compiler
  - backend
  - tech-debt
dependencies:
  - TASK-0221
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0221 (b) deferral: nucleus/compiler/tests/check_frame_codegen.rs uses pthreads_sync::emit(), which requires the compiler crate to have a dev-dep on pthreads-sync (backwards dep arrow: pthreads-sync already depends on compiler at the regular-dep level; the test-only reverse adds an arrow that test-locks the compiler suite to the tier-1 backend). Moving the file to backends/pthreads-sync/tests/ (where multi_worker.rs lives) eliminates the backwards arrow. Pure refactor; no behavior change.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Move check_frame_codegen.rs to backends/pthreads-sync/tests/; update imports + Cargo.toml dev-dep on pthreads-sync REMOVED from compiler/Cargo.toml.
- [ ] #2 Gate: cargo test, e2e, clippy all green; test count unchanged (same tests, different location).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle (pure refactor)

Moved `nucleus/compiler/tests/check_frame_codegen.rs` to `nucleus/backends/pthreads-sync/tests/check_frame_codegen.rs` via `git mv` (history preserved). Closes the backwards dev-dep arrow: compiler/Cargo.toml no longer dev-deps pthreads-sync; the test lives where it belongs (alongside `multi_worker.rs` in pthreads-sync's own test suite).

### Implementation
- `git mv nucleus/compiler/tests/check_frame_codegen.rs nucleus/backends/pthreads-sync/tests/check_frame_codegen.rs` — file move, no content change.
- `nucleus/compiler/Cargo.toml`: removed the `pthreads-sync = { path = "../backends/pthreads-sync" }` dev-dep block; replaced with a "TASK-0225 — closed" rationale comment for future readers.
- The test file's `use pthreads_sync::NameTables;` still resolves — within pthreads-sync's own crate, the `pthreads_sync` crate is accessible the same way external integration tests access their parent crate.
- All `compiler::` imports stay the same (pthreads-sync already deps on compiler at the regular-dep level).

### Gate (orchestrator re-ran)
- cargo test workspace: 549 pass / 0 fail / 2 ignored (unchanged — same tests just relocated).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 / 0 / 7 (baseline unchanged).
- Verified the moved tests actually run (pthreads_sync_emit_includes_panic_instrumentation_on_check_loop, ac3_log_tight_threshold_runs_to_completion_and_logs, ac3_count_tight_threshold_runs_and_prints_summary_at_exit all OK in pthreads-sync's test target).

### Architectural impact
- Compiler library + test suite are now genuinely backend-independent. No dev-dep arrow points from compiler/ to any backend/.
- pthreads-sync's test directory now houses: emit.rs (TASK-0036), multi_worker.rs (TASK-0117), check_frame_codegen.rs (TASK-0052.02/04/05 moved here). Coherent home.

### Forward-carry
None. The change is local and the new placement is more discoverable for future implementers of TASK-0042.01 (pthreads-async) — the pattern of "backend-specific codegen tests live in that backend's tests/ directory" is now the precedent.
<!-- SECTION:NOTES:END -->
