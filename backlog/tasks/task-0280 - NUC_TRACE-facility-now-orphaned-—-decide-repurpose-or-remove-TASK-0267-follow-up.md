---
id: TASK-0280
title: >-
  NUC_TRACE facility now orphaned — decide: repurpose or remove (TASK-0267
  follow-up)
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-24 13:43'
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
