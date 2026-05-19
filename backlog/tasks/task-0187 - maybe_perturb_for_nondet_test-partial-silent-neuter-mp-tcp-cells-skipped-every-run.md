---
id: TASK-0187
title: >-
  maybe_perturb_for_nondet_test partial-silent-neuter: mp-tcp cells skipped
  every run
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 05:13'
updated_date: '2026-05-19 05:46'
labels:
  - M2
  - backend
  - tech-debt
  - gate-trust
dependencies:
  - TASK-0157
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0157 review gate (mped-architect MAJOR finding, independently reproduced). The relocated harness-side determinism-negative perturbation maybe_perturb_for_nondet_test (nucleus/e2e/src/main.rs ~1514) hard-targets tree/src/main.rs. mp-tcp-bufsync emits src/bin/<worker>.rs / src/bin/nuc-generated.rs and NO src/main.rs (nucleus/backends/mp-tcp-bufsync/src/lib.rs ~125/150/172), so ALL ~13 mp-tcp cells return Err -> DetCellStatus::Skipped on EVERY run, today (empirically: NUC_NONDET_TEST=1 yields pass:0 fail:13 skipped:17). determinism-check-negative still bites ONLY because the pthreads-sync cells emit src/main.rs and Fail. The falsifier integrity therefore rests on an implicit, unasserted invariant (>=1 pthreads-sync cell emits src/main.rs and Fails). Partial-silent-neuter risk: if pthreads-sync ever moves to src/bin/ (as mp-tcp already did) while some unrelated cell Fails for another reason, the recipe prints OK while NUC_NONDET_TEST perturbed nothing. Inconsistent with the TASK-0145/0163/0167/0178 gate-trust lineage (a falsifier must PROVABLY bite). Total-drift IS loud (recipe exit 1); this is the PARTIAL case the TASK-0157 notes under-stated.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 maybe_perturb_for_nondet_test perturbs a file EVERY backend emits (e.g. Cargo.toml) OR is made backend-layout-aware so mp-tcp cells are perturbed too (no longer silently Skipped under NUC_NONDET_TEST=1)
- [x] #2 When NUC_NONDET_TEST=1, zero successful perturbations across the whole matrix is a hard FAIL (a Failed cell / non-zero exit), never a uniform Skipped that the recipe inverts to OK
- [x] #3 A unit/integration test asserts the perturbation actually mutated a tree (>=1 cell perturbed) so the falsifier cannot silently become a no-op; determinism-check-negative still bites 100% (>=5 runs) and bare determinism-check stays byte-identical 30/26/0/4
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: maybe_perturb_for_nondet_test targets tree.join("Cargo.toml") (emitted by BOTH backends; src/main.rs is pthreads-only). Append a TOML `# NUC_NONDET_TEST nonce: pid=.. nanos=..` comment (valid TOML, inert) NOT a Rust // comment. Keep per-process nonce, exact-"1" gate, loud banner, env-unset no-op.
2. AC#2: thread a perturbation counter through check_cell_determinism -> DetCellResult. In --check-determinism exit path, if NUC_NONDET_TEST=1 and total successful perturbations == 0, return non-zero (hard FAIL) so the recipe can only print OK when >=1 tree was genuinely mutated AND the diff bit. Also: perturbation Err under the gate already returns Skipped; keep but the zero-count guard is the real invariant.
3. AC#3: integration test in nucleus/e2e: (a) perturb a synthetic temp tree w/ Cargo.toml under NUC_NONDET_TEST=1 -> content changed >=1 + still parses as TOML; (b) env-unset -> strict no-op; (c) zero-perturb guard test.
4. Gate: determinism-check x2 byte-identical 30/26/0/4; determinism-check-negative >=5 consecutive each OK exit0; prove mp-tcp cells now FAIL (pass:0 fail:>13); xbackend-check-negative still bites; just test/e2e/clippy/ci.
5. Commit per logical unit, no AI credit, leave task md unstaged. Append notes w/ verbatim evidence. Forward-carry seam to TASK-0183.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation (TASK-0187, commit 706065d)

