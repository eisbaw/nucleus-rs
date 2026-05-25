---
id: TASK-0323
title: >-
  Cross-file doc-lie sweep: pthreads-async/multi_worker.rs + walker narrative
  paragraphs still claim 'two backends' / 'slot vs ring' (TASK-0322 cycle-141
  architect P3 #1 follow-up)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 12:25'
updated_date: '2026-05-25 12:30'
labels:
  - backend-common
  - pthreads-async
  - doc-lie
  - silent-sibling
  - forward-carried-from-TASK-0322
dependencies:
  - TASK-0322
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0322 cycle 141 architect P3 #1 fold-back fixed two doc-lies in
the `WalkerCtx` rustdoc at
`nucleus/backend-common/src/multi_worker_walker.rs:128-129` and
`:134-137` (lines pre-fold-back; now lines 128-150 in the rewritten
docstring). Both inherited from the cycle-31 (TASK-0239) era when the
shared walker existed for only two backends (pthreads-sync + pthreads-
async); cycle 79 added mp-tcp-event (`"chan"`) and the doc-lies went
unupdated.

**The cycle-128 silent-sibling discipline requires that the same
doc-lie pattern be swept across the codebase**, not fixed only at the
architect-cited site. This task tracks the broader sweep filed as a
honest-scope follow-up rather than expanded into cycle-141 inline.

## Sites identified (grep witness)

`grep -nE 'two backends|both pthreads-sync|pthreads-async' nucleus/backend-common/src/ nucleus/backends/`:

1. **`nucleus/backend-common/src/multi_worker_walker.rs:45-46`** —
   module docstring design-rationale paragraph:
   ```
   //! That keeps the two backends' real semantic difference (one-shot
   //! rendezvous vs bounded buffered channel) visible at the `emit()`
   //! entry point.
   ```
   Now THREE prefix-using backends with three semantic differences
   (pthreads-sync: one-shot rendezvous; pthreads-async: bounded
   buffered channel; mp-tcp-event: event-driven via mio reactor).
   The "two backends" framing is stale.

2. **`nucleus/backend-common/src/multi_worker_walker.rs:365-368`** —
   `render_worker_events` fn rustdoc:
   ```
   /// This is the SHARED walker — both pthreads-sync's `Plan` and
   /// pthreads-async's `Plan` call through it.
   ```
   Now ALSO mp-tcp-event's `Plan`. The `{prefix}_<id>.push(...)` /
   `{prefix}_<id>.wait()` substitution-surface description is correct
   (and now anchored by the cycle-141 grep witness in the
   `WalkerCtx::rendezvous_prefix` rustdoc), but the calling-backends
   enumeration is missing the third backend.

3. **`nucleus/backends/pthreads-async/src/multi_worker.rs:33-35`** —
   module-doc cycle-31 narrative paragraph:
   ```
   //!   `pthreads_sync::multi_worker_walker`, parameterised by ONE
   //!   string (`rendezvous_prefix: "slot"` for pthreads-sync, `"ring"`
   //!   for pthreads-async). Both backends now route through that ...
   ```
   Two cycle-141 lies: (a) "parameterised by ONE string" is true (only
   `rendezvous_prefix`) but the listed two prefixes miss `"chan"`;
   (b) "Both backends now route through" — three backends now.

4. **`nucleus/backends/pthreads-async/src/multi_worker.rs:557-562`** —
   inline narrative comment near `Plan::emit`:
   ```
   // and the per-worker rendezvous-id collector — is the single source
   // of truth across both pthreads-sync (rendezvous_prefix = "slot") and
   // pthreads-async (rendezvous_prefix = "ring"). This module retains
   ```
   Same pattern: stale "both" framing. Should enumerate three prefix-
   using backends.

## Acceptance criteria

1. Rewrite each of the four cited paragraphs to honestly enumerate
   the three prefix-using backends (plus mp-tcp-bufsync's bypass
   where the framing is "all backends that use this walker"). Each
   rewrite must carry a grep-witness anchor following the cycle-141
   pattern (`grep -n 'rendezvous_prefix:' nucleus/backends/*/src/`
   yields three field-init sites — name them).

2. Where the same paragraph also makes a count claim (e.g. "ONE
   string", "TWO backends"), update the count to current reality and
   anchor with a grep witness.

3. Add a one-line cycle-141 fold-back reference if helpful (the
   discipline pattern is now established in
   `multi_worker_walker.rs:WalkerCtx` rustdoc as the canonical example).

## Honest scope

- LOW priority. No functional defect — these are doc/comment
  staleness only. The shared walker correctly handles all three
  prefix-using backends and the mp-tcp-bufsync bypass; the lies are
  in the surrounding narrative, not in the code.
- Trigger: any cycle that touches `multi_worker_walker.rs` or the
  `pthreads-async/multi_worker.rs` module-doc — would otherwise re-
  inherit the staleness. Doing it now also closes a sibling sweep
  proactively (cycle-128/138/140/141 discipline).
- Cost: small (four narrative paragraph rewrites + grep-witness
  anchors).

## Cross-reference

- TASK-0322 cycle 141 final summary (architect P3 #1 origin).
- TASK-0321 cycle 140 (the parametric Wait pin that motivated this
  whole doc-lie thread).
- MEMORY.md `feedback-comment-doc-lie-recurring` (the meta-pattern).
- MEMORY.md `feedback-silent-sibling-defect` (the sweep discipline).
<!-- SECTION:DESCRIPTION:END -->
