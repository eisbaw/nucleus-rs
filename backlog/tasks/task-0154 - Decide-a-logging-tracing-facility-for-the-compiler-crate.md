---
id: TASK-0154
title: Decide a logging/tracing facility for the compiler crate
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 09:22'
updated_date: '2026-05-19 04:27'
labels:
  - compiler
  - tooling
  - decision
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Several tasks (TASK-0151 AC#2, future diagnostics) want traceable debug output (e.g. 'cross-scope finalisation skipped for block-governed seq N'). The compiler crate currently has NO logging facade and deliberately minimal deps (chumsky/syn/serde only; MSRV 1.83; no env_logger/tracing). Adding one is a project-wide decision: log+env_logger vs tracing vs a tiny in-house cfg!(debug)-gated eprintln helper vs a structured diagnostics sink surfaced via the driver. Until decided, deferral points are documented in-code with TASK references instead of logged. Pick an approach consistent with PRD tech-stack and the no-spam ethos.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A logging/diagnostics approach is chosen and documented in PRD or a decision record
- [x] #2 transfer_inject per-subtree skip emits a traceable message via the chosen facility (closes TASK-0151 AC#2)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: Decide facility. Reject log+env_logger/tracing (contradicts PRD §12 "three tools, each doing one thing", minimal-dep ethos, MSRV pin). Choose a zero-dep, runtime env-var-gated (NUC_TRACE) nuc_trace! macro -> stderr, silent unless set. Mirrors existing NUC_NONDET_TEST / NUC_XBACKEND_NEGATIVE precedent. Env-gate over cfg!(debug_assertions): runtime-selectable without rebuild + matches existing NUC_* discipline.
2. Document decision: backlog decision create (decisions/ dir empty -> establish via CLI) + short transfer_inject doc note.
3. Add nucleus/compiler/src/trace.rs: nuc_trace! macro, no-op unless NUC_TRACE set; register module in lib.rs.
4. Pass A (~862): walk the opaque Repeat for deferred Wait/Push placeholders, emit one trace line per deferred (data symbol + seq), marked TASK-0149/0150 per-tile deferral (not error).
5. Pass B (~1185): emit trace for each Wait excluded by contains_block_inner (data symbol + seq).
6. Fix lying module-doc/comments that say "documented in-code instead of logged".
7. Test in transfer_inject_hoist.rs: gate-on => trace fires for mixed program, gate-off => silent. Use subprocess or BufWriter sink.
8. Gate: just test / just e2e (must stay 30/26/0/4) / determinism-check + -negative / xbackend-check-negative / clippy -D warnings / just ci. Prove default path byte-silent.
9. Commit (no AI credit). Reconcile TASK-0151 AC#2.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED (commit 36a27c2). Gotchas/lessons feed-forward (subagents stateless):

- WHY no log/tracing crate: PRD §12 "three tools, each doing one thing" + flake-pinned MSRV + no-spam ethos. env_logger drags regex/aho-corasick; a logging facade+backend is 2 crates + a 2nd MSRV surface for ~a handful of deferral lines. Disproportionate. A real crate is only warranted for level/target filtering or 3rd-party log interop — internal pass tracing needs none. Documented in backlog/decisions/decision-0001 (decisions/ dir was empty; established via `backlog decision create`, body filled by editing the file — decisions have no CLI body setter, unlike tasks).
- env-gate vs cfg!(debug_assertions): chose runtime NUC_TRACE env-gate. cfg! is compile-time (rebuild to toggle, no release emission, diverges from existing NUC_* precedent). NUC_TRACE mirrors NUC_NONDET_TEST / NUC_XBACKEND_NEGATIVE exactly — value-gated, runtime, zero-dep.
- PROOF default path is byte-silent: just determinism-check stayed byte-identical (5 files/cell) and just e2e stayed 30/26/0/4 AFTER instrumenting both skip sites. The macro guard returns before format_args! when NUC_TRACE unset AND no test sink. Any unconditional eprintln would have broken determinism-check — it did not.
- name_data threaded into hoist_invariant_waits + collect_waits so the trace is emitted at the skip DECISION site (MPED: trace where the choice is made, not reconstruct elsewhere). collect_waits Pass B early-returns on the opaque branch — had to emit before the return or the deferred set is lost.
- Comment-honesty: module doc said deferral "silently defers" / invisible — updated to "traceable, not invisible" + cited NUC_TRACE and the new test. (Comment-honesty is a recurring reviewed defect class here.)
- Pre-existing UNRELATED clippy break: cargo clippy --workspace --all-targets fails on nucleus/e2e (~line 2253, empty-line-after-doc-comment) on CLEAN master. The project gate (just clippy / just ci) does NOT use --all-targets so it is green. Filed TASK-0186.
- Forward-carried: future diagnostics tasks should reuse nuc_trace! (compiler::trace::TraceCapture for testable assertions); escalate to a driver-side structured sink only for user-facing diagnostics (would supersede decision-0001).

Gate (actual numbers): just test all suites ok 0 failed (1 pre-existing ignored); transfer_inject_hoist 14/14 incl 2 new trace tests; just e2e total 30 pass 26 fail 0 skipped 4 required-fail 0; determinism-check 30/26/0/4 byte-identical; determinism-check-negative bites (25/1); xbackend-check-negative bites (required-fail 1); just clippy clean; just ci exit 0.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a zero-dependency, runtime NUC_TRACE-env-gated `nuc_trace!` diagnostics facility to the compiler crate and used it to make transfer_inject's block-governed cross-scope deferral traceable. Closes TASK-0151 AC#2.

AC#1 (facility chosen + documented): rejected log+env_logger / tracing (contradicts PRD §12 "three tools, each doing one thing", minimal-dep ethos, flake-pinned MSRV; a facade+backend = 2 crates + a 2nd MSRV surface for a handful of lines). Rejected cfg!(debug_assertions) (compile-time, no release emission, diverges from existing precedent). Chose a runtime env-var gate mirroring the established NUC_NONDET_TEST / NUC_XBACKEND_NEGATIVE discipline. Documented as backlog/decisions/decision-0001 (status accepted).

AC#2 (transfer_inject skip emits traceable message): both block-governed skip sites — Pass A `hoist_invariant_waits` opaque-Repeat arm and Pass B `collect_waits` exclusion — now emit one trace line per deferred Xfer naming the data symbol + seq (+ role/src/dst), worded as a TASK-0149/0150 per-tile deferral, not an error. `name_data` threaded to the decision sites so the trace fires where the skip is decided.

Changes: new nucleus/compiler/src/trace.rs (nuc_trace! macro + thread-local TraceCapture test sink); registered in lib.rs; transfer_inject.rs instrumented + stale "silently defers/invisible" module doc corrected to "traceable, not invisible"; 2 new tests in transfer_inject_hoist.rs.

Default path byte-silent: macro guard returns before formatting when NUC_TRACE unset and no test sink. Proven by determinism-check (byte-identical) and unchanged e2e snapshot.

Tests run (actual): just test all suites green 0 failed; transfer_inject_hoist 14/14 (incl block_deferral_is_traceable_under_nuc_trace + deferral_trace_is_silent_by_default); just e2e total 30 pass 26 fail 0 skipped 4 required-fail 0; determinism-check 30/26/0/4 byte-identical; determinism-check-negative + xbackend-check-negative still bite; just clippy clean; just ci exit 0.

Follow-up filed: TASK-0186 (pre-existing, unrelated: clippy --all-targets fails on nucleus/e2e; project gate unaffected).

Risks/limitations: traces emitted per-Xfer in opaque subtrees (could be verbose on large block nests under NUC_TRACE, but off by default and diagnostic-only). The facility is internal-pass-only; user-facing structured diagnostics would warrant the driver-sink option (would supersede decision-0001).
<!-- SECTION:FINAL_SUMMARY:END -->
