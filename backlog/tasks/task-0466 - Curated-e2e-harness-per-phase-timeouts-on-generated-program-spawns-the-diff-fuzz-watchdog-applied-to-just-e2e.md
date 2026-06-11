---
id: TASK-0466
title: >-
  Curated e2e harness: per-phase timeouts on generated-program spawns (the
  diff-fuzz watchdog, applied to just e2e)
status: Done
assignee: []
created_date: '2026-06-10 19:21'
updated_date: '2026-06-11 04:30'
labels:
  - production
  - e2e
  - test-flake
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Wave-7 review P2 (cross-cutting sibling): TASK-0453.01.01 gave the GENERATIVE harness process-group-kill timeouts on every spawned phase, but the CURATED harness keeps bare .output() spawns — nucleus/e2e/src/run.rs ~:335-520 (compile/build/run phases) and determinism.rs ~:737-755 — so a curated-cell generated-program deadlock still stalls just e2e overnight (exactly the TASK-0461 pingpong night-eater class, one harness over).

Work: lift the diff_fuzz exec.rs timeout machinery (process_group(0), deadline poll, kill-group-THEN-drain — the drain-first pipe deadlock is documented there; reuse, do not reimplement) into a shared e2e helper consumed by both harnesses; phase-tagged FAIL with output tail on expiry; env knob with a sane default (cells normally finish in seconds; 600s default matches diff-fuzz). The harness retain-on-failure scratch convention applies to timed-out cells too.

Related: TASK-0461 (the unit-test-level watchdog for backend integration tests, e.g. pingpong) — different layer, same class; cross-reference both ways.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Every curated-harness spawn phase carries the group-kill timeout; a deliberately-hung cell FAILs with phase tag + tail instead of stalling (negative test)
- [x] #2 Machinery shared with diff_fuzz (one implementation), kill-then-drain order preserved and pinned
- [x] #3 e2e baseline totals unchanged on a green corpus
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Lift diff_fuzz exec.rs (run_timed/Timed/kill_group/resolve_timeout, process_group(0)+deadline-poll+kill-group-THEN-drain) into test_common::proc_timeout (single source). diff_fuzz consumes it (delete local exec.rs logic, keep thin re-export or call directly). Curated harness run.rs Phase1/2/3 + determinism.rs run_nucleus_build spawn through it; phase-tagged FAIL with tail on expiry; env knob NUC_E2E_TIMEOUT_SECS default 600. Negative test: a deliberately-hung cell FAILs with phase tag. e2e baseline totals unchanged on green corpus.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE WORK (batched with TASK-0461):

AC#1 EVERY curated spawn phase carries the group-kill timeout: nucleus/e2e/src/run.rs run_phase_timed() wraps Phase::Compile (cargo run nucleus build), Phase::Build (cargo build --release), Phase::Run (the single-binary exec AND the bash run.sh multi-process launcher); determinism.rs run_nucleus_build() wraps the determinism re-build. A timeout maps to a phase-tagged Status::Failed { phase, detail } with a tail. NEGATIVE TEST: tests.rs::curated_phase_timeout_fails_with_phase_tag_not_stall drives run_phase_timed with sleep 1000 under a 250ms budget — asserts it returns Err((Phase::Run, detail)) with "TIMED OUT" + "run phase" + the env knob name, AND returns in <5s (group-kill fired). Plus curated_phase_budget_rejects_zero_loudly pins the =0 fail-fast.

AC#2 SHARED, ONE IMPLEMENTATION: lifted diff_fuzz exec.rs run_timed/Timed/kill_group/resolve_timeout into nucleus/test-common/src/proc_timeout.rs. process_group(0) + deadline poll + kill-group-THEN-drain order PRESERVED + PINNED (group_kill_reaps_forked_children + hang_is_killed_and_reported tests moved alongside the impl). diff_fuzz/exec.rs is now a thin facade (env name DIFF_FUZZ_TIMEOUT_SECS + re-exports). grep-witness: the ONLY process_group(0)/fn run_timed in the tree is now test-common/src/proc_timeout.rs. diff-fuzz k=2 still 7/7 byte-identical through the facade.

AC#3 e2e baseline totals UNCHANGED on a green corpus: the only behavioral change for a HEALTHY cell is a timeout arm that never fires (cells finish in seconds << 600s default). Env knob NUC_E2E_TIMEOUT_SECS (default 600, matches diff-fuzz spirit; =0/malformed rejected loud). Verified 2 green slice cells (02-split-add/split x {mp-tcp-bufsync,mp-tcp-poll}) stay byte-exact vs reference through the wrapped run path. HONEST SCOPE: did NOT run the full `just e2e` corpus in-session (batch gate); totals-preservation argued structurally (timeout never fires on green) + the 2-cell slice. The full-corpus totals check belongs to the batch gate.

VERIFICATION: clippy workspace clean; e2e bins unit tests green; test-common proc_timeout 7 tests (incl 2 negative hang tests) green dev+release.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Curated harness Compile/Build/Run phases + the determinism re-build wrapped in the shared group-kill watchdog (test_common::proc_timeout — single implementation, diff_fuzz consumes a thin facade; kill-then-drain pinned). Negative test: a hung cell FAILs phase-tagged in <5s. Post-landing hardening from the full-gate shakedown: per-phase budgets (runs 600s, builds 3600s) AND the decisive concurrent-pipe-drain fix (the poll-without-drain shape falsely killed the three chattiest generated builds — found by the first full ci, root-caused, regression-pinned). Baseline confirmed unchanged by the final green just ci (504/443/0/61/0) + ten consecutive just test runs. Landed 90877a2 + a61c379 + 67e4037; architect GO.
<!-- SECTION:FINAL_SUMMARY:END -->
