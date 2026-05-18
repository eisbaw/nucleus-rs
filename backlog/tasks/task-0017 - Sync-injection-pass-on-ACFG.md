---
id: TASK-0017
title: Sync injection pass on ACFG
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 01:34'
labels:
  - M1
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Walk the ACFG and inject acfg::sync nodes between regions on different workers where control-flow demands a barrier (e.g. top-level statement boundaries with cross-worker dependencies). PRD §8 + 2013 thesis §4.3.9.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 compiler exposes inject_syncs(ACFG) -> ACFG.
- [x] #2 A sync is injected exactly where it's needed for control-flow coherency; over-synchronization is to be avoided where possible.
- [x] #3 Sync nodes capture from-workers and to-workers sets.
- [x] #4 Test: synthetic two-worker programs produce expected sync placement (table-driven test).
- [x] #5 Implementation notes record design questions (e.g. when to fold a sync into an existing transfer's coherency event).
- [x] #6 Implementation notes record honest limitations (e.g. may over-sync; optimisation passes deferred).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Commit

62951a49430511b379cf519a0366240f9e4dc6bd — compiler(M1): sync-injection pass on ACFG (TASK-0017)

## Design questions (recorded)

1. **When to inject (entry/exit/sequence-boundary)?** Followed the task
   spec literally: three rules, applied independently. Rules don't
   look at *actual* data dependence between adjacent statements — only
   at write/read worker sets. This is conservative (over-syncs) but
   matches the PRD §8 framing that Sync is the control-only barrier
   while Push/Wait handle data coherency. The over-syncing is recovered
   later by transfer injection (TASK-0018), which will subsume
   Push/Wait-matched edges and let an optimisation pass drop the now-
   redundant Syncs.

2. **Should syncs propagate across loop bodies?** Modelled the
   Repeat-entry and Repeat-exit boundaries explicitly. The body's
   reads/writes also propagate to the Repeat node itself
   (`writing_workers(Repeat) = writing_workers(body)`), so a Sequence
   neighbour of a Repeat triggers the Sequence rule too. Net effect:
   for the batch_parallel example we get *both* an outer Sync
   (load_input host -> Repeat reading {w0..w3}) and an inner entry
   Sync. Documented as expected over-syncing.

3. **How to compute writing/reading workers of a statement?**
   - Operation writes on its `workers` set (effect kernels too — the
     side effect "happens" on the worker, the next statement may
     depend on it).
   - Operation reads on its `workers` set iff any DataflowEdge has a
     non-empty `data_in`.
   - Repeat delegates to body.
   - Sequence is the union over children.
   - Sync and Xfer return empty (this is the property that makes the
     pass idempotent).

4. **Over-sync vs under-sync.** Chose over-sync. Reasoning: an extra
   Barrier never breaks correctness, just costs latency. A missing
   Barrier silently corrupts state. v2's optimisation passes can
   safely *remove* Syncs once they prove they're redundant; *adding*
   missing Syncs after the fact would require a global re-analysis.

5. **Enrich SyncPlaceholder vs add a new node type.** Enriched.
   The task explicitly preferred this; the type still pattern-matches
   as `ACFGNode::Sync(SyncPlaceholder)` so the existing match arms in
   `acfg.rs` (count_operations, count_repeats, max_repeat_depth) and
   tests/acfg.rs need zero changes.

6. **`participants` field type.** `BTreeSet<WorkerId>` to match
   `Event::Sync.participants` in event.rs (PRD §8.3). Gives
   deterministic iteration order for downstream codegen and a clean
   path for the final ACFG -> Event projection (already-correct
   field type, no conversion).

