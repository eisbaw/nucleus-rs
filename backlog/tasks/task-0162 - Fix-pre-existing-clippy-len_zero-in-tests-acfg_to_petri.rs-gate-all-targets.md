---
id: TASK-0162
title: >-
  Fix pre-existing clippy::len_zero in tests/acfg_to_petri.rs + gate
  --all-targets
status: Done
assignee: []
created_date: '2026-05-18 22:06'
updated_date: '2026-05-22 21:07'
labels:
  - tooling
  - tech-debt
  - quality
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
qa-test-runner finding during TASK-0142 review (pre-existing, NOT introduced by 8fd0ffc; traced to commit 5df3f62 / TASK-0150). Running clippy with --all-targets reports 4 clippy::len_zero errors in nucleus/compiler/tests/acfg_to_petri.rs (~lines 481/501/524/541): the net.transitions.len() greater-than-0 idiom should be the is_empty() negation. The project clippy gate (the just clippy recipe = cargo clippy --workspace -- -D warnings) has NO --all-targets, so test-crate lints are ungated and silently drifting. Fix the 4 lints and switch the just clippy recipe (and the CI job, TASK-0057) to --all-targets so test code is gated too.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The 4 len_zero lints in tests/acfg_to_petri.rs are fixed
- [x] #2 just clippy recipe uses --all-targets and passes clean
- [x] #3 No other test-crate clippy debt remains (or is filed)
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 60d tracker hygiene (2026-05-22). All 3 ACs structurally met by pre-session work + this session's continuous gate enforcement.

AC#1: 4 len_zero lints in tests/acfg_to_petri.rs are fixed. Verified: grep '.len() > 0' returns no matches.
AC#2: just clippy recipe uses --all-targets and passes clean. Verified: justfile recipe is 'cargo clippy --workspace --all-targets -- -D warnings'. Continuously green across 39 cycles in 2026-05-22.
AC#3: no other test-crate clippy debt remains. Verified by --all-targets being part of the gate.

No source changes; no gate impact.
<!-- SECTION:FINAL_SUMMARY:END -->
