---
id: TASK-0276
title: Apply accumulated rustfmt drift (TASK-0256 follow-up — bulk fmt cleanup)
status: To Do
assignee: []
created_date: '2026-05-24 09:51'
labels:
  - infra
  - tooling
  - fmt
  - forward-carried-from-TASK-0256
dependencies:
  - TASK-0256
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0256 (cycle 91) added the \`just fmt-check\` recipe. On its first run it bites: ~330 drift lines across multiple crates in \`nucleus/\` — accumulated over many cycles before the gate existed.

## Scope

1. Run \`just fmt\` from the dev shell.
2. Verify \`just e2e\` stays at 92/77/0/15/0 (rustfmt should NOT affect emit-time rendered bytes, since they come from string builders not source position; the e2e cells should be byte-identical).
3. Verify \`just determinism-check\` stays green at 92/77/0/15.
4. Verify \`just test\` workspace stays 0 failed.
5. Verify \`just fmt-check\` returns 0 after the fmt run.
6. Commit as a SINGLE mechanical commit (the diff is large but reviewable — every line is rustfmt output).

## Honest scope / risk

- Rustfmt touches only Rust syntax, not literal string contents. include_str! files are untouched.
- Determinism risk: very low. Emit-time bytes come from rendered string builders, not source layout.
- Diff size will be large (~330 lines across many files). Review by accepting "all changes are rustfmt-produced" rather than line-by-line.
- ONE potential snag: if rustfmt re-orders some macro_use lines or modifies a doc-test, behaviour could subtly change. Mitigation: full gate (test + e2e + determinism) MUST pass before commit.

## Acceptance

1. Single commit: \`fmt: apply accumulated rustfmt drift (TASK-0276)\`.
2. \`just fmt-check\` returns 0 post-commit.
3. \`just e2e\` 92/77/0/15/0 preserved.
4. \`just determinism-check\` green preserved.
5. \`just test\` 0-failed preserved.
6. \`cargo clippy --all-targets -- -D warnings\` clean preserved.

## Dependencies

- Forward-carried from: TASK-0256 cycle-91 (the gate that revealed the accumulated drift).
- Should be done in a fresh session — large mechanical diff deserves its own clean cycle, not a same-session add-on.
<!-- SECTION:DESCRIPTION:END -->
