---
id: TASK-0335
title: >-
  transfer_inject emits duplicate Push for multi-consume shared data on
  host-side combiner (03/distributed × mp-tcp-bufsync seq-tag mismatch; same
  defect on mp-tcp-event masked by per-seq demux)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 22:00'
updated_date: '2026-05-25 23:13'
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

## Cycle 158 implementation plan (orchestrator-direct)

Per project memory feedback-spawned-agents-refuse-code-edits + the structural-design depth here, orchestrator implements directly; parallel read-only review gate runs after.

### Root cause confirmed (cycle 158 regen)

Regenerated /tmp/task0335-bufsync from 03-reduction/distributed:
- host.rs lines 134-137: 4 reads of partials (seqs 4-7) for half1 = combine(partials[0], partials[1])
- host.rs lines 139-142: 4 MORE reads of partials (seqs 8-11) for half2 = combine(partials[2], partials[3])
- w0.rs lines 77-78: write_msg(seq=8) FIRST, then write_msg(seq=4) — producer-side ordering inversion compounds the duplicate-Push issue
  (splice_pushes_global splices at fixed producer index; later iterations insert ahead of earlier)

### Why the existing dedup misses

inject_in_sequence line 1075: `if !is_duplicate_xfer(out.last(), &w)` — only checks IMMEDIATELY PRECEDING element. The 2nd combine Op's Waits come AFTER the 1st combine Op in out; out.last() is the 1st Op, not a Wait, so dedup never fires.

### Fix plan

1. Add helper `is_duplicate_xfer_in_epoch(out: &[ACFGNode], cand: &XferPlaceholder) -> bool` that scans `out` from the end backward, returning true on the first matching (role,src,dst,data,tile) Xfer, and stopping at the first `ACFGNode::Sync` (sync_inject runs before transfer_inject; Syncs mark fresh rendezvous epochs where duplicate Waits are legitimate).
2. Replace the `is_duplicate_xfer(out.last(), &w)` call at line 1075 with the new in-epoch scan. Keeps tile-match semantics so partition-slice-differing Waits are NOT collapsed.
3. Update the long-form comment in splice_pushes_global lines 1685-1700: the warning about multi-consumer-Op deadlock from suppressing the 2nd Wait NO LONGER APPLIES because we now collapse at the Wait emit site BEFORE splice_pushes_global runs. After dedup, the surviving Wait's buffer place is filled by ONE Push and the subsequent consumer Op reads from the same per-data scratch (local Vec on host).
4. Update the dedup-sites docstring at lines 261-320 to reflect that the inject_in_sequence dedup is now epoch-scoped (Sync-bounded) instead of immediate-prev-only.
5. Add regression test `task0335_ac4_dedupes_multi_consume_in_same_sequence` in transfer_inject.rs: 2 consumer Ops on host in same Sequence, both reading a cross-worker symbol from 4 worker producers. Pre-fix: 8 Waits + 8 Pushes. Post-fix: 4 Waits + 4 Pushes.
6. Promote 03-reduction/distributed × mp-tcp-bufsync from [[skip]] to [[required]] in e2e-matrix.toml. Update the 03/distributed × {pthreads-sync, pthreads-async, mp-tcp-event} headers to reflect that TASK-0335 is resolved and bufsync is now part of the four-way differential.

### Risks / Honest-failure budget

- pthreads-sync + pthreads-async + mp-tcp-event currently pass 03/distributed; the dedup reduces their Wait/Push count too. If their codegen depends on per-Wait buffer state (e.g. a Vec<Buffer> indexed by seq), reducing the count should still leave the surviving seqs correctly indexed — but if a regression appears, the fix may need to be backend-aware (e.g. only dedupe when the Petri lowering supports multi-consumer-read).
- The first-firing Wait survives; later-firing Wait's seq is REMOVED. If anyone keys off the removed seq downstream, they'll panic loudly (no silent miscompile expected).
- The dedup is epoch-scoped — but the definition of 'epoch' (Sync-bounded vs sequence-bounded) is a design choice. Cycle 158 picks Sync-bounded because that matches sync_inject's barrier-insertion semantics. If a future shape needs sequence-bounded only, the helper has a tight scope.

### Gate plan

After implementation:
- nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e'
- Expect e2e: 112/98/0/14/0 → 112/99/0/13/0 (+1 promotion on 03/distributed × bufsync) — actually total stays 112 (cell promoted from skip to required), pass +1 → 99, skip -1 → 13.
- Verify non-flake × 2 by re-running just e2e.
- Spawn parallel qa-test-runner + mped-architect review (read-only).

## Cycle 158 implementation outcome (orchestrator-direct)

### Fix landed

`transfer_inject.rs` (nucleus-compiler/src/passes/transfer_inject.rs):
- Introduced helper `is_duplicate_xfer_in_epoch(out: &[ACFGNode], cand: &XferPlaceholder) -> bool` that scans `out` from the end backward on (role, src, dst, data, tile), stopping at the first `ACFGNode::Sync(_)` (sync_inject runs BEFORE transfer_inject; barriers mark fresh rendezvous epochs where duplicate Waits are legitimate).
- Replaced `is_duplicate_xfer(out.last(), &w)` at `inject_in_sequence`'s per-Op Wait-emission with `is_duplicate_xfer_in_epoch(&out, &w)`.
- Deleted the now-dead helper `is_duplicate_xfer` (sole caller above).
- Updated the module-level dedup-sites docstring to split the inject_in_sequence Wait dedup into two sub-bullets (hoisted-Waits-drain vs per-Op-emission). The cycle-158 site uses the helper whose role-check is `existing.role == cand.role` (no literal `XferRole::`), so it does NOT appear in the literal-pattern grep witness.
- Updated the long-form comment at `splice_pushes_global` explaining why seq-only dedup is still correct downstream: the legitimate multi-Wait cases (cross-epoch consumers, structurally-distinct tile slices) still survive the upstream Sequence-scope dedup; only the multi-consumer-Op SAME-tile duplicates are suppressed.

