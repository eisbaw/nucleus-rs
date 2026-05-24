---
id: TASK-0280
title: >-
  NUC_TRACE facility now orphaned — decide: repurpose or remove (TASK-0267
  follow-up)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 13:43'
updated_date: '2026-05-24 17:33'
labels:
  - infra
  - tooling
  - follow-up
  - TASK-0267
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0267 (cycle 101) removed the `trace_block_deferral` helper in
`nucleus/nucleus-compiler/src/passes/transfer_inject.rs` — the ONLY
in-source consumer of the `nuc_trace!` macro. Grep:
```
grep -rn 'nuc_trace!\|trace_enabled\|test_sink_active' nucleus/nucleus-compiler/src/
```
confirms zero remaining callers of `nuc_trace!` outside `trace.rs`'s
own definition. The facility (and its `TraceCapture` test-side sink)
is now exported but unused.

Decision required:
1. **Keep + document.** The facility was decision-driven (CLAUDE.md
   decision-0001: zero-dep `NUC_TRACE` env-gated `nuc_trace!`; do
   NOT add log/tracing). Keeping it preserves the convention for
   FUTURE diagnostic needs. Cost: a tiny module (~150 LoC) with
   no callers.
2. **Remove the facility entirely.** Smaller surface area; future
   diagnostic-trace needs would re-introduce or substitute. Risk:
   the next person solving a TASK-0151-shaped deferral problem
   re-invents the wheel.
3. **Re-purpose it as the default diagnostic path** (audit which
   other passes — sync_inject, petri_to_events, partition_*,
   halo_inference — could profit from `NUC_TRACE` instrumentation,
   add the calls).

Recommendation: option (1) is the lowest-friction; this task is the
documented decision artefact. If option (3) is chosen, it should be
a deliberate cycle of its own with a precise list of injection sites.

Acceptance criteria:
- A decision is made and recorded (in a code comment at
  `nucleus/nucleus-compiler/src/trace.rs:1`, or as a decision note
  in PRD §X).
- If option (1) is chosen: a comment at `trace.rs:1` notes that the
  facility is preserved as a convention even with zero in-source
  callers; the test file `trace_capture_tests.rs` (if any) is
  retained or removed in keeping with the decision.
- If option (2): the facility is removed (`trace.rs` deleted,
  `nuc_trace!` export removed, the `TraceCapture` helper removed,
  `CLAUDE.md`'s decision-0001 is updated/removed).
- If option (3): each new call site is reviewed for "would
  `NUC_TRACE` output help diagnose a future regression in this
  pass?" and added or skipped consciously.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## CYCLE-108 LANDING (orchestrator-led, 2026-05-24)

TASK-0280 closed. Decision: option (1) KEEP + preserved-as-convention.

### Audit at cycle 108

Re-grepped all in-source nuc_trace!/NUC_TRACE/trace_enabled references. Production consumer set is NOT empty (the task brief's 'zero in-source callers' claim was correct AT FILING TIME — cycle 101 right after TASK-0267 removed transfer_inject::trace_block_deferral — but cycles 96/101 introduced/preserved):

- nucleus/driver/src/main.rs:399 — emits halo_inference advisory errors (PartitionAware mode; non-fatal when the affected iv has no partition= directive in scope, so the transfer_inject halo consumer would not fire on it). Landed cycle 96 (TASK-0275 (B) partition-policy-aware promotion).

### Implementation

trace.rs module docs updated to:
1. Reflect the live consumer set (the driver halo_inference advisory site).
2. Document the decision: preserve per PRD section 12 + CLAUDE.md decision-0001 (zero-dep, env-gated, do NOT add log/tracing).
3. Update the stale doctest example at trace.rs:103 (was 'transfer_inject: deferred {} seq {}' from the cycle-101-removed call site; now uses the live halo_inference advisory pattern).
4. Surface a separate observation: TraceCapture + TRACE_SINK + test_sink_active are dead code (no test in the workspace uses them). Filed as TASK-0285 for prune-or-use decision.

### Why option (1) not (2) or (3)

Option (2) REMOVE was a candidate when the facility was fully orphaned. With 1 live caller it's no longer orphaned; the alternative would be inlining eprintln! at that one site + ripping the macro. Not worth it: the convention is established (PRD section 12 decision-0001), and a second/third caller is likely as more passes gain advisory-error buckets. Option (3) REPURPOSE (add more call sites proactively) was rejected per the task brief — should be a deliberate cycle of its own, not piggy-backed on the decision artefact.

### Gate post-decision

- cargo test --workspace: 818 / 0 / 3 (unchanged; doc-only change).
- cargo clippy: clean.
- e2e + determinism not re-run (doc-only diff cannot affect emit).

### ACs MET

- Decision made and recorded in a code comment at nucleus/nucleus-compiler/src/trace.rs:3-30: MET.
- Option (1): facility preserved as convention; module docs document the current state + decision rationale: MET.

### Follow-up filed

TASK-0285 (LOW): prune or use TraceCapture. Today it has zero in-source users; the prune is ~30 LoC removal of dead RAII machinery. Independent of TASK-0280's KEEP decision (KEEP is about nuc_trace! the macro; TraceCapture is about its test-side sink helper).

Status: Done. Commit pending.
<!-- SECTION:NOTES:END -->
