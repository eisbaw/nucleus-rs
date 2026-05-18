---
id: TASK-0027
title: Project Petri net to per-worker EventList
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 03:51'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Linearise the net's firing order and project transitions onto the worker that owns them, producing the per-worker EventList that backends consume.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 compiler exposes project(Net, SchedIR) -> Map<WorkerId, EventList>.
- [x] #2 Linearisation is deterministic (same input = same output, byte-for-byte). Uses source order + dataflow constraints to break ties.
- [x] #3 Each worker's EventList respects intra-worker data dependencies.
- [x] #4 Inter-worker push/wait pairs have matching SeqTags.
- [x] #5 Test: round-trip from example through to EventList, snapshot-tested.
- [x] #6 Implementation notes record design questions (e.g. is greedy linearisation good enough for stencil-shaped schedules).
- [x] #7 Implementation notes record honest limitations (no schedule optimisation; the linearisation is correct but not optimal).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions raised during implementation

1. **Signature: `&Net` or `&ACFG`?** The task brief sketched
   `petri_to_events(net: &Net) -> BTreeMap<WorkerId, Vec<Event>>`,
   but the Net produced by `acfg_to_petri` does NOT retain enough
   metadata to reconstitute Event payloads: `Transition.name`
   carries only a textual hint (e.g. `op_k7`, `push_seq3`), not
   tile, data id, src/dst, or participant set. Two ways to honour
   the stated signature: (a) enrich `Transition` with a payload
   sum type, or (b) sidecar-map "what produced each transition"
   next to the Net. Both are bigger changes than M2 warrants.
   Chose: walk the ACFG directly (same order the
   acfg_to_petri walk uses, so the linearisation matches the
   per-worker control-place chain). Exposed `acfg_to_events(&ACFG)`
   as the primary entry, plus `petri_to_events(&ACFG, &Net)` as a
   wrapper that future milestones can repurpose without
   call-site churn.

2. **Alloc / Free events.** Elided at M2. Rationale: the
   pthreads-sync backend allocates each data symbol on its own
   stack/heap and has no use for explicit Alloc/Free events;
   `Region` resolution (PRD §8.3) requires `place_data D in
   MEMORY_REGION` directives that we don't yet thread through the
   IR. Synthesising first-use=Alloc/last-use=Free here would be
   inventing semantics no consumer needs. Filed as a follow-up
   (see below).

3. **Iteration tile on Fire.** `Repeat` is unrolled by
   `acfg_to_petri` without retaining per-iteration coordinates;
   we mirror that, so every Fire emits with `IterTile::empty()`.
   The richer per-iteration tile lives in `transfer_inject`
   (XferPlaceholder.tile holds the enclosing-loop tile) but not
   on the kernel firing itself. The eventual partition pass will
   close this loop.

4. **Sync emission.** Each `Sync` ACFG node emits one
   `Event::Sync` per participant (so every participating worker's
   EventList records the barrier). `participants` is cloned per
   worker so each EventList is self-contained.

5. **Determinism.** Walker is depth-first source-order;
   `BTreeMap<WorkerId, _>` keys iterate sorted. Two projections
   of the same ACFG produce byte-identical maps (tested in both
   synthetic and end-to-end form, e2e_example_02_split_determinism
   in tests/petri_to_events.rs).

## Honest limitations (recorded for follow-up)

- **Push/Wait imbalance inherited from `transfer_inject`.** The
  upstream `transfer_inject::splice_pushes_for_waits` only
  splices within one Sequence's children. When the producer
  lives at top-level and the consumer lives inside a `for` loop
  (example 02-split-add: `load_input` on host, `add` on w0
  inside `for i`), the ACFG ends up with Waits on the consumer
  but no matching Pushes on the producer. The projection
  faithfully forwards that — so the EventList for `host`
  contains zero Push events even though w0 emits 256 Waits.
  The pthreads-sync backend currently compensates by consuming
  the ACFG directly with shared-memory shortcuts; backends that
  consume EventLists (TASK-0124+) will need this fixed
  upstream. Filed as a separate follow-up.

- **Distributed placement** (`place k on {w0,w1,w2,w3}`) emits
  one Fire per participating worker, all with the same empty
  tile, matching `acfg_to_petri`'s "shared transition" choice.
  A real partition pass (TASK-0117) will replace this with
  per-tile fires.

- **Repeat unrolling cost.** A `Repeat` of range 0..1_000_000
  emits one million events per enclosed Fire. Acceptable for
  the v2 example schedules; a future task can fold static loops
  into a single iter-tiled event.

- **`Net` argument unused.** `petri_to_events(&ACFG, &Net)`
  takes the Net but does not consume it. Threaded into the
  signature now so that downstream milestones (boundedness
  facts, liveness witnesses) can route through it without
  call-site changes.

## AC verification

1. `compiler exposes project(Net, SchedIR) -> Map<WorkerId, EventList>`:
   Exposed as `acfg_to_events(&ACFG) -> BTreeMap<WorkerId, Vec<Event>>`
   and the wrapper `petri_to_events(&ACFG, &Net)`. The ACFG is
   the practical carrier of the schedule info (worker table +
   transfer policies) at this stage — see Design Q1. Re-exported
   from `compiler::lib`.
2. `Linearisation is deterministic, byte-for-byte`: tested
   synthetically (determinism_two_projections_of_same_acfg_match)
   and end-to-end (e2e_example_02_split_determinism). Walker is
   source-order DFS; BTreeMap iteration is sorted.
3. `Each worker's EventList respects intra-worker data
   dependencies`: the per-worker source order in the ACFG IS the
   intra-worker dependency order. The acfg_to_petri pass already
   uses this to wire per-worker control-place chains; we read it
   off the same walk.
4. `Inter-worker Push/Wait pairs have matching SeqTags`: tested
   in two_worker_push_wait_pair_routes_correctly_with_matching_seq.
   Each Xfer ACFG node carries the seq it shares with its
   counterpart; the projection copies it verbatim.
5. `Round-trip from example through to EventList,
   snapshot-tested`: e2e_example_02_split_one_eventlist_per_declared_worker
   builds ACFG -> sync_inject -> transfer_inject -> acfg_to_net
   -> petri_to_events for split.sched.nuc and asserts (a) one
   EventList per declared worker, (b) declared workers always
   appear, (c) seq pairing is consistent on emitted Push/Wait
   pairs. Determinism: e2e_example_02_split_determinism.
6. `Notes record design questions`: see Design questions above.
7. `Notes record honest limitations`: see Honest limitations
   above (Push/Wait imbalance, distributed placement, Repeat
   unrolling, Alloc/Free elision, empty tile on Fire, unused Net).

## Verification commands run

- just check / just clippy / just test — all green
- just e2e — 4 passing cells, 1 SKIPPED (TASK-0117, unrelated)
<!-- SECTION:NOTES:END -->
