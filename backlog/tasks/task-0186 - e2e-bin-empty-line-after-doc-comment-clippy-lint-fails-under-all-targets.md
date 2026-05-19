---
id: TASK-0186
title: 'e2e bin: empty-line-after-doc-comment clippy lint fails under --all-targets'
status: To Do
assignee: []
created_date: '2026-05-19 04:26'
updated_date: '2026-05-19 04:35'
labels:
  - compiler
  - tooling
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
cargo clippy --workspace --all-targets -- -D warnings fails. Pre-existing on clean master, NOT introduced by TASK-0154 (verified: cycle commits 36a27c2/8adcc6c touch zero e2e/test files; review gate confirmed). The project gate (just clippy / just ci) does NOT pass --all-targets so it is currently green, but the TEST targets have accumulated lint rot invisible to the gate. Known offenders found by the TASK-0154 review gate: nucleus/e2e/src/main.rs (~2256, empty-line-after-doc-comment on a commented-out doc block) PLUS pre-existing test-target lints in nucleus/compiler/tests/acfg_to_petri.rs and nucleus/compiler/tests/petri_to_events.rs (~5 lints). AC#1 cannot be satisfied by fixing only the e2e line — ALL --all-targets lints must be cleared. Fix each at root cause (convert/rephrase, not blanket #[allow] unless genuinely warranted). Then decide whether the project gate (just clippy / just ci) should adopt --all-targets so test-target lint rot is caught going forward (the architect review flagged this gate gap as real).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 cargo clippy --workspace --all-targets -- -D warnings is clean
- [ ] #2 just ci still exit 0
- [ ] #3 Decide and document (decision record or PRD note) whether just clippy / just ci should pass --all-targets; if yes, wire it into the justfile gate so test-target lint rot is caught
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0154 review gate (qa-test-runner + mped-architect, both GO): the --all-targets failure is broader than the single e2e doc-comment line originally noted — also ~5 lints in compiler/tests/acfg_to_petri.rs and compiler/tests/petri_to_events.rs. All are pre-existing on clean master (git blame: e2e doc region from 946159f6 / 8875bba TASK-0167; not this cycle). AC#1 (--all-targets clean) is the real bar — do not tick it by fixing only one file. The gate deliberately omits --all-targets today (honest, disclosed) but that is a genuine gap: test-target lints are invisible to just ci — hence the new AC#3 gate-adoption decision.
<!-- SECTION:NOTES:END -->
