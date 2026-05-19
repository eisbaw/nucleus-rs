---
id: TASK-0188
title: >-
  Harden negative-gate safety invariant: explicit machine-checkable
  perturbation/corruption count, not exit-code-inversion only
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 05:45'
updated_date: '2026-05-19 06:11'
labels:
  - M2
  - backend
  - tech-debt
  - gate-trust
dependencies:
  - TASK-0187
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0187 review gate (mped-architect Finding 1 + Recommendation, non-blocking). The determinism-check-negative AC#2 safety invariant ("the falsifier actually perturbed >=1 tree") is currently encoded SOLELY as the harness exit code, whose meaning is supplied entirely by the inverting shell `if HARNESS; then FAIL; else OK; fi` at justfile:69 — a correct-today but fragile cross-layer coupling: a future refactor of the recipe that drops the inversion would silently re-neuter the falsifier. The committed guard (nucleus/e2e/src/main.rs ~2127-2152, return Ok(0) on zero-perturb-under-gate) is correct, loudly bannered, and regression-tested (zero_perturbation_guard_makes_negative_recipe_fail models the inversion), so this is hardening, not a live defect. Fix: have the harness emit an explicit machine-checkable line (e.g. NUC_NONDET_PERTURBED_CELLS=<n>) on stdout and change justfile:69 to ALSO assert that line, so the safety invariant no longer rests solely on exit-code inversion. The parallel xbackend-check-negative recipe (justfile:85) has the same coupling and must be covered too; TASK-0183 (relocate xbackend wire injection harness-side) will inherit this pattern, so coordinate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 determinism-check-negative harness emits an explicit machine-checkable perturbed-cell-count signal and justfile:69 asserts it IN ADDITION to the exit-code check (a recipe refactor dropping the inversion fails loud, not silently)
- [x] #2 xbackend-check-negative (justfile:85) gets the equivalent explicit machine-checkable corrupted-cell-count assertion
- [x] #3 A test proves the recipe fails loud if the machine-checkable signal says zero perturbations/corruptions even if the exit code alone would invert to OK; determinism-check-negative + xbackend-check-negative still bite 100% (>=5 runs) and bare determinism-check stays byte-identical 30/26/0/4
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: in main() --check-determinism NUC_NONDET_TEST=1 block, after computing perturbed_cells, also println! a stdout line `NUC_NONDET_PERTURBED_CELLS=<n>` (n=perturbed_cells, emitted in BOTH the zero-guard arm with 0 and the normal arm). Document key in code comment. Line only when gate set.
2. AC#2: in e2e run() path, when NUC_XBACKEND_NEGATIVE=1, compute corrupted_detected = required mp-tcp-bufsync cells Failed at Phase::Diff; println! `NUC_XBACKEND_CORRUPTED_DETECTED=<n>`. Only when gate set. Keep exit-code semantics (Ok(required_failed)).
3. justfile:69 + :85: capture combined harness output to a temp file while preserving the exit-code `if` (cargo run >"$out" 2>&1 drives if). After the if verdict, additionally assert the explicit line present AND n>=1; if absent/zero print loud FAIL + exit 1 regardless of exit code. cat "$out" so user still sees output.
4. AC#3: extend tests near zero_perturbation_guard_makes_negative_recipe_fail modeling dual assertion (exit-code AND count-line) for BOTH recipes; signal=0/absent => recipe FAIL even if exit code alone inverts to OK.
5. Gate: determinism-check x2 byte-identical; determinism-check-negative >=5; xbackend-check-negative >=5; e2e 30/26/0/4/0; test; clippy --all-targets; ci. Explicit-signal-fails-loud demo for both.
6. Commit per logical unit, no AI credit, task md unstaged. Forward-carry to TASK-0183.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation (commit 6c703c1)

