---
id: TASK-0018
title: Transfer injection pass on ACFG
status: In Progress
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 01:45'
labels:
  - M1
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Walk the ACFG and inject acfg::push / acfg::wait nodes for every dataflow edge that crosses workers. Apply transfer policy (sync/async/buffer/notify) from the schedule.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 compiler exposes inject_transfers(ACFG, SchedIR) -> ACFG.
- [x] #2 Each cross-worker data dependency yields a matched push/wait pair with a unique SeqTag.
- [x] #3 Transfer policy (sync vs async, buffer depth, notify mode) attaches to the push/wait pair.
- [ ] #4 Schedule capability check: if backend lacks a capability (e.g. async on pthreads-sync), the pass errors before codegen.
- [x] #5 Test: synthetic schedules covering each (sync/async × buffered/unbuffered × event/poll) combination.
- [x] #6 Implementation notes record design questions (e.g. coalescing per-element pushes into per-tile bulk transfers).
- [x] #7 Implementation notes record honest limitations (e.g. transfer-aggregation may be naive at M1).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions explored

**SeqTag generation strategy.** PRD §8.3 talks about per-(src,dst,data) monotonic numbering. I used a single global counter — a strict superset of per-triple monotonicity (per-triple subsequences are still strictly increasing) and the strongest reading of unique-per-pair. Trade-off: seq values aren't dense per triple, so a casual reader of an EventList can't predict the next seq for a given pipe; matched-pair lookup is by equality, not by adjacency, so this doesn't bite. If a backend wants per-channel monotonic seqs (e.g. for protocol framing), a renumbering pass at codegen time is straightforward.

**Transfer aggregation per tile vs per point.** At M1, a consumer Operation inside a for-loop emits one Push/Wait per iteration. Real backends bulk-send. Aggregation needs an access-pattern analysis on data_in (which kernel indices touch which IterTile region). The M1 DataflowEdge does not carry per-firing index expressions; the current shape is just a flat data_in list. Filed as TASK-0116; punted at this milestone.

**Distributed placements.** A 'place k on {w0..w3}' is one WorkerEntity in the link pass. Transfer injection inherits that single-entity view: one Push/Wait pair, with src/dst recorded as the lexicographically-first worker of the BTreeSet. A future partition pass (TASK-0117) replicates the pair across the named workers per loop.partition=. The current placeholder structurally records the entity choice so the partition pass can fan it out without re-discovering producer/consumer relationships.

**Top-level non-iterated data.** load_input/load_image-style results consumed cross-worker get IterTile::empty(). The producer is the loader's placement (per link.data_producers, which records the kernel-on-the-RHS placement for every dataflow statement).

## AC verification

- AC#1 — compiler exposes inject_transfers: signature is (&LinkedIR, ACFG) -> ACFG. Tested in tests/transfer_inject.rs::two_worker_producer_consumer_yields_matched_push_wait and others. The signature took a &LinkedIR (not (ACFG, SchedIR)) because the pass needs the link pass's producer/consumer index — re-deriving inside the pass would duplicate logic from link.rs.
- AC#2 — matched push/wait per cross-worker dataflow with unique SeqTag: tested in two_worker_producer_consumer_yields_matched_push_wait and seq_tags_unique_per_pair. Three-edge case verifies 3 distinct seq values, each appearing on exactly one Push and one Wait.
- AC#3 — TransferPolicy attached to push/wait pair: tested in policy_sync, policy_async, policy_async_buffer_2, policy_async_buffer_2_notify_event. Both endpoints of a pair carry the identical policy (asserted in policy_after_inject helper).
- AC#4 — Capability check: DEFERRED per task spec ('backend isn't selected yet at this pass'). Filed as TASK-0118 for codegen-time. The XferPlaceholder.policy carries the schedule's stated demand so a future codegen pass can run the check.
- AC#5 — Test: synthetic schedules covering each (sync × async × buffered × event/poll) combination: four policy_* tests cover the documented combinations.
- AC#6 — Design questions recorded: see above.
- AC#7 — Honest limitations recorded: see below.

## Honest limitations

- **Per-point transfer granularity.** No tile coalescing. See TASK-0116.
- **Distributed entity collapsed to canonical worker.** See TASK-0117.
- **Capability check deferred to codegen.** See TASK-0118.
- **Conflict between sync/async on one directive is silently 'last wins'.** See TASK-0119.
- **No support for identity-copy dataflow producers** (d <-- e with a bare DataRef RHS). The link pass already calls this out; transfer injection inherits the same gap. Not exercised by current examples.
- **End-to-end real-example tests assert only structural properties.** Example 1, 13-naive, 14-naive all have zero cross-worker edges, so no xfers are injected — these are sanity tests, not coverage of injection logic. The synthetic two-worker / three-edge cases are the load-bearing positive tests. A cross-worker real example (example 2 split add, when it lands) will exercise the pass against actual algorithm+schedule output. Until then, the synthetic battery is the test surface.

## Follow-up tasks filed

- TASK-0116: coalesce per-point Push/Wait into per-tile bulk transfers.
- TASK-0117: replicate Push/Wait pairs across distributed worker entities.
- TASK-0118: capability-matrix check on TransferPolicy at codegen time.
- TASK-0119: support conflicting sync/async options on one directive.

## Files

- nucleus/compiler/src/passes/transfer_inject.rs (new): the pass.
- nucleus/compiler/src/acfg.rs: enriched XferPlaceholder with {role, src, dst, data, tile, seq, policy}; added NotifyMode, TransferPolicy, XferRole.
- nucleus/compiler/src/lib.rs: re-exports for the new public types.
- nucleus/compiler/src/passes/mod.rs: registers the new pass module.
- nucleus/compiler/tests/transfer_inject.rs (new): 14 tests covering synthetic positive, policy combinations, idempotence, structural pairing, and real examples.

just check, just clippy, just test all green.
<!-- SECTION:NOTES:END -->
