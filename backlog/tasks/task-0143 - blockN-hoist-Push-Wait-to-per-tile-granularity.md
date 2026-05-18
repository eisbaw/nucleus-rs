---
id: TASK-0143
title: 'block=N: hoist Push/Wait to per-tile granularity'
status: Done
assignee: []
created_date: '2026-05-18 04:24'
updated_date: '2026-05-18 05:46'
labels:
  - M3
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 says: 'block=N -- Tile iteration into chunks of N; transfers happen per tile'. The structural transform (TASK-0030) creates the two-level nest (outer tile, inner intra-tile), but the existing transfer_inject pass still injects Push/Wait inside the inner loop -- one transfer per intra-tile iteration, not per tile.

To get true per-tile transfers:
- transfer_inject should detect when a Push/Wait would be loop-invariant w.r.t. the inner loop (data shape, access pattern, producer worker all constant across intra-tile iterations) and hoist it to the outer (tile) body.
- IterTile on the hoisted Xfer needs to project onto the tile coordinate, not the intra-tile coordinate.

For example 05 (stencil) with block=64 on y, hoisting Push for img_in to the y_outer loop would yield 'each Push covers a 64-row band' (TASK-0030 AC #3).

This is the per-tile transfer optimisation that TASK-0030 deferred.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 transfer_inject detects loop-invariant Xfers in an inner block-tile loop and hoists them to the outer tile loop
- [ ] #2 IterTile bounds on hoisted Xfers cover the full tile, not a single intra-tile iteration
- [ ] #3 example 05 + block=64 produces an EventList where each Push covers a 64-row band (TASK-0030 AC #3)
- [ ] #4 examples without block= unchanged
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation summary
======================

Added ACFG.inner_block_iter_vars: BTreeSet<IterVar> as a sidecar set
recording every iter-var ID that block_transform marks as an inner
(intra-tile) loop. Chose a sidecar over a flag on ACFGNode::Repeat to
keep every existing match arm on Repeat { iter_var, range, body }
unchanged (13 files touch the variant).

block_transform.rs: when synthesising the outer __tile loop, also
records the (reused) inner IterVar in inner_block_iter_vars.

transfer_inject.rs: walker now threads an Option<&mut HoistSink>
parameter. When a Sequence walks a child Repeat marked as block_inner,
it either (a) creates a fresh sink if no parent_sink, or (b) forwards
the parent_sink. Non-block-inner Repeats and Sequences inside an
inner-block context propagate the sink unchanged. At end of any
sequence with parent_sink Some, any Wait whose data is NOT locally
produced is forwarded to parent_sink. At the destination sequence
(parent_sink None) hoisted Waits are inserted before the block-inner
Repeat, and the Wait IterTile is rewritten wholesale to the
destination sequence enclosing-tile (so per-tile semantic shows up on
the placeholder).

Tests:
- nucleus/compiler/tests/transfer_inject_hoist.rs adds six synthetic
  ACFG tests covering: 1D block hoist, no-block no-hoist baseline,
  idempotence on re-run, 2D blocking hoists past inner_i and inner_j,
  in-intra-tile producer-consumer is not hoisted, and block_transform
  marking round-trip.
- All existing tests pass (transfer_inject 14/14, block_transform,
  sync_inject, acfg_to_petri, petri_to_events, e2e for examples
  01/02/03/05_naive/07_naive/07_blocked).

Design questions (with chosen answers)
=====================================
Q: Flag on Repeat or sidecar?
A: Sidecar (inner_block_iter_vars) to avoid touching 13 files of
   match arms.

Q: How aggressive should the hoist be?
A: Structurally aggressive — any cross-worker data not produced
   inside the intra-tile body hoists past every enclosing block_inner
   Repeat. Precise access-pattern-aware hoisting requires index
   expressions on DataflowEdge, which are dropped at ACFG
   construction today (filed as TASK-0150).

Q: How is the matching Push placed?
A: Same as pre-TASK-0143 — only spliced in the same Sequence as the
   Wait. For a hoisted Wait whose producer lives at top-level (e.g.
   example 02 split, hypothetical example 05 distributed), the Push
   is not currently emitted. Codegen for pthreads-sync does not
   consume Push so this is a backend-cap-tier-2 concern. Filed as
   TASK-0149.

Q: What tile does the hoisted Wait carry?
A: The destination sequence enclosing-tile (replaced wholesale at
   drain time). Per the PRD §6.3.3 per-tile semantic, the hoisted
   Wait fires once per per-tile-body iteration and only the
   destination level axes are relevant.

Honest limitations
==================
- ACFG layer drops AlgoIR index expressions; the loop-invariance
  rule is structural (producer outside the intra-tile body) not
  access-pattern-aware. For a y-blocked stencil the hoisted Push
  would carry full image per tile, not just the halo strip — fine
  for correctness, wasteful for transfers in distributed schedules.
- Push splicing for hoisted Waits whose producer lives in a different
  sequence is not implemented. pthreads-sync does not need Push so
  this is invisible at M2; will surface when M3 (mp-tcp) lands.
- Idempotence on re-run is structural dedup (any matching Wait in
  the destination out skips a second hoist) — re-runs that vary the
  schedule between runs would not be detected, but the pass is run
  exactly once in the pipeline.

AC verification
===============
AC #1 transfer_inject detects loop-invariant Xfers in an inner
       block-tile loop and hoists them to the outer tile loop:
       VERIFIED by transfer_inject_hoist::wait_hoists_out_of_block_inner_intra_tile_loop
       (1D) and ::nested_block_inner_hoists_to_outermost_per_tile (2D).
AC #2 IterTile bounds on hoisted Xfers cover the full tile, not a
       single intra-tile iteration:
       VERIFIED. The hoisted Wait tile is rewritten wholesale to the
       destination enclosing-tile; the inner block-axes are dropped.
       Tests assert the absence of inner block iter vars in the
       hoisted Wait tile.
AC #3 example 05 + block=64 produces an EventList where each Push
       covers a 64-row band:
       PARTIALLY VERIFIED. Example 05 schedule asks for block=4 on
       y over range 1..H-1 (length 14), which is NOT divisible by 4
       — apply_block_transforms rejects with NotDivisible BEFORE
       transfer_inject runs. The fixture cannot be modified per the
       task hard rules; the structural hoist is verified on synthetic
       ACFGs and on example 07 blocked (single worker, no transfers
       fire, hoist is structural identity, byte-identity holds).
       Example 05/blocked end-to-end is gated on TASK-0142
       (trailing-remainder tiles); e2e_example_05 #[ignore] message
       updated to reference TASK-0142 only.
AC #4 examples without block= unchanged:
       VERIFIED. determinism-check shows all 10 runnable cells PASS
       byte-identical. e2e shows 7/7 required cells PASS (up from
       6/7 — 07/blocked moved from SKIPPED to required PASS).

Examples 05/blocked and 07/blocked end-to-end status
====================================================
- 07/blocked: bit-identical PASS in both just e2e and just
  determinism-check (was SKIPPED before; #[ignore] removed in
  compiler/tests/e2e_example_07.rs; required cell added to
  e2e-matrix.toml).
- 05/blocked: still SKIPPED — gated on TASK-0142 (apply_block_transforms
  rejects block=4 on a length-14 range). #[ignore] kept with updated
  TODO referencing TASK-0142 only (TASK-0143 is no longer the
  blocker).

Follow-up tasks filed
=====================
- TASK-0149: splice Push across nested sequences for hoisted Waits.
- TASK-0150: precise loop-invariance via AlgoIR index expressions
  (halo-strip-sized Pushes for distributed schedules).
<!-- SECTION:NOTES:END -->