**AC#1 — layout-agnostic.** maybe_perturb_for_nondet_test now appends a `# NUC_NONDET_TEST nonce: pid=.. nanos=..` line to tree/Cargo.toml (emitted by BOTH backends: pthreads-sync lib.rs:272, mp-tcp-bufsync lib.rs:132) instead of src/main.rs (pthreads-ONLY; mp-tcp emits src/bin/<worker>.rs). GOTCHA enforced: a `#` TOML comment is valid+inert TOML; a Rust `//` line would make a generated Cargo.toml unparseable. Signature changed Result<(),String> -> Result<bool,String> (Ok(true)=perturbed, Ok(false)=env-unset strict no-op, Err=gate-set-but-file-missing).

**AC#2 — zero-perturb invariant + the recipe-inversion gotcha.** DetCellResult gained a `perturbed: bool` threaded through ALL ~18 constructors (pre-perturbation=false, post=did_perturb, perturb-Err=false). KEY GOTCHA: the determinism-check-negative recipe (justfile:69) INVERTS the exit code — `if HARNESS; then echo FAIL+exit1; else echo OK`. So harness exit 0 => recipe FAIL, harness exit non-zero => recipe OK. To make zero-perturbation a loud gate FAIL the harness must exit CLEAN (0) under the gate, NOT non-zero (exiting non-zero would let the recipe invert a no-op into a false OK). First implementation had this backwards; caught by live demo, fixed. Seam: in main() --check-determinism path, after print_determinism_summary, if NUC_NONDET_TEST=1 and perturbed_cells==0 -> loud FATAL + return Ok(0) so the recipe prints "FAIL: did NOT detect" exit 1.

**Root cause (mp-tcp silently skipped):** the relocated TASK-0157 perturbation hard-targeted tree/src/main.rs; mp-tcp emits no main.rs so all mp-tcp cells returned Err -> DetCellStatus::Skipped, and Skipped does NOT contribute to the harness exit code (only Failed does, main.rs:2090). The falsifier bit ONLY off pthreads cells that happened to emit main.rs+Fail — an implicit unasserted invariant. Failed-vs-Skipped exit semantics: exit non-zero IFF >=1 Failed cell; Skipped is inert.

## Verification gate evidence (final code, commit 706065d)

**mp-tcp NOW BITES (was the bug):** NUC_NONDET_TEST=1 --check-determinism -> `total: 30 pass: 0 fail: 26 skipped: 4`; every mp-tcp-bufsync cell now `FAIL Cargo.toml: length differs (offset 0, len a=365 b=429)` (previously: all ~13 silently SKIPPED). Sanity line: `26 of 30 cell(s) were perturbed` (the 4 skipped are pre-existing distributed-placement gaps, short-circuit before perturbation; 26>=1 so guard inert, 26 FAIL -> exit non-zero -> recipe OK).

**determinism-check-negative x6 consecutive:** runs 1-6 each printed verbatim `OK: determinism check correctly bit on injected nondeterminism` (exit 0).

**bare determinism-check x2:** run1 `total: 30 pass: 26 fail: 0 skipped: 4`; run2 identical. Env-unset is a strict no-op; Cargo.toml untouched (early return Ok(false) before reading tree).

**AC#2 zero-perturb LIVE demo (temp NUC_TASK0187_DEMO_NOOP hook, added+removed, NOT committed):** gate on + perturbation forced no-op for ALL 30 cells -> diff showed `pass: 26 fail: 0` (old logic would exit 0 and recipe would falsely invert... actually old logic exit-on-Failed; the false-confidence scenario). Guard fired: loud `FATAL: NUC_NONDET_TEST=1 but ZERO of 30 cell(s) were actually perturbed ... Forcing a CLEAN exit`, harness exit=0, and through the real recipe inversion: `RECIPE: FAIL: determinism check did NOT detect injected nondeterminism; exit 1`. Demo hook fully reverted (grep-confirmed absent), temp files removed.

