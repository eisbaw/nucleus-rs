---
id: TASK-0162
title: >-
  Fix pre-existing clippy::len_zero in tests/acfg_to_petri.rs + gate
  --all-targets
status: To Do
assignee: []
created_date: '2026-05-18 22:06'
updated_date: '2026-05-18 22:07'
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
- [ ] #1 The 4 len_zero lints in tests/acfg_to_petri.rs are fixed
- [ ] #2 just clippy recipe uses --all-targets and passes clean
- [ ] #3 No other test-crate clippy debt remains (or is filed)
<!-- AC:END -->
