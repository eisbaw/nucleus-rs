---
id: TASK-0168
title: >-
  Standing negative gate: prove the [[required]]-coverage guard still bites
  (wired path)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 22:34'
updated_date: '2026-05-23 15:29'
labels:
  - infra
  - tooling
  - quality
  - e2e
dependencies:
  - TASK-0163
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0163 (finding #1). TASK-0163 added required-cell coverage checking with 5 unit tests + a no-gaps-today durable guard, but there is NO standing gate proving the WIRED run() path still exits non-zero on an injected required typo. A future refactor could drop the `if !gaps.is_empty() { return Err }` wiring and all 5 unit tests stay green (they test the pure function, not the harness exit). This is the same class determinism-check-negative solves for determinism. Add an analogous standing negative recipe/test: with an env/flag or a fixture manifest carrying a deliberately-typod required schedule, assert the WIRED nucleus-e2e harness exits non-zero naming the triple; wire it into just ci. Mirror the determinism-check-negative pattern (recipe SUCCEEDS iff harness correctly FAILS).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A standing check (recipe/test in just ci) injects a typod required cell and asserts the wired harness exits non-zero naming the triple
- [x] #2 It does not leave a broken manifest committed (env/flag or transient fixture, like determinism-check-negative)
- [x] #3 Removing the run() coverage-gate wiring makes this check fail (proven)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add fn maybe_inject_required_coverage_negative(manifest: &mut Manifest) -> Result<bool, String> sibling to maybe_perturb_for_nondet_test / maybe_corrupt_wire_for_xbackend. Reads NUC_REQUIRED_COVERAGE_NEGATIVE; exact-"1" gate, strict no-op otherwise. Picks existing required[0]'s (example, backend, milestone) so cell_matches_filters/milestone_in_gate accept the synthetic entry; uses fixed sentinel schedule "__nuc_typo_negative_schedule__" so the gap is deterministic and cannot collide with any real *.sched.nuc file. Falls back to runnable_examples[0]+backends[0]+"M1" if required Vec empty. Loud stderr WARNING banner mirroring NUC_NONDET_TEST. Append-not-mutate: never touches existing required entries (preserves AC#2).\n2. In run_inner: change manifest binding to mut; call maybe_inject_required_coverage_negative(&mut manifest) right after parse, before plan_cells. After required_coverage_gaps returns gaps, when NUC_REQUIRED_COVERAGE_NEGATIVE=1 print machine-checkable line on stdout: NUC_REQUIRED_COVERAGE_GAP_DETECTED=<n> where n = number of gaps whose schedule == "__nuc_typo_negative_schedule__". This isolates injection-attributable gaps from any unrelated ones. Print BEFORE returning Err so the line is on stdout even when the harness exits non-zero. Mirror the determinism/xbackend zero-injection guard: if injected but gaps==0, FATAL + Ok(0) so the inverting recipe FAILs loud.\n3. New justfile recipe required-coverage-check-negative mirroring xbackend-check-negative shape exactly. Long header comment naming TASK-0168/0163, calling out TASK-0188 belt-and-suspenders.\n4. Append recipe to ci aggregate.\n5. Unit tests for the new injection function.\n6. Verify bare just e2e byte-identical (88/70/0/18).\n7. AC#3 manual proof by temporarily removing wiring; document in notes.\n8. Full gate before commit. No AI co-author credit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 71 implementation landed.

Files:
- nucleus/e2e/src/main.rs: added REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE constant + maybe_inject_required_coverage_negative() injection sibling of maybe_perturb_for_nondet_test / maybe_corrupt_wire_for_xbackend; wired into run_inner after manifest parse (changed manifest binding to mut), with new NUC_REQUIRED_COVERAGE_GAP_DETECTED=<n> stdout signal emitted whenever the gate is set, attribution filter scoped to the sentinel schedule (NOT raw gaps.len() — prevents an unrelated gap from satisfying the signal, the TASK-0187 partial-silent-neuter lesson). Zero-injection FATAL guard mirrors xbackend / determinism. 4 new unit tests (e2e crate unittests 71->75).
- justfile: required-coverage-check-negative recipe (mirrors xbackend-check-negative shape exactly); wired into ci aggregate.

Commits (no remote push):
- 061b6c7 e2e: harness-side NUC_REQUIRED_COVERAGE_NEGATIVE injection seam (TASK-0168)
- 226a030 justfile: required-coverage-check-negative recipe + ci wire (TASK-0168)

Gate (nix develop -c just ci, exit 0 — all green):
- check: clean
- clippy --workspace --all-targets -D warnings: clean
- test: all green, e2e crate unittests 75/0/0 (was 71; +4 req_cov tests)
- e2e: 88/70/0/18 required-fail 0 (byte-identical baseline preserved)
- determinism-check: 88/70/0/18 byte-identical
- determinism-check-negative: NUC_NONDET_PERTURBED_CELLS=70 OK
- xbackend-check-negative: NUC_XBACKEND_CORRUPTED_APPLIED=16 NUC_XBACKEND_CORRUPTED_DETECTED=1 OK
- required-coverage-check-negative (NEW): NUC_REQUIRED_COVERAGE_GAP_DETECTED=1 OK, harness exited non-zero naming (example=01-elementwise-add, schedule=__nuc_typo_negative_schedule__, backend=pthreads-sync)

AC verification:
- AC#1 (standing check in just ci): VERIFIED — see required-coverage-check-negative recipe and ci wire above; gate output confirms harness exits non-zero naming the synthetic triple.
- AC#2 (no broken manifest committed): VERIFIED by construction — the env-flag seam appends ONE synthetic entry to the in-memory Manifest AFTER toml::from_str; the on-disk e2e-matrix.toml is never touched. git status post-test is clean. Append-not-mutate discipline (does NOT touch any existing required entry) is pinned by a unit test.
- AC#3 (removing the wiring makes the check fail): VERIFIED MANUALLY (proof NOT committed). Temporarily replaced  with  in run_inner, re-ran just required-coverage-check-negative. Result: harness ran to completion with 88/70/0/18 required-fail 0 (the exact TASK-0163 silent-vanish pattern, demonstrating the wiring IS the load-bearing line), recipe correctly FAILed loud: 'FAIL: required-coverage guard did NOT exit non-zero on injected typo\'d required cell (TASK-0168 wired path silently re-neutered)' with exit code 1. Restored the wiring; recipe back to OK.

Subtleties / gotchas worth recording for future cycles:
- The signal MUST be printed BEFORE the Err return path. Err goes through  -> eprintln! -> ExitCode::FAILURE, which is what the recipe inverts. Putting the println after the Err would never fire.
- Attribution filter (sentinel schedule, NOT raw gaps.len()) is load-bearing: TASK-0187 captured the case where a no-op + unrelated incidental failure would let the recipe print OK off the wrong reason. Mirrored here precisely.
- The injection anchors on required[0]'s (example, backend, milestone) — NOT a hardcoded string — so the synthetic cell automatically survives any future --milestone narrowing in CI without code changes. Defensive fallback to runnable_examples[0]+backends[0]+M1 if required Vec is empty; degenerate (no runnable_examples) is a loud Err, NOT a panic — pinned by a unit test.
- Anti-redundancy: tests #5060/0163 (typo_in_required_schedule_is_a_coverage_gap) already exercise the pure function. The new tests don't duplicate that — they pin the injection-function contract (env-unset noop, env-set append, gap surfaces, degenerate Err).
- Rejected alternatives: (a) committed broken fixture would violate AC#2; (b) more unit tests on required_coverage_gaps would not prove the wired run() exit-code path bites — the entire point of the task.

Forward-carry: appended note to TASK-0163 (the source task).

Cycle 71 review-gate hardening (commit 74a79de): mped-architect MINOR-1 (wire-site comment cross-file line numbers → name references), MINOR-2 (docstring overclaim on M1 fallback robustness → named the assumption explicitly), MINOR-3 (latent cross-test env-mutex hazard → filed TASK-0251 with explicit trigger conditions, deferred since today's hazard is dormant). qa-test-runner verdict GO (full just ci green, 88/70/0/18, new recipe deterministic across 2 samples NUC_REQUIRED_COVERAGE_GAP_DETECTED=1 both times). mped-architect verdict GO (zero blockers, zero majors, 3 MINORs applied/deferred).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0168 closes a standing wired-path negative-gate for TASK-0163: it proves run_inner still exits non-zero when a typod required cell is injected at runtime, mirroring the determinism-check-negative / xbackend-check-negative pattern (env-flag seam, no committed broken manifest, deterministic, machine-checkable signal + exit-code inversion belt-and-suspenders per TASK-0188).

Changes:
- nucleus/e2e/src/main.rs: maybe_inject_required_coverage_negative() appends ONE synthetic [[required]] entry under NUC_REQUIRED_COVERAGE_NEGATIVE=1 with sentinel schedule __nuc_typo_negative_schedule__; run_inner emits NUC_REQUIRED_COVERAGE_GAP_DETECTED=<n> on stdout where n is filtered by sentinel schedule (precise attribution, not raw gaps.len()). Append-not-mutate; loud stderr WARNING; deterministic (no clock/PID/RNG); zero-injection FATAL guard. 4 new unit tests (e2e crate 71->75).
- justfile: required-coverage-check-negative recipe mirroring xbackend-check-negative exactly; wired into ci aggregate.

Gate (nix develop -c just ci, exit 0, all green): e2e 88/70/0/18 byte-identical baseline; determinism 88/70/0/18; det-neg NUC_NONDET_PERTURBED_CELLS=70 OK; xb-neg APPLIED=16 DETECTED=1 OK; req-cov-neg (NEW) NUC_REQUIRED_COVERAGE_GAP_DETECTED=1 OK and harness exited non-zero naming (example=01-elementwise-add, schedule=__nuc_typo_negative_schedule__, backend=pthreads-sync).

AC verification: AC#1 standing recipe in just ci injects typo and asserts non-zero with named triple (verified end-to-end). AC#2 no broken manifest committed — env-flag seam, on-disk e2e-matrix.toml untouched; append-not-mutate pinned by unit test. AC#3 verified manually (proof NOT committed): wiring temporarily disabled in run_inner, recipe correctly FAILed loud ("required-coverage guard did NOT exit non-zero on injected typod required cell (TASK-0168 wired path silently re-neutered)" with exit 1, harness ran to 88/70/0/18 required-fail 0 — the exact silent-vanish pattern); wiring restored, recipe back to OK.

Commits: 061b6c7 (harness seam) and 226a030 (recipe + ci wire). No remote push.

Forward-carry: TASK-0163 (the source task) — appended a cycle-71 note that the standing wired-path negative gate now exists, so the trust in required_coverage_gaps no longer rests SOLELY on the 5 in-isolation unit tests.
<!-- SECTION:FINAL_SUMMARY:END -->