**Other gate:** just e2e standalone `total: 30 pass: 26 fail: 0 skipped: 4 required-fail: 0`; just xbackend-check-negative `OK: cross-backend differential correctly bit` (untouched); just test 40 ok-result lines / 0 FAILED (4 new TASK-0187 tests green: perturb_mutates_cargo_toml_and_stays_valid_toml, perturb_is_strict_noop_when_env_unset, perturb_errs_when_gate_set_but_cargo_toml_missing, zero_perturbation_guard_makes_negative_recipe_fail); just clippy --workspace --all-targets -D warnings exit 0; just ci exit 0.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO. qa-test-runner re-verified determinism-check byte-identical x2, determinism-check-negative 6/6, mp-tcp now genuinely bites (pass:0 fail:26, harness EXIT=1), 4 new tests pass, e2e 30/26/0/4, clippy --all-targets clean, ci exit 0; committed zero-perturb guard (main.rs:2127-2152 return Ok(0)) is the correct inversion direction (NOT the implementer first-cut backwardness — that was caught pre-commit). mped-architect: perturbed-flag threading trustworthy (no misset constructor; perturb-Err arm correctly false), Cargo.toml/# approach sound, unset path strict no-op, comments honest, Done honest, TASK-0183 forward-carry accurate+superseding. Non-blocking hardening rec (exit-code-inversion is a fragile cross-layer coupling) filed as TASK-0188 (dep TASK-0187, covers xbackend too). TASK-0187 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closes the partial-silent-neuter in the determinism-negative falsifier: it now provably bites on EVERY backend.

## Problem
The relocated TASK-0157 harness perturbation hard-targeted tree/src/main.rs. mp-tcp-bufsync emits src/bin/<worker>.rs and NO src/main.rs, so all ~13 mp-tcp cells returned Err -> DetCellStatus::Skipped on every negative-gate run. Skipped does not contribute to the harness exit code (only Failed does), so determinism-check-negative bit ONLY off pthreads cells that happened to emit main.rs+Fail — an implicit, unasserted invariant. If pthreads ever moved to src/bin/ while some unrelated cell Failed, the recipe would print OK while NUC_NONDET_TEST perturbed nothing.

## Changes (nucleus/e2e/src/main.rs, commit 706065d)
- AC#1: maybe_perturb_for_nondet_test perturbs tree/Cargo.toml (emitted by every backend) with a `#` TOML-comment nonce — valid, inert TOML (a Rust `//` line would break generated manifests). Signature -> Result<bool,String>.
- AC#2: DetCellResult.perturbed threaded through all constructors; under NUC_NONDET_TEST=1 a matrix-wide zero-perturbation count forces a CLEAN harness exit so the exit-code-INVERTING recipe fires its loud FAIL branch rather than inverting a no-op into a false OK. Bare determinism-check unaffected (env-unset strict no-op).
- AC#3: 4 e2e unit/integration tests (mutation+valid-TOML, env-unset no-op, missing-file Err, zero-perturb guard models the recipe inversion).

## User impact / risk
The headline determinism falsifier now demonstrably bites on all 30 matrix cells (26 fail + 4 pre-existing distributed skips), not just pthreads. No production codegen change (harness-only). Risk: the perturbation count excludes legitimately-Skipped cells (distributed placement gaps) — acceptable, the guard only requires >=1 genuine perturbation.

## Gate (all green)
determinism-check byte-identical 30/26/0/4 x2; determinism-check-negative OK x6 consecutive; mp-tcp cells now FAIL (pass:0 fail:26); AC#2 zero-perturb live demo -> recipe FAIL exit 1; xbackend-negative still bites; just test/e2e/clippy(--all-targets)/ci all exit 0.
<!-- SECTION:FINAL_SUMMARY:END -->
