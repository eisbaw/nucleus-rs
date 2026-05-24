---
id: TASK-0291
title: should_panic test rests on debug_assert! — fails under cargo test --release
status: To Do
assignee: []
created_date: '2026-05-24 21:13'
labels:
  - backend-common
  - tests
  - release-mode
  - hardening
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

`backend-common`'s `project_skeleton::multi_binary_tests::run_sh_multi_debug_asserts_so_buf_comment_lines_are_shell_comments` (commit 9fa807e, TASK-0257 cycle-112 hardening) is `#[should_panic]` and relies on a `debug_assert!` in `render_run_sh_multi` to fire. Under `cargo test --release` the debug_assert is stripped, the call does NOT panic, and the `#[should_panic]` test fails ("test did not panic as expected").

The pre-existing `just test` recipe uses the dev profile (no `--release`), so this is silently green on the gate. But `cargo test --release` (which qa-test-runner-like sanity checks may use) reports a hard fail.

## Acceptance criteria

1. Either:
   (a) replace `debug_assert!` with `assert!` inside the function — the line-not-a-shell-comment check is cheap and the runtime cost is irrelevant outside of hot loops; OR
   (b) explicitly annotate the test as `#[cfg(debug_assertions)]` so it is compiled out under `--release` and not visible to the test runner; OR
   (c) leave the assertion as debug_assert! but rewrite the test to check the function's defensive behaviour in some way that does NOT rely on the panic.
2. `cargo test --release -p backend-common` runs clean (no failures).
3. `cargo test -p backend-common` (dev) still asserts the contract bite (whichever shape (a)/(b)/(c) lands).

## Honest scope

LOW priority. Discovered TASK-0289 cycle 114a review-hardening by orchestrator while sanity-checking the release-profile gate. The qa-test-runner agent did NOT run `--release` for the unit-test gate, so it was hidden. No production-path defect, just a CI/sanity hole.

## Recommendation

Option (a) is the cleanest — convert to `assert!`. The check is one regex per line of a small comment string at codegen time; it does not bite a hot loop. Replacing `debug_assert!` with `assert!` removes the dev-vs-release skew permanently. Also tighten `just test` (or add a `just test-release` recipe) so this profile is gate-visible going forward.
<!-- SECTION:DESCRIPTION:END -->
