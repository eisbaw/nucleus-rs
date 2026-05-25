---
id: TASK-0323
title: >-
  Cross-file doc-lie sweep: pthreads-async/multi_worker.rs + walker narrative
  paragraphs still claim 'two backends' / 'slot vs ring' (TASK-0322 cycle-141
  architect P3 #1 follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 12:25'
updated_date: '2026-05-25 12:48'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 142 + 142b implementation summary

### What landed

**Cycle 142 (commit f5e3346)**: rewrote 9 narrative paragraphs across
3 files (4 originally cited in TASK-0323 description + 5 silent
siblings discovered in scope-expansion):

- `nucleus/backend-common/src/multi_worker_walker.rs` — module-doc
  L1-2 + L14 + L45-46 + L60 + render_worker_events rustdoc L382 +
  TASK-0209 inline comment L473 + wait_slice rustdoc L933.
- `nucleus/backends/pthreads-async/src/multi_worker.rs` — module-doc
  L32-35 + RingId rustdoc L76 + Plan::emit narrative L557-562.
- `nucleus/backends/pthreads-sync/src/multi_worker.rs` — SlotId
  rustdoc L127.

Each rewrite carries a grep-witness anchor enumerating the three
prefix-using backends (slot/ring/chan) and the mp-tcp-bufsync bypass.

**Cycle 142b (commit 5eb531b)**: review fold-back addressing all
three review findings:

- **Architect P1 (NO-GO, in-thread fix)**: `mp-tcp-bufsync/src/
  lib.rs:328` carried the same "both backends" doc-lie pattern
  (host election claim now valid across ALL FOUR tier-1 backends,
  not just bufsync+pthreads-sync). The cycle-142 residuals grep glob
  was `nucleus/backends/*/src/multi_worker.rs`; mp-tcp-bufsync has
  NO multi_worker.rs (its multi-worker code lives in lib.rs), so
  the glob STRUCTURALLY excluded the one site where the sibling
  lived. Rewritten to enumerate all four tier-1 backends.
- **QA P2 (in-thread fix)**: field-init line stamps in WalkerCtx
  rustdoc were 536/516/493; actual was 538/522/493 (off by +2 and
  +6 — fresh-written stamps in cycle 142 were never re-grepped
  before stamping). Digit-only update applied; re-grepped post-
  edit. The cycle-141 stamp-twice pattern fired AGAIN on this
  fold-back's own narrative-content edit; emit-template stamps
  shifted 831/851 → 833/853, re-anchored.
- **Architect P3.1 (memory promotion)**: cycle-141 stamp-twice
  lesson promoted to long-term memory at
  `~/.claude/projects/-home-mpedersen-topics-mark-thesis/memory/feedback-stamp-twice-when-narrative-content-shifts-line.md`
  with cross-link from MEMORY.md.

### Scope-expansion disclosure

TASK-0323 originally cited 4 sites; cycle 142 expanded to 9; cycle
142b added 1 more (the P1 silent sibling at mp-tcp-bufsync/lib.rs).
**Net: 10 sites fixed across 4 files** in the
`render_worker_events`-family doc-lie sweep.

Final honest-residuals list (7 sites, all genuinely correct in
context, NOT defects):

1. `pthreads-async/multi_worker.rs:32` — cycle-26 historical
   narrative ("was lifted out of both backends into ...").
2. `backend-common/multi_worker_walker.rs:2` — "Originating
   consumers were the pthreads-sync and pthreads-async backends"
   framing.
3. `mp-tcp-event/multi_worker.rs:21` — DATA+CTRL split, two-mp-tcp
   framing.
4. `mp-tcp-event/src/lib.rs:49` — DATA+CTRL split, two-mp-tcp
   framing (architect P2.1 disclosure).
5. `mp-tcp-event/src/lib.rs:95` — TCP-transport tradeoff column,
   two-TCP framing.
6. `pthreads-async/tests/skeleton.rs:284` — specific cross-backend
   differential test comparing exactly two backends.
7. `mp-tcp-bufsync/tests/check_frame_emit.rs:537` — cycle-23
   incident narrative about a TASK-0236 specific pair.

### AC status

- **AC#1**: DONE. Every rewritten paragraph honestly enumerates the
  three prefix-using backends (plus mp-tcp-bufsync bypass / fourth
  tier-1 backend where the framing is "all backends"); each
  carries a grep-witness anchor following the cycle-141 pattern.
- **AC#2**: DONE. Where paragraphs also made count claims (ONE
  string, TWO backends, FOUR substitutions), counts updated to
  current reality.
- **AC#3**: DONE. Cycle-141 fold-back pattern (in-thread for small
  precise findings) is now the established canonical example in
  the WalkerCtx rustdoc.

### Gates

- `just build && just clippy`: green (no warnings under `-D warnings`).
- `just test` (dev): 874/0/3 (baseline preserved end-to-end).
- `just test-release`: 874/0/3 (baseline preserved).
- `just e2e`: 108/92/0/16/0 (baseline preserved).

### Review gate

Parallel read-only review on commit f5e3346:
- qa-test-runner: GO with one P2 (field-init stamps off by +2/+6;
  fixed in cycle 142b).
- mped-architect: NO-GO with P1 (silent sibling at
  mp-tcp-bufsync/lib.rs:328; fixed in cycle 142b) + P2.1 (secondary
  honest-residual disclosure; included in this final summary) +
  P3.1 (memory promotion; landed in cycle 142b).

Both findings resolved in cycle 142b; no further review gate
re-run because cycle 142b is a doc-only fold-back to a doc-only
cycle (no behaviour change possible).

### Lessons forward-carried

1. **Stamp-twice protocol** when narrative-content edits shift the
   cited lines — promoted to long-term memory (see
   feedback-stamp-twice-when-narrative-content-shifts-line.md).
2. **Glob-completeness check** when claiming "defect-class
   wipeout": the cycle-142 grep glob excluded mp-tcp-bufsync by
   construction because bufsync's file layout (monolith lib.rs) is
   different from the other three backends' (split into
   multi_worker.rs). When sweeping across the backend family, use
   `nucleus/backends/*/src/{lib,multi_worker,...}.rs` or
   unrestricted `nucleus/backends/` rather than assuming a uniform
   file layout.
3. **Silent-sibling meta-rule firing for the SIXTH time** in this
   session — the cycle that was specifically intended as "defect-
   class wipeout" itself missed a silent sibling. Reinforces the
   discipline: scope expansion does not absolve the grep
   completeness check.

### Cycle conclusion

All ACs met; all review findings resolved; baseline preserved.
Closing as Done.
<!-- SECTION:NOTES:END -->
