---
id: TASK-0277
title: >-
  sched/ast.rs:67-68 doc-lie: claims a non-existent path resolver for schedule
  `algo_path`
status: To Do
assignee: []
created_date: '2026-05-24 10:07'
labels:
  - doc-lie
  - sched
  - parser
  - forward-carried-from-TASK-0274
dependencies:
  - TASK-0274
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Found by cycle-92 architect review of TASK-0274.

\`nucleus/nucleus-compiler/src/sched/ast.rs:67-68\` says:

\`\`\`rust
/// Path-string from \`schedule for "<algo_path>" { ... }\`. Stored
/// verbatim; the build driver resolves it relative to the
/// schedule file (grammar §2 note 9).
pub algo_path: String,
\`\`\`

This is a comment-doc-lie. The driver does NOT resolve \`algo_path\` from the schedule file — it uses the \`--algo\` CLI argument directly (\`nucleus/driver/src/main.rs:161\`: \`let algo_path = a.algo.ok_or("missing required --algo")?\`). The string in \`schedule for "..."\` is stored verbatim but never consulted by the driver.

Verified by:
- grep \`algo_path\` across \`nucleus/driver/src/\` and \`nucleus/nucleus-compiler/src/\` — only appears as the SchedAst field (stored) or in test fixtures. No reader.
- Cross-cycle context: TASK-0274 fixture's \`schedule for "./prog.algo.nuc"\` worked despite the path being relative-to-CWD (not relative-to-sched-file); the driver never opens this path.

## Why this matters (recurring failure-mode)

This is the \`feedback-comment-doc-lie-recurring\` memory pattern. A reader of \`SchedAst\` is told the field is load-bearing for a resolver that doesn't exist, and might:
- Write code depending on the supposed resolver semantics.
- Spend time hunting for the resolver in the driver.
- Make the resolver real when the actual contract is "CLI \`--algo\` is the source of truth; the \`schedule for\` path is documentation only".

## Acceptance

1. Update the docstring at \`sched/ast.rs:67-68\` to reflect reality:
   - Stored verbatim by the parser.
   - NOT consumed by the driver (\`--algo\` CLI arg is the source of truth).
   - Useful for human readers + future tooling (e.g. an IDE jump-to-definition).
2. EITHER: drop the "grammar §2 note 9" reference if grammar §2 itself does not promise resolution, OR: update the grammar doc to match this clarification.
3. Add a unit test (or doc-test) asserting the field is stored verbatim from the parser — a contract pin that codifies "no resolution happens here".
4. \`cargo test --workspace\` stays 0 failed. \`cargo clippy -D warnings\` stays clean.

## Honest scope

LOW priority cosmetic + correctness-of-docs. The behaviour is fine; only the doc lies. A reader-cost not a correctness-cost.

If the long-term intent IS to add a resolver (e.g. so the schedule can be invoked without \`--algo\` by reading \`algo_path\`), that's a DIFFERENT task — file separately.

## Dependencies

Forward-carried from: TASK-0274 cycle-92 architect P2-1 finding.
Related memory: \`feedback-comment-doc-lie-recurring\`.
<!-- SECTION:DESCRIPTION:END -->