**Chosen signal keys + stream + capture (the gotcha record):**
- AC#1: `NUC_NONDET_PERTURBED_CELLS=<n>` on STDOUT (println!), emitted UNCONDITIONALLY under NUC_NONDET_TEST=1 (n=0 in zero-perturb arm, n=perturbed_cells otherwise). n = det_results.iter().filter(|r| r.perturbed).count() (the TASK-0187 flag).
- AC#2: `NUC_XBACKEND_CORRUPTED_DETECTED=<n>` on STDOUT, ONLY under NUC_XBACKEND_NEGATIVE=1. PRECISE quantity = count of results where required AND backend=="mp-tcp-bufsync" AND Status::Failed{phase:Phase::Diff}. Each conjunct deliberate: mp-tcp-bufsync (corruption is mp-tcp-exclusive; pthreads emits no wire), Phase::Diff (output.bin != reference.bin oracle; a Compile/Build/Run fail is unrelated breakage), required (matches exit-code semantics). Cannot be satisfied by an unrelated required-fail. Observed value in practice: exactly 1 (02-split-add/split).
- Stream choice: stdout via println! so it is a semantically-distinct parseable RESULT line; loud human diagnostics stay on stderr. Recipe captures COMBINED output (`>"$out" 2>&1`) to a mktemp file so it can grep the line WITHOUT breaking the exit-code `if` — cargo run's own exit status (not tee/grep) still drives `if ...; then bit=0; else bit=1; fi`. Output is `cat`-ed so the user still sees everything. trap rm cleans the tempfile.
- Recipe dual verdict: parse n; if absent (`-z`) OR `< 1` => loud FAIL + exit 1 REGARDLESS of exit code; else fall back to the (unchanged) exit-code inversion. So a refactor dropping the inversion still fails loud via the count assertion. Why robust where pure exit-code inversion was not: the invariant is now asserted in the SAME file as the inversion AND carried as explicit data the harness emits — two independent encodings, not one cross-file coupling.

## Verification gate evidence (commit 6c703c1, all inside nix develop)

**determinism-check x2 byte-identical:** run1 `total: 30 pass: 26 fail: 0 skipped: 4`; run2 identical. NUC_NONDET_PERTURBED_CELLS line ABSENT in both (grep -c = 0/0) — bare path unaffected.

**determinism-check-negative x6 consecutive:** each printed `NUC_NONDET_PERTURBED_CELLS=26` then verbatim `OK: determinism check correctly bit on injected nondeterminism`, exit 0. (runs 1-6 all identical.)

**xbackend-check-negative x5 consecutive:** each printed `NUC_XBACKEND_CORRUPTED_DETECTED=1` then verbatim `OK: cross-backend differential correctly bit on injected mp-tcp corruption`, exit 0. (runs 1-5 all identical.)

**Explicit-signal-fails-loud demonstration (controlled run, exact recipe shell logic):** Demo A: count=0 line present + harness exit NON-ZERO (bit=1, the scenario where exit-code-ONLY inversion prints a false `OK: correctly bit`) -> new recipe printed `FAIL: NUC_NONDET_PERTURBED_CELLS=0 — perturbed NOTHING (TASK-0188)`. Demo B: signal line ABSENT + bit=1 -> `FAIL: NUC_NONDET_PERTURBED_CELLS signal MISSING (TASK-0188; contract broken)`. Both FAIL independent of exit code. xbackend recipe shares identical shell structure (same key) so same property holds.

**just e2e standalone:** `total: 30 pass: 26 fail: 0 skipped: 4 required-fail: 0`; zero NUC_XBACKEND_CORRUPTED_DETECTED / NUC_NONDET_PERTURBED lines (grep -c = 0) — gates absent when unset, byte-for-byte unaffected.

**just test:** whole workspace 0 failed; e2e crate 30 passed (was 29; +1 new test explicit_count_signal_makes_negative_recipes_fail_loud_independent_of_exit_code, green). **just clippy** `--workspace --all-targets -- -D warnings` exit 0 (fixed one doc_lazy_continuation in the new test doc-comment). **just ci** exit 0 (negative-arm tail `pass:25 fail:1 ... required-fail:1` then `NUC_XBACKEND_CORRUPTED_DETECTED=1` then `OK:`).

