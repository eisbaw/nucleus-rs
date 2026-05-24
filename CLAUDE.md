# Nucleus v2 — project-level guidance for Claude Code sessions

This file is auto-loaded into every Claude Code session in this repo.
For broader project context, the human-facing entry points are
`nuc-nucleus/PRD.md` (product requirements + invariants) and the
`backlog/` tracker (`backlog task list --plain`, never use the TUI
in a Claude session).

## Tracker

- Task state lives in `backlog/` and is managed exclusively through
  the `backlog` CLI. Never hand-edit task markdown files.
- Always pass `--plain` to `backlog task view`, `backlog task list`,
  `backlog board`, etc. — the default TUI hangs in a non-interactive
  shell.
- Commits do NOT include AI / Claude co-author trailers (deliberate
  project policy).

## Verification gate

The full hard gate is `just ci`. It runs (in order): `just check`,
`just clippy`, `just test`, `just test-release`, `just
check-textual-replace-on-codegen`, `just check-include-str-coverage`,
`just e2e`, plus four negative/determinism arms.

The cheap subset you should run before EVERY commit is:

```
nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"
```

E2E baseline as of the most recent landed cycle: track it via `git
log` — it is recorded in commit messages of recent tracker-close
commits. A regression in the totals line is a hard failure.

The `test-release` step is load-bearing: it catches `debug_assert!`-
gated `#[should_panic]` divergence that `just test` (dev profile)
silently hides (see TASK-0291).

## Recurring defect patterns (read before reviewing or implementing)

These patterns recur often enough that the durable memory at
`~/.claude/projects/-home-mpedersen-topics-mark-thesis/memory/`
tracks them. Treat any of these as live possibilities every cycle:

1. **Comment / doc lies** (`feedback-comment-doc-lie-recurring`).
   A multi-claim docstring saying "X happens because Y" is a
   *claim*, not a *fact*. Spot-check 3-5 such comments per review;
   verify against the code each claim describes. The implementer's
   own "subtleties & gotchas" disclosure is just as susceptible
   (see `feedback-implementer-disclosure-mechanism-wrong`).

2. **Silent-sibling defects** (`feedback-silent-sibling-defect`).
   When you fix a defect at call site X, grep for every other call
   site of the same symbol/pattern before claiming closure. The
   structurally-identical sibling that silently skipped the fix is
   the #1 recurring source of "we thought we fixed this".

3. **Panic-not-diagnostic** (`feedback-panic-not-diagnostic-recurring`).
   Compiler passes that `panic!` on valid input ship as latent
   crashes for downstream users. Use `EmitError` / typed-error
   surfaces; reserve `panic!` for genuinely unreachable invariants.

4. **Opacity-gate-rot** (`feedback-opacity-gate-rot`).
   A deferral facility (an opacity gate, an advisory variant)
   filed in cycle N becomes redundant — or wrong — when cycle N+M
   lands the precise machinery that subsumes it. Audit when
   precise tracking lands.

5. **Cheap empirical verification** beats trusting the narrative.
   When an implementer reports "without X, Y panics", try removing
   X and re-running tests if the cost is bounded. Static traces
   beat narratives in code review.

## Cheap structural checks (mandatory parts of `just ci`)

- `just check-textual-replace-on-codegen` — `String::replace` on a
  rendered Rust expression is dangerous (sibling identifiers contain
  the iv as a substring). Annotate the line with `// ALLOW textual
  replace: <reason>` if you genuinely need it; default is "build the
  derived expression structurally".
- `just check-include-str-coverage` — every `include_str!` MUST be
  paired with `mod <name>;` or `include!("<path>")` in the same
  crate so `cargo test` compiles the file.

When you add a new file with one of these patterns, run the
relevant `just check-*` recipe locally — they're zero false
positives on the current tree.

## Implementer / reviewer subagent norms

- Spawned subagents (especially "implementer" briefs) sometimes
  refuse code edits citing a safety reminder. Mitigation: lead
  every implementer brief with explicit benign framing + concrete
  file pointers. See `feedback-spawned-agents-refuse-code-edits`.
- Run the parallel read-only review gate (qa-test-runner +
  mped-architect) after EVERY implementer cycle, before considering
  the cycle closed. Read-only agents do not have the refusal
  problem.

## File / commit hygiene

- Do NOT commit summary / status files. Add the summary to the
  commit message instead.
- Stale or outdated files: move to `cruft/`, do NOT delete (the
  history may still be load-bearing).
- Never move backlog task markdown to `cruft/`.
- Match the existing commit-message style: `<area>: TASK-NNNN
  <one-line>`. See `git log --oneline -10` for current convention.

## Dev shell

This repo uses Nix flakes. All `cargo` / `just` commands need to run
inside the dev shell:

```
nix develop --command <command>
```

Do not run `cargo` outside the shell — the toolchain pin
(`rustChannel` in `flake.nix`) is the single source of truth for
MSRV per PRD §12.1 / §13.
