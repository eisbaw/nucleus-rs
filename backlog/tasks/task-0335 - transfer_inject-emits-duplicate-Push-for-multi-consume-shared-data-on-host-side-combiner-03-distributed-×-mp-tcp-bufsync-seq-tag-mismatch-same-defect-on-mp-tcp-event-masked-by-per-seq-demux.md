---
id: TASK-0335
title: >-
  transfer_inject emits duplicate Push for multi-consume shared data on
  host-side combiner (03/distributed × mp-tcp-bufsync seq-tag mismatch; same
  defect on mp-tcp-event masked by per-seq demux)
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-25 22:00'
updated_date: '2026-05-25 22:19'
labels:
  - compiler
  - transfer_inject
  - multi-consume
  - wire-shape-masking
  - M6
  - forward-carried-from-TASK-0334
dependencies:
  - TASK-0334
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0334 cycle 157 empirical verification of 03-reduction/distributed × mp-tcp-bufsync surfaced a real transfer-injection defect that prior orchestrator narratives mis-attributed to TASK-0329 (host-excluding-barrier mediation).

## Empirical findings (cycle 157)

03-reduction/distributed × mp-tcp-bufsync: codegen succeeds (no ContractGap fires; barriers correctly include host). Runtime fails with:

```
wire: seq tag mismatch: receiver expected 4, wire delivered 8 — Push/Wait pairing diverged
```

Inspecting the emitted code:

**Host expects per-consumer Waits**:
- read_msg_expect(data_w0, seq=4) for partials[0..1] used by half1
- read_msg_expect(data_w1, seq=5) for partials[1..2] used by half1
- read_msg_expect(data_w2, seq=6) for partials[2..3] used by half2
- read_msg_expect(data_w3, seq=7) for partials[3..4] used by half2
- [compute half1 = combine(partials[0], partials[1])]
- read_msg_expect(data_w0, seq=8) for partials[0..1] again (because half1 reads partials[0])
- ... seqs 9, 10, 11 for the other workers ...

**Each worker emits TWO Pushes of partials back-to-back**:
```
wire::write_msg(&mut data_host, 8, &wire::enc_vec(&partials, ...));  // seq=8 FIRST
wire::write_msg(&mut data_host, 4, &wire::enc_vec(&partials, ...));  // seq=4 SECOND
```

Two defects compound:

1. **Duplicate Push emission**: transfer_inject emits ONE Push per host-side consume site. The algorithm has 4 such sites:
   - half1 = combine(partials[0], partials[1])  -> 2 consumes (one per partials index)
   - half2 = combine(partials[2], partials[3])  -> 2 consumes

   Even though each worker only writes its own partials[w] ONCE in phase 1, transfer_inject demands the producer push twice (once per host's consume of partials[w]).

2. **Producer-side ordering inversion**: the two Pushes on each worker fire in seq 8-then-4 order, but host's FIFO read sequence is 4-then-8 → wire FIFO seq mismatch at runtime.

## mp-tcp-event masks the same defect

03-reduction/distributed × mp-tcp-event also EMITS the duplicate Push (verified — w0 emits chan_8.push then chan_4.push), but the per-channel-per-seq demux topology (one chan per XferId) means the receiver matches by seq regardless of arrival order. **Output is bit-identical against reference.bin** despite the redundant data transmission. The defect is wire-shape-masked, NOT fixed at the pass level. Per the memory note 'project-mp-tcp-event-vs-bufsync-safety-profile'.

## Acceptance criteria

### AC#1: deduplicate Push emission per (data, producer) pair

When the same DataId is consumed at multiple sites on the same consumer worker, transfer_inject should emit ONE Push per (data, producer) — not one per (data, consumer-site). The single transfer carries the union of consume-side slices. Producer-side Push count drops from 2 to 1 per worker on 03/distributed.

### AC#2: 03/distributed × mp-tcp-bufsync promotion

Once AC#1 lands, promote 03-reduction/distributed × mp-tcp-bufsync from [[skip]] to [[required]]. Bit-identical against 03-reduction/reference.bin.

### AC#3: mp-tcp-event sibling verification

Confirm mp-tcp-event still produces bit-identical output after the dedupe (it should — fewer transfers, same data). Once cycle-157 lands the [[required]] promotion of 03/distributed × mp-tcp-event (currently masked-passing), this AC verifies it survives the dedupe.

### AC#4: regression test

Add a test that pins the per-(data,producer) Push dedupe on 03-reduction/distributed.

## Cross-reference

- TASK-0334 cycle 157: empirical-verification audit that discovered this defect class (the orchestrator-narrative-third-firing of misattribution to TASK-0329).
- feedback-orchestrator-narrative-also-wrong (memory) — fourth firing.
- project-mp-tcp-event-vs-bufsync-safety-profile (memory) — same wire-shape-masking pattern.
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs — the pass that emits per-consume-site Push.
- /tmp/task0334-codegen-03-reduction-distributed-mp-tcp-bufsync/src/bin/w0.rs (representative emitted code, cycle 157 reproducer).

## Honest scope

- MEDIUM priority. Real correctness defect on mp-tcp-bufsync (silent seq mismatch panic at runtime; not silent miscompile, but still hard failure).
- Same defect dormant-on mp-tcp-event due to wire-shape masking — promoting that cell exposes redundant bandwidth use but not incorrectness.
- Dependency: the dedupe must respect downstream consumer scopes (e.g. consumes across different Sequences may legitimately need separate transfers per sync_inject's barrier insertion). Requires careful design — not a trivial 5-line fix.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 157 architect P2.2 fold-back: AC#1 Sequence-scope tightening

The original AC#1 wording said 'dedupe Push emission per (data, producer) pair'. Honest-scope footnote acknowledged 'consumes across different Sequences may legitimately need separate transfers per sync_inject's barrier insertion'. Per architect P2.2 review of cycle 157: hoist the footnote INTO AC#1 to prevent scope-overshoot.

**Tightened AC#1**: dedupe Push emission per (data, producer, Sequence-scope) tuple. Multiple consume-sites WITHIN A SINGLE Sequence collapse to one Push; consume-sites in DIFFERENT Sequences keep separate Pushes (per sync_inject's barrier-insertion model).

This matches the historical split between `splice_pushes_for_waits` (transfer_inject.rs same-Sequence path, ~line 1232 in the cycle-157 tree) and `splice_pushes_global` (cross-Sequence path, ~line 1674). A future implementer should NOT collapse those paths — only dedupe within each.

The 03-reduction/distributed schedule has both `combine` calls inside the same top-level Sequence (the post-phase-1 host-side block), so the dedupe applies. If a future schedule placed half1 and half2 in DIFFERENT phases separated by a barrier, the dedupe would NOT apply across those two phases (each phase legitimately re-Pushes).
<!-- SECTION:NOTES:END -->
