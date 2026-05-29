---
id: TASK-0363
title: >-
  check_kernels_contract rustc_check breaks on --kernels filenames with dots
  (e.g. kernels.embedded.rs -> invalid crate name)
status: To Do
assignee: []
created_date: '2026-05-29 08:00'
updated_date: '2026-05-29 09:00'
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
- [ ] #1 rustc_check passes an explicit --crate-name (sanitised: dots/dashes -> underscores) so a --kernels file with a dotted stem (kernels.embedded.rs) passes phase-1 rustc check
- [ ] #2 A test pins a dotted-stem kernels file no longer triggers RustCheckFailed
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Architect P2.2 (TASK-0049.06 review): the driver warning text at nucleus/driver/src/main.rs:339 hardcodes "(aggregate-typed I/O is a known gap, TASK-0012)" — so when this dotted-stem rustc_check failure fires, the build output MISATTRIBUTES it to TASK-0012. When fixing this task (sanitised --crate-name), also fix/disambiguate that warning text so the dotted-stem cause is not conflated with the TASK-0012 aggregate-typed-IO gap. Also: documenting-not-fixing was accepted for the TASK-0049.06 cycle, but the consequence is rustc_check ALWAYS fails for kernels.embedded.rs, so a real Rust syntax error in that file would NOT be caught at phase 1 (only later at the embedded cross-compile) — mild safety-net erosion, another reason to do the one-call-site fix.
<!-- SECTION:NOTES:END -->