## Gotchas / limitations
- The recipe now has more shell; if `just` is replaced or the recipe is rewritten in another language, the grep/`-z`/`-lt 1` triad must be ported (documented in justfile comments + the e2e test model).
- NUC_XBACKEND_CORRUPTED_DETECTED counts ONLY Phase::Diff fails; a corruption that somehow caused a Build/Run fail instead would read 0 and (correctly, conservatively) FAIL the recipe loud rather than falsely pass — acceptable: the differential's contract IS a Diff divergence.
- TASK-0183 (relocate maybe_corrupt_wire harness-side) must inherit this explicit-signal contract; forward-carry appended to TASK-0183 notes.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no blocking findings, both explicitly "no genuinely-needed follow-up". qa-test-runner re-verified: determinism-check byte-identical x2 + signal absent unset; determinism-check-negative 5/5 (NUC_NONDET_PERTURBED_CELLS=26 + verbatim OK); xbackend-check-negative 5/5 (NUC_XBACKEND_CORRUPTED_DETECTED=1 + verbatim OK); absent/zero-signal path empirically fails loud independent of exit code (static + live sh reproduction + unit test); e2e 30/26/0/4/0 + xbackend signal absent unset; cargo test workspace 0 failed (e2e 30 passed, +1); clippy --all-targets clean; ci exit 0. mped-architect: dual-assert genuinely independent (two encodings in same file as assertion; redirect preserves cargo exit status; grep ^-anchored numeric; no set -e abort — reproduced in real just+sh); signal emitted on every gated path incl n=0 before the zero-perturb return; xbackend 3-conjunct definition precise & correctly conservative (Phase::Diff is exactly differential divergence; corruption is mp-tcp-exclusive); early-harness-abort collapses to the SAFE direction (absent signal -> loud FAIL, never false OK); comments honest. Minor: in-code comments cite justfile:69/:85 but bodies are now :78/:101 (line drift) — architect judged cosmetic/not-a-lie/no-follow-up; comments also reference recipes by NAME (drift-proof), left as-is (disproportionate to spend a full gate cycle on a comment line-number). TASK-0188 Done stands; TASK-0183 forward-carry accurate & complete.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Hardens both negative gates so the "the falsifier actually perturbed/corrupted >=1 cell" safety invariant no longer rests SOLELY on the cross-file exit-code inversion (justfile:69/:85).

## Problem
The determinism-check-negative / xbackend-check-negative recipes invert the harness exit code (`if HARNESS; then FAIL; else OK; fi`). The "did it actually bite?" invariant was encoded ONLY in that exit code, whose meaning is supplied entirely by the inversion in a DIFFERENT file — a recipe refactor dropping the inversion would silently re-neuter the falsifier (TASK-0187 mped-architect non-blocking finding).

## Changes (commit 6c703c1; nucleus/e2e/src/main.rs, justfile)
- AC#1: `--check-determinism` under NUC_NONDET_TEST=1 prints stable stdout line `NUC_NONDET_PERTURBED_CELLS=<n>` (unconditional under the gate; n=perturbed-cell count). justfile:69 captures combined output to a tempfile (cargo exit still drives the `if`), then asserts the line present AND n>=1 in addition to the inversion; missing/zero => loud FAIL exit 1 regardless of exit code.
- AC#2: e2e run() under NUC_XBACKEND_NEGATIVE=1 prints `NUC_XBACKEND_CORRUPTED_DETECTED=<n>`, n = required mp-tcp-bufsync cells Failed at Phase::Diff (corruption-present-AND-detected, NOT any unrelated required-fail). justfile:85 asserts it the same way.
- Both lines gated: absent under bare determinism-check / e2e (verified byte-identical / 30-26-0-4-0).
- AC#3: new e2e test explicit_count_signal_makes_negative_recipes_fail_loud_independent_of_exit_code models the dual verdict for BOTH recipes and proves count=0 / absent => recipe FAILs even when the exit code alone would invert to OK.

## User impact / risk
Gate-trust hardening only; no production codegen change. Both falsifiers now provably bite via TWO independent encodings (exit code AND explicit count) in the same file as the assertion. Risk: more shell in the recipes (documented + modelled by the test); a non-`just` rewrite must port the count triad.

## Gate (all green, measured)
determinism-check byte-identical 30/26/0/4 x2 (signal absent unset); determinism-check-negative x6 each `NUC_NONDET_PERTURBED_CELLS=26` + OK exit0; xbackend-check-negative x5 each `NUC_XBACKEND_CORRUPTED_DETECTED=1` + OK exit0; explicit-signal-fails-loud demo (count=0 & absent, both with non-zero exit) -> recipe FAIL exit1; e2e standalone 30/26/0/4/0 (no signal lines); just test workspace 0 failed (e2e 30 passed, +1 new); clippy --all-targets exit0; ci exit0.
<!-- SECTION:FINAL_SUMMARY:END -->
