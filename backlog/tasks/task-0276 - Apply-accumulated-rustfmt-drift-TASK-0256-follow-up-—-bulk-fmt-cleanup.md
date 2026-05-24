---
id: TASK-0276
title: Apply accumulated rustfmt drift (TASK-0256 follow-up — bulk fmt cleanup)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 09:51'
updated_date: '2026-05-24 11:49'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 97 closure (orchestrator + parallel review gate, 2026-05-24)

### Commits
- 0138df8 fmt: apply accumulated rustfmt drift (TASK-0276) — 54 .rs files, 1354/1022 (+332 net) lines, pure cargo fmt --all.

### Gate (verified twice for non-flaky confirmation)
- just e2e: total: 92 pass: 77 fail: 0 skipped: 15 required-fail: 0 (byte-identical)
- just determinism-check: total: 92 pass: 77 fail: 0 skipped: 15 (GREEN, bit-identical annotations survived fmt sweep — honest-scope claim 'rustfmt does not affect emit-time rendered bytes' confirmed)
- just fmt-check: exit 0 (THE GATE IS NOW EFFECTIVE GOING FORWARD)
- just clippy: clean
- just test: 67 buckets test result: ok, 0 failed across workspace

### Parallel review gate
- qa-test-runner: GO. Diff verified mechanical-only by spot-check of e2e/main.rs (highest churn 600 lines), algo/parser.rs (largest hunk 346 lines), sched/lower.rs. All changes whitespace/wrapping/use-reorder. Zero corpus/include_str! files touched.
- mped-architect (read-only): GO. 4 classic rustfmt buckets confirmed (use reorder, signature wrap, closure expansion, format! arg collapse). 46/46 removed/added line-comment counts match (re-flow only). Three pub use reorderings flagged P3 but harmless (distinct identifiers, Rust pub use ordering doesn't affect public surface when names don't collide).

### AC status (all 6 met)
1. Single commit fmt: apply accumulated rustfmt drift (TASK-0276): 0138df8 ✓
2. just fmt-check returns 0 post-commit: verified ✓
3. just e2e 92/77/0/15/0 preserved: verified ✓
4. just determinism-check green preserved: verified ✓
5. just test 0-failed preserved: verified ✓
6. cargo clippy --all-targets -- -D warnings clean preserved: verified ✓

### Honest limits
- The fmt-check gate IS now effective for future commits — any developer's local fmt drift will fail loud at the gate. This is exactly the constraint TASK-0256 (cycle 91) targeted.
- No forward-carries to file: TASK-0276 is mechanical cleanup; no architectural lessons. Three pub use reorderings are pinned by the gate going forward — future PRs touching those files must accept rustfmt's ordering.

### Cycle 97 outcome
TASK-0276 Done; gate effective; no hardening required (both reviewers GO with zero material findings).
<!-- SECTION:NOTES:END -->