`tests/transfer_inject.rs`:
- Added `task0335_ac4_dedupes_multi_consume_in_same_sequence`: positive case asserting 4 Waits / 4 Pushes for the 03-reduction/distributed shape (2 host-side consumer Ops in same Sequence both reading `partials` from 4 worker producers). Pre-fix: 8/8.
- Added `task0335_ac4_sync_between_consumers_separates_epochs` (cycle-158 architect P2.3 fold-back): negative case with explicit Sync between consumer Ops asserting 8 Waits / 8 Pushes — pins the Sync-stopping arm of `is_duplicate_xfer_in_epoch`.

`nuc-nucleus/e2e-matrix.toml`:
- 03-reduction/distributed × mp-tcp-bufsync promoted [[skip]] → [[required]].
- Three narrative blocks rewritten (around 03/distributed cells for pthreads-sync, pthreads-async, mp-tcp-event) to reflect the four-way differential closure.

### Acceptance criteria

- **AC#1** dedupe Push per (data, producer, Sync-bounded epoch within Sequence): DONE — implemented as `is_duplicate_xfer_in_epoch`. NOTE on AC fidelity (cycle-158 architect P2.4 fold-back): the cycle-157 architect tightened AC#1 to 'Sequence-scope'; cycle-158 implementation is strictly NARROWER (Sync-bounded epoch within Sequence). The narrower scope is conservative-safe: when a Sync sits intra-Sequence, the second consumer's Wait is preserved so its buffer place is filled by the producer's epoch-2 Push. The negative test `task0335_ac4_sync_between_consumers_separates_epochs` pins this choice as load-bearing. See memory note `project-sync-epoch-vs-sequence-scope-dedup-key` for the design distinction.
- **AC#2** promote 03/distributed × mp-tcp-bufsync: DONE — e2e shifted 112/98/0/14/0 → 112/99/0/13/0 (verified non-flake × 2).
- **AC#3** mp-tcp-event sibling verification: DONE — still bit-identical post-dedup (transmits 4 Pushes per phase instead of 8; same correctness, half the bandwidth).
- **AC#4** regression test: DONE — `task0335_ac4_dedupes_multi_consume_in_same_sequence` + Sync-epoch companion.

### Gate (cycle 158)

- `just build`: clean (0 warnings after dead-helper delete).
- `just clippy`: 0 warnings.
- `just test`: every suite green.
- `just test-release`: 903 passed / 0 failed / 3 ignored (80 suites; cycle-157 baseline +2 from cycle-158 new tests).
- `just check-textual-replace-on-codegen` + `check-include-str-coverage`: OK.
- `just e2e`: **112/99/0/13/0** (was 112/98/0/14/0). Non-flake × 2.
- QA review gate (qa-test-runner read-only): **GO** with 1 P3 (line-stamp drift folded back in-thread).
- Architect review gate (mped-architect read-only): **GO** with 3 P2 (P2.1 + P2.2 filed as TASK-0335.01 / TASK-0335.02 follow-ups; P2.3 negative test folded back in-thread; P2.4 AC fidelity drift noted above + pinned in memory).

### Forward-carried lessons

- **Sync-epoch vs Sequence-scope** — memory note `project-sync-epoch-vs-sequence-scope-dedup-key` added. Future dedup sites in transfer_inject (or any post-sync_inject walker) MUST explicitly pick one and pin the choice with docstring + negative regression test.
- **Cycle-158 silent-sibling sweep was incomplete**: orchestrator found ZERO remaining `out.last()` literal call sites and declared closure, but architect P2.1 + P2.2 found TWO structurally-identical 'scan-only-narrow' sites (lines 1208-1218 and 1493-1503) using `out.iter().any(matches!(…))` WITHOUT Sync-stopping. Both are currently LATENT (Pass A receives epoch-collapsed input from the cycle-158 fix), filed as TASK-0335.01 + TASK-0335.02. Lesson: silent-sibling sweep must include semantically-equivalent patterns, not just the literal helper-name match.
- **Stamp-twice prediction**: cycle-158 line-stamp update at the dedup-sites docstring was off by exactly +14 lines (the narrative-prose addition shifted lines below the stamp). Architect P3.1 + QA P3 both caught it. Recurrence of `feedback-stamp-twice-when-narrative-content-shifts-line` — bumped one digit-only edit, stamp now verified post-edit (line count unchanged so single edit sufficed). Memory note continues to apply to ANY tracker md or source-file digit-only update made in the same edit-batch as narrative content.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 158 implementation by orchestrator (direct, per project memory feedback-spawned-agents-refuse-code-edits). transfer_inject's per-Op Wait-emission dedup widened from out.last()-only to a Sync-bounded epoch scan (is_duplicate_xfer_in_epoch). 03-reduction/distributed × mp-tcp-bufsync promoted to [[required]]; e2e 112/98/0/14/0 → 112/99/0/13/0 (non-flake × 2). Both review gates GO. Three follow-ups filed: TASK-0335.01 / TASK-0335.02 (silent-sibling investigation at hoisted-Waits-drain + place_or_bubble sites); memory project-sync-epoch-vs-sequence-scope-dedup-key recording the conservative-safe scope choice.
<!-- SECTION:FINAL_SUMMARY:END -->