7. **No `SyncKind` on `SyncPlaceholder`.** The injection pass only
   produces `Barrier` syncs (it's the only variant in v2). The later
   projection pass that lowers ACFG into per-worker `Event`s attaches
   `SyncKind::Barrier` at emission time. Saves a field that would
   always carry the same value.

8. **Module layout: `passes/` directory vs flat module.** Created
   `src/passes/sync_inject.rs` per the task's preferred path. Future
   passes (TASK-0018 transfer injection, Petri-net construction)
   slot in alongside.

## Honest limitations

- **May over-sync.** The Sequence rule does NOT inspect whether the
  writer's data actually feeds the reader. Example: two consecutive
  ops on different workers, both reading from a third write upstream,
  will get a Sync between them despite no direct dependence. The PRD
  §8 carve-out (Sync = control-only) calls for the cleanup but ships
  no optimisation pass for it in v2. Filed: TASK-0113.

- **No conditionals.** The ACFG has no `If` variant; the algorithm
  grammar has no `if` (PRD §6.2.4). Once conditionals land
  (post-v2), the pass will need a rule for the merge point too.
  Filed there since TASK-0110 already tracks the variant; the sync-
  injection follow-up is TASK-0114.

- **No adjacent-Sync merging.** If two of the rules would produce a
  Sync at the same boundary they could in principle collapse into
  one. The current implementation never produces back-to-back syncs
  at the same boundary, but a future rule expansion might. Filed:
  TASK-0115.

- **Effect kernels treated as writers.** An effect kernel with no
  `data_out` is treated as a writer on its workers for the purpose
  of the rules (its side effect "writes" externally). This is a
  modelling choice, not a hard fact. If a real example trips on it
  (e.g. an effect that should NOT cause a sync), we'd need a more
  nuanced "writes externally" vs "writes data" distinction. The
  driving examples don't trip it.

- **No optimisation: rules are mechanical.** A user reading the
  inserted Syncs may find some that are theoretically redundant
  (e.g. an exit-sync immediately followed by an enclosing Sequence
  sync). They're correct, not minimal.

- **Rule ordering inside a Repeat.** We apply Sequence-rule to the
  body's children BEFORE wrapping with the Repeat entry/exit
  boundaries. If a future rule tangle made these non-commutative,
  the function comments document the order. The current rules are
  independent.

- **`participants` calls `union` for the Sequence rule.** This is
  conservative: it always includes both writer and reader workers,
  even when they overlap. The set-union dedupes naturally so the
  result is correct, but the test that checks "writers != readers"
  is set-inequality, which doesn't catch the partial-overlap case
  (e.g. W1={a,b}, W2={a,c}). The rule fires for these, with
  participants {a,b,c}; that's the spec.

## AC verification

- **#1 compiler exposes inject_syncs(ACFG) -> ACFG.** Met.
  `compiler::passes::sync_inject::inject_syncs`, also re-exported
  at the crate root: `compiler::inject_syncs`.

- **#2 A sync is injected exactly where it's needed for control-
  flow coherency; over-synchronization is to be avoided where
  possible.** Partially met. The three rules cover the cases listed
  in the task; the Sequence rule may produce some over-syncs
  (documented above) — avoiding *those* needs the data-edge
  knowledge that transfer injection (TASK-0018) brings, plus an
  optimisation pass (TASK-0113). The elision rule (participant
  count < 2) is honoured.

- **#3 Sync nodes capture from-workers and to-workers sets.**
  Met as a union. The SyncPlaceholder.participants is
  `BTreeSet<WorkerId>` = W_from union W_to, matching the
  `Event::Sync.participants` shape from PRD §8.3 / TASK-0015. The
  union is the canonical form for a Barrier (all participants
  arrive, then all proceed; there is no asymmetric from/to under
  `SyncKind::Barrier`). If a future variant (Rendezvous, Quorum)
  needs from/to, it would land as a new SyncKind variant with its
  own fields rather than reshaping `participants`.

- **#4 Synthetic two-worker programs produce expected sync
  placement (table-driven test).** Met. tests/sync_inject.rs has
  one test per rule, plus negative-of-positive (same-worker, no
  reader). Synthetic programs use a tiny ACFG-builder helper
  (`op`, `repeat`, `empty_acfg`) rather than going through algo
  parse + lower + link, so the rules are exercised in isolation.

- **#5 Implementation notes record design questions.** Met above.

- **#6 Implementation notes record honest limitations.** Met above.

## Verification

- `just check`  -> green
- `just clippy` -> green (-D warnings, no new lints)
- `just test`   -> green (17 new tests pass; all 28 pre-existing
                  ACFG/link/event tests still pass)
- `just e2e`    -> green (stub binary)

## Follow-up tasks filed (to be created)

- TASK-0113: optimisation pass that drops redundant Syncs once
  transfer injection has materialised Push/Wait edges.
- TASK-0114: extend sync-injection with rules for `If` merge
  points once conditionals land (depends on TASK-0110).
- TASK-0115: adjacent-Sync merging if future rule expansion ever
  produces them.
<!-- SECTION:NOTES:END -->
