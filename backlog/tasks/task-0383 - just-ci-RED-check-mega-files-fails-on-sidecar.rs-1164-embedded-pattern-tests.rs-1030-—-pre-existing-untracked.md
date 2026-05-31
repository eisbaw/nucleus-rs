---
id: TASK-0383
title: >-
  just ci RED: check-mega-files fails on sidecar.rs (1164) +
  embedded-pattern/tests.rs (1030) — pre-existing, untracked
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-31 02:39'
updated_date: '2026-05-31 03:33'
labels:
  - ci
  - hygiene
  - mega-file
  - tech-debt
dependencies:
  - TASK-0340
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRE-EXISTING just ci gate failure, surfaced by the TASK-0370 cycle-220 review gate (qa-test-runner). just check-mega-files (an arm of just ci) FAILS direction-A (exit 1): two files exceed the 1000-LoC fence and are NOT in the allow-list, NOT covered by any existing TASK-0340 split slice: (1) nucleus/nucleus-compiler/src/sidecar.rs = 1164 LoC; (2) nucleus/backends/embedded-pattern/src/tests.rs = 1030 LoC. Both grew past 1000 during recent cycles (gather / embedded work, last touched 2026-05-30) and were never split nor allow-listed, so just ci has been RED on this arm since they crossed the threshold. NOTE: the cheap pre-commit subset (build/clippy/test/test-release/e2e) is GREEN and unaffected; this is a full-just-ci-only failure. Resolution per the recipe fix-options: split each along its module-doc seams (TASK-0340 AC#2 preferred), OR add to the check-mega-files allow-list with a one-line rationale if genuinely a coherent unit. Sibling of the TASK-0340 mega-file split epic.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
SPLIT (not allow-list) per TASK-0340 AC#2:
Part A: extract sidecar.rs inline `#[cfg(test)] mod cumulative_tests` (~265 LoC, lines 899-1164) into `sidecar/cumulative_tests.rs` child module via `mod cumulative_tests;` decl (2018-edition file-module dir resolution; precedent sched/, acfg/). `use super::*` reaches private parent items.
Part B: extract embedded-pattern/tests.rs BIN-shape tests (`*_bin_*`, check-frame bin tests + the bin helper) into `tests/bin_shape.rs` child module via `mod bin_shape;`. Helpers stay in tests.rs, child calls via `super::`.
Pure code-move, ZERO behavior change. Same test count before/after.
DoD: both files <1000 LoC via split, just check-mega-files GREEN, all tests pass at same count. Then run check-doc-citation-staleness + check-doc-links + clippy.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-221 orchestrator review gate (GO x2, independently re-verified):
- mped-architect (read-only): GO, no P1/P2. Verified zero test loss (sidecar cumulative_tests 5->5; embedded tests 11->11 across tests.rs+bin_shape.rs), bodies byte-identical modulo mod-wrapper/de-indent, NO new pub (no visibility widening), allow-list byte-untouched (resolution was SPLIT not allow-list), the one stale bare-filename prose ref in embedded-pattern/src/lib.rs:389 (in tests.rs -> tests/bin_shape.rs) was the only one. P3 note: file momentarily fmt-dirty between commit 69763ef and 3b007c4 (harmless, by-design two-commit discipline).
- Orchestrator self-ran FULL just ci (qa-test-runner returned early without numbers): CI_EXIT=0. check-mega-files OK (was the RED arm). just test 1165/0/3 dev, test-release 1164/0/3 (1-test delta = known TASK-0291 profile skew). e2e 329/272/0/57/0 (baseline preserved exactly). determinism/xbackend/required-coverage negative arms all correctly bit. Final file sizes: sidecar.rs 905, sidecar/cumulative_tests.rs 271, embedded tests.rs 534, tests/bin_shape.rs 515 (all <1000).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle-220 RESOLVED via SPLIT (not allow-list), per TASK-0340 AC#2 + the recipe's preferred fix-option. No formal ACs; DoD met and verified.

Part A — nucleus-compiler/src/sidecar.rs 1164 -> 905 LoC: extracted the inline `#[cfg(test)] mod cumulative_tests` into the child file sidecar/cumulative_tests.rs (263 LoC) via `mod cumulative_tests;` (2018-edition file-module dir resolution; precedent sched/, acfg/). Child stays a child of `sidecar` so `use super::collect_cumulative_data_names` reaches the private parent fn. Test paths unchanged: sidecar::cumulative_tests::* (5 tests).

Part B — embedded-pattern/src/tests.rs 1030 -> 534 LoC: carved the M10 BIN-shape tests + bin-only helpers (emit_bin_example_naive, try_emit_bin_ex1_with_check, the 3 ex*_bin_* tests, bin_rejects_multi_worker, 3 check_loop_* tests = 7 #[test]) into child tests/bin_shape.rs (515 LoC) via `mod bin_shape;` (already inside #[cfg(test)] mod tests, no extra cfg). LIB-shape tests (4) + shared repo_root helper stay in tests.rs; child calls super::repo_root. Test paths now tests::bin_shape::*.

GOTCHAS hit:
1. `use std::path::PathBuf;` copied into bin_shape.rs was UNUSED (bin tests only call repo_root(), never name PathBuf) -> dead-import warning that would FAIL `just clippy -D warnings`. Removed it. Lesson: when carving a child test mod, re-derive its imports from what the MOVED code actually names, don't copy the parent's import block.
2. INTRODUCED doc-lie caught by silent-sibling grep: embedded-pattern/src/lib.rs had a comment 'pinned by bin_rejects_multi_worker_* in tests.rs' — the test moved to tests/bin_shape.rs. Re-anchored the comment. (No fully-qualified file.rs:N citations pointed into either moved range, so check-doc-citation-staleness stayed GREEN; the lie was a bare-filename location claim the fence does not catch.)
3. The fmt pass (TASK-0378, separate commit) reformatted the new sidecar/cumulative_tests.rs (assert_eq! one-liners wrapped) — expected, harmless.

GATE (cheap subset, nix dev shell): build OK; clippy OK (-D warnings); test 1165/0/3 == baseline (ZERO test loss, pure code-move); test-release 1164/0/3 (the 1165->1164 delta is the pre-existing TASK-0291 debug_assert/should_panic profile skew — confirmed NO should_panic in any moved file, so unrelated to this split); check-mega-files OK (was the RED arm); check-doc-citation-staleness OK; check-doc-links OK.

Final sizes (all <1000): sidecar.rs 905, cumulative_tests.rs 263, tests.rs 534, bin_shape.rs 515. Commit 69763ef (split). Did NOT run full `just ci` (heavy e2e/determinism arms) — deferred to the read-only review gate.
<!-- SECTION:FINAL_SUMMARY:END -->
