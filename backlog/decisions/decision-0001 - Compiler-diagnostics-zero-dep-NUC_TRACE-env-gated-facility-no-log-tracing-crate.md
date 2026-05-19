---
id: decision-0001
title: >-
  Compiler diagnostics: zero-dep NUC_TRACE env-gated facility, no log/tracing
  crate
date: '2026-05-19 04:25'
status: accepted
---
## Context

Several tasks want traceable compiler debug output (TASK-0151 AC#2:
"cross-scope finalisation deferred for block-governed seq N"; future
diagnostics tasks). The `compiler` crate deliberately carries only four
dependencies (chumsky / syn / quote / serde), each pulling its weight,
with the MSRV pinned in the Nix flake (PRD §12.1, *not* in Cargo.toml)
and a hard no-spam ethos (PRD §12.3; `~/.claude/CLAUDE.md`).

Options weighed: (a) `log` + `env_logger`, (b) `tracing` +
`tracing-subscriber`, (c) a tiny in-house `cfg!(debug_assertions)`-gated
`eprintln!` helper, (d) a tiny in-house **runtime env-var-gated**
`eprintln!` helper, (e) a structured diagnostics sink surfaced via the
driver.

## Decision

Adopt (d): a zero-dependency `nuc_trace!` macro in
`nucleus/compiler/src/trace.rs`, **silent unless the `NUC_TRACE`
environment variable is set to a non-empty value**, writing to stderr.

Rationale, argued against PRD §12 explicitly:

- **vs (a)/(b) — a real logging crate.** PRD §12 is "Three tools, each
  doing one thing." A logging facade + backend is two more crates plus
  their transitive trees (`env_logger` drags `regex`/`aho-corasick`), a
  second MSRV surface that can drift past the flake pin (§12.1), and a
  global mutable logger init that fights "no hidden machinery" (§12.2).
  That cost is not proportionate to a handful of structural
  deferral-trace lines. A real crate is only justified if we need
  level/target filtering, structured fields, or third-party log
  interop — none of which a research pre-compiler's internal deferral
  tracing requires. Rejected.
- **vs (c) — `cfg!(debug_assertions)`.** Compile-time knob: toggling
  trace needs a rebuild, the release binary can never emit it, and it
  diverges from the existing `NUC_*` precedent (all runtime-gated).
  Rejected.
- **(d) chosen because it is the discipline the codebase already
  standardised on**, not a novel mechanism: `NUC_NONDET_TEST`
  (`pthreads-sync/src/multi_worker.rs:288`) and `NUC_XBACKEND_NEGATIVE`
  (`mp-tcp-bufsync/src/lib.rs:1154`) are both
  `std::env::var(..).as_deref()`-gated, value-gated, loud-stderr, zero
  new dependency. `NUC_TRACE` mirrors that exactly. Runtime-selectable
  without recompiling, works on the shipped binary.
- **(e)** is the right long-term home for *user-facing* diagnostics but
  is over-scoped for internal pass tracing; revisit if/when the driver
  grows a structured diagnostics channel.

## Consequences

- New module `nucleus/compiler/src/trace.rs` exporting the
  `#[macro_export] nuc_trace!` macro; zero new crate dependency; zero
  MSRV surface added.
- **Default path is byte-silent.** `NUC_TRACE` unset ⇒ the macro guard
  returns without formatting args or touching stderr/stdout. Proven:
  `just determinism-check` stays byte-identical and `just e2e` stays
  30/26/0/4 after instrumenting two transfer_inject skip sites.
- A thread-local `TraceCapture` test sink lets tests assert emitted
  lines deterministically without racing the process environment or
  scraping real stderr.
- Future diagnostics tasks should reuse `nuc_trace!` rather than
  introduce a logging crate; escalate to option (e) only if user-facing
  structured diagnostics are needed (then supersede this record).
- Closes the facility half of TASK-0154 (AC#1) and unblocks
  TASK-0151 AC#2.
