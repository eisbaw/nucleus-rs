---
id: TASK-0033
title: 'Determinism CI test: byte-identical generated code'
status: To Do
assignee: []
created_date: '2026-05-17 23:06'
labels:
  - M2
  - validation
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §8 promises: same source + same backend = same emitted code, byte-for-byte. Add a CI check that compiles every (example, schedule, backend) twice and diffs the generated source.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 e2e harness gains a --check-determinism flag that builds each cell twice and byte-compares the generated Rust sources.
- [ ] #2 Any difference is a hard failure with the offending file path.
- [ ] #3 CI runs this on every commit at M2 and onwards.
- [ ] #4 Test: deliberately introducing a HashMap iteration in codegen breaks the check (proves the test bites).
- [ ] #5 Implementation notes record design questions (e.g. whether to include the Cargo.toml manifest in the byte-diff, or only .rs sources).
- [ ] #6 Implementation notes record honest limitations (e.g. timestamp comments in generated files must be stripped before comparison).
<!-- AC:END -->
