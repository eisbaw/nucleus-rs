---
id: TASK-0363
title: >-
  check_kernels_contract rustc_check breaks on --kernels filenames with dots
  (e.g. kernels.embedded.rs -> invalid crate name)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-29 08:00'
updated_date: '2026-05-29 09:28'
labels:
  - backend
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DISCOVERED in TASK-0049.06. The contract check phase 1 (nucleus-compiler/src/contract.rs rustc_check) invokes `rustc <path>` WITHOUT `--crate-name`, so rustc derives the crate name from the file stem. For a kernels file with a dot in the stem (e.g. `kernels.embedded.rs`, used by the M11 ex14 sync sibling via `--kernels`), rustc rejects with "invalid character `.` in crate name: `kernels.embedded`" and the contract check reports a spurious RustCheckFailed. This is NON-FATAL in the driver (cmd_build surfaces contract errors as a warning and proceeds), so the M11 cross-compile is unaffected — but it pollutes the contract output and means rustc_check ALWAYS fails for any dotted-stem kernels file, defeating the phase-1 compile-check for those files. FIX: pass `--crate-name <sanitised>` (replace non-ident chars) to the rustc invocation in rustc_check. Low-risk, single call site. Sibling check: confirm no other rustc/cargo invocation in the contract path derives a crate name from a dotted path.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 rustc_check passes an explicit --crate-name (sanitised: dots/dashes -> underscores) so a --kernels file with a dotted stem (kernels.embedded.rs) passes phase-1 rustc check
- [x] #2 A test pins a dotted-stem kernels file no longer triggers RustCheckFailed
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Architect P2.2 (TASK-0049.06 review): the driver warning text at nucleus/driver/src/main.rs:339 hardcodes "(aggregate-typed I/O is a known gap, TASK-0012)" — so when this dotted-stem rustc_check failure fires, the build output MISATTRIBUTES it to TASK-0012. When fixing this task (sanitised --crate-name), also fix/disambiguate that warning text so the dotted-stem cause is not conflated with the TASK-0012 aggregate-typed-IO gap. Also: documenting-not-fixing was accepted for the TASK-0049.06 cycle, but the consequence is rustc_check ALWAYS fails for kernels.embedded.rs, so a real Rust syntax error in that file would NOT be caught at phase 1 (only later at the embedded cross-compile) — mild safety-net erosion, another reason to do the one-call-site fix.

IMPL PLAN (cycle): Added sanitise_crate_name(path) in contract.rs: maps file_stem non-[A-Za-z0-9_] chars to '_', prefixes '_' if empty or leading-digit. rustc_check passes --crate-name <sanitised>. Removed the KNOWN GAP comment (replaced with a TASK-0363 one-liner stating crate-name is now sanitised). Driver warning at main.rs:~339 de-conflated: dropped the hardcoded 'aggregate-typed I/O is a known gap, TASK-0012' parenthetical -> generic 'see individual issues below'. Test dotted_stem_kernels_file_does_not_trip_rust_check in tests/contract.rs: writes good/kernels.rs content to a temp kernels.embedded.rs, asserts no RustCheckFailed. SIBLING CHECK: workspace-wide grep — contract.rs rustc_check is the ONLY rustc-by-file-stem call site; all cargo build/run invocations (e2e/main.rs, backend tests, justfile check-embedded thumbv7em cross-compile) derive crate names from Cargo.toml, not file stems — safe.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Commit 88fc406. AC#1 MET: rustc_check passes --crate-name from sanitise_crate_name(path) — file_stem with non-[A-Za-z0-9_] -> '_', leading-digit/empty -> '_'-prefixed (kernels.embedded -> kernels_embedded; 3d -> _3d). AC#2 MET: dotted_stem_kernels_file_does_not_trip_rust_check (writes good/kernels.rs to a temp kernels.embedded.rs, asserts no RustCheckFailed). Architect note DONE: driver warning de-conflated (dropped the TASK-0012 parenthetical -> 'see individual issues below'). SIBLING CHECK: workspace-wide grep — contract.rs rustc_check is the ONLY rustc-by-file-stem call site; all cargo build/run invocations (e2e/main.rs x3, backend pingpong/multi_worker tests, justfile check-embedded thumbv7em cross-compile) derive crate names from Cargo.toml package names, NOT file stems — confirmed safe, no follow-up needed. Gate: build ok, clippy clean, test 1091/0/3, test-release 1090/0/3, e2e 308/246/0/62/0. GOTCHA for next: test runs rustc directly via check_kernels_contract (no PATH gate — rustc assumed present, which it is in the Nix dev shell; mirrors the existing bad-rust-check-failed test). Sanitisation collisions across distinct stems (a.b and a-b both -> a_b) are harmless: each rustc invocation compiles exactly one file, the crate name is internal to a throwaway .rmeta.
<!-- SECTION:FINAL_SUMMARY:END -->
