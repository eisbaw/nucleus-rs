---
id: TASK-0267
title: >-
  transfer_inject host-side Push synthesis drops for partitioned consumers under
  inner block transform OR async transfer
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 08:02'
updated_date: '2026-05-24 13:45'
labels:
  - M5
  - bug
  - compiler
  - transfer_inject
dependencies:
  - TASK-0266
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Spun out of TASK-0266 cycle-85 precise diagnosis (2026-05-24). When 05-stencil/distributed × pthreads-async is promoted from [[skip]] to [[required]], the generated binary hangs because the host fn main contains zero ring_*.push(img_in) calls. Workers w0..w3 block forever on w_i_ring_i.wait().

## Root cause (verified by Petri-net diff across partition policies)

Same algorithm + same place + same workers, ONLY the partition policy differs:
- partition=workers (WORKS): Petri net contains push_seq0..7 — 4 host-side Pushes for img_in + 4 worker Pushes for img_out. Host emit shows ring_0..3.push(img_in.clone()) at top level.
- partition=rows (BROKEN): Petri net contains push_seq4..7 ONLY — only the worker-side Pushes for img_out exist. The host-side Pushes for img_in (push_seq0..3) are ENTIRELY MISSING.

Additionally, cycle-85 ATTEMPT-2 showed that even with partition=workers (the working policy) the Push synthesis ALSO drops when EITHER of these is present:
- inner block transform on the partitioned axis: `loop x : block=64, vectorize=8, reuse;`
- async transfer mode: `transfer img_in : async, buffer=2, notify=event;`

Removing both restored host-side Push synthesis under partition=workers. Under partition=rows the host-side Push drops unconditionally.

## Probable bug site

transfer_inject (passes/transfer_inject.rs) — one of the following passes drops the host-side Push for partitioned consumers when (partition_rows) OR (partition_workers + block transform) OR (partition_workers + async transfer):

1. hoist_invariant_waits — for a 2D nest with partition on the OUTER axis, the inner (x) loop may not be transparent to invariance checking. Pass A may not recognise img_in as invariant w.r.t. the partition axis when the inner loop is a block-transformed strip.
2. splice_pushes_global / splice_pushes_for_waits — the cut decision (lowest common Repeat enclosing producer but not wait) may fail to place a Push at the correct level when the consumer Waits are nested inside a partition Repeat AND a block Repeat.
3. canonical-worker collapse + fan-out expansion (pre-TASK-0117) — the host-Push site may key off a check that recognises partition=workers but not partition=rows, OR that fails when the consumer side has a block Repeat in the path.

## Reproduction

- Schedule: nucleus/compiler/tests/fixtures/examples/05-stencil/distributed.sched.nuc
- partition=rows × any inner block × any transfer mode: deadlock, no host-side Push in emit.
- Swap to partition=workers + remove inner block + sync transfer: WORKS bit-identical.
- Swap to partition=workers + KEEP inner block OR async transfer: deadlock, no host-side Push.

## Probe recipe (next implementer)

Add eprintln! instrumentation to transfer_inject.rs at:
- build_waits_for_op exit (line ~2089): print the Vec<XferPlaceholder> waits emitted.
- splice_pushes_for_waits exit (line ~963): print inserts performed.
- splice_pushes_global enter+exit (lines ~1461 / ~1591): print waits collected and pushes spliced.
- inject_transfers exit (line ~411): print final ACFG node count + Xfer count by role + Xfer roles by src/dst.

Run with partition=rows AND partition=workers + with/without inner block + sync/async transfer. The ONE pass whose output diverges is the bug site.

## Acceptance criteria

1. Failing fixture in nucleus/compiler/tests/ that pins the bug as a typed transfer_inject ContractGap (or a positive test asserting host-side Push presence in the lowered ACFG for partitioned consumers + inner block + async transfer combos).
2. Fix lands so the Petri net for 05-stencil/distributed × pthreads-async (with partition=workers AND inner block AND async transfer) contains the host-side Push events.
3. e2e cell 05-stencil/distributed × pthreads-async promotes from [[skip]] to [[required]] AND PASSES bit-identical to reference.bin UNDER partition=workers + inner block + async transfer.
4. partition=rows variant ALSO passes (closes the second arm of the bug; if partition=rows-specific extension is non-trivial beyond the block/async fix, allowed to defer to TASK-0258.bis with a precise note here).
5. Regression test: a multi-policy + multi-block + multi-transfer-mode fixture so a future regression in any direction surfaces.

## Honest limits / scope

- DO NOT fix BUG 2 (per-iteration barrier deadlock) in this task — it's separate (TASK-0268). The two interact: even after this fix, partition_rows × unequal-iter still deadlocks. The two must BOTH land for M5 AC#4 to close.
- Allowed scope: transfer_inject (passes/transfer_inject.rs) + closely-related sidecar/ACFG plumbing. NOT sync_inject; NOT backend codegen.

## Dependencies

- Prerequisite of: TASK-0266 (M5 AC#4 closure umbrella).
- Sibling: TASK-0268 (BUG 2 barrier-deadlock).

## Forward-carry context

Memory entries that bear:
- project_partition-silent-drop (partition_rows = TASK-0258 cycle 79c; transfer_inject was never updated for partition_rows consumers).
- project_negative-seam-and-backend-layout (transfer_inject pass landscape, post-fan-out shape).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle 101 (2026-05-24, orchestrator-direct) ===

LANDED in commit 336836f (+ review-hardening to follow).

Root cause: the TASK-0151 `contains_block_inner` opacity gate in
Pass A `hoist_invariant_waits` and Pass B `collect_waits` /
`splice_pushes_global` stranded the host-side Push for img_in.
With the gate removed, Pass A's existing per-Wait stay-vs-bubble
classification (keyed on `produced_data_set` of the enclosing
Repeat's subtree) subsumes the gate semantically: data produced
outside ⇒ bubble + whole-symbol Push at producer's scope; data
produced inside ⇒ stay for per-iteration rendezvous. The
per-worker partition slice is added later by
`rewrite_partition_tiles` (TASK-0117) and halo widths by
`extend_xfer_tiles_for_halo` (TASK-0263), so the Wait still
carries the right slice regardless of which Repeat it crossed.

Probe results (with the fix applied):
- 05-stencil/distributed × pthreads-async emits the 4 host-side
  ring_<N>.push(img_in.clone()) lines in main() (lines 174-177 of
  the generated main.rs).
- Petri net contains push_seq0..7 (4 host-side for img_in + 4
  worker-side for img_out). Pre-fix only push_seq4..7 existed.
- TRACE-level confirmation: `NUC_TRACE=1` no longer emits the
  cross-scope-deferral lines for img_in (the entire deferral
  facility was removed; see TASK-0280 follow-up).

Cycle-85 hypothesis correction: the diagnosis claimed "async
ALSO triggers" the bug, separate from inner block. Cycle-101
probes proved this WRONG — async transfer alone (no inner block)
DOES emit host-Pushes correctly (probed at /tmp/probe-0267-noblock
with `block=` removed but async kept). The bug was solely the
contains_block_inner opacity gate. Single-axis fix.

AC status:
- AC#1 (failing fixture pinning the bug): MET. Test
  `block_nested_in_plain_loop_pairs_the_invariant_wait` flipped
  to assert the FIXED behaviour; the synthetic shape exercises
  exactly the contains_block_inner trigger condition.
- AC#2 (fix lands so the Petri net contains host-side Push
  events): MET. Verified by direct inspection at
  /tmp/probe-0267-fixed/src/main.rs.
- AC#3 (e2e cell promotes to [[required]] bit-identical):
  BLOCKED-NOT-FAILED on TASK-0268 (BUG 2 — sync_inject barrier
  deadlock under unequal partition counts).
- AC#4 (partition=rows variant also passes): BLOCKED-NOT-FAILED
  on TASK-0268.
- AC#5 (regression test for multi-policy + multi-block +
  multi-transfer-mode): MET via two pins:
  * Synthetic: `wait_hoists_out_of_block_inner_intra_tile_loop`
    (top-level hoist + Push pinning) +
    `block_nested_in_plain_loop_pairs_the_invariant_wait` (mixed
    block/non-block shape) +
    `mixed_block_and_nonblock_program_pairs_the_nonblock_transfer`
    (both d AND e arms pinned post-fix).
  * Real pipeline: `distributed_pthreads_async_host_pushes_img_in_to_every_worker`
    in nucleus-compiler/tests/e2e_example_05.rs builds the real
    05-stencil/distributed × pthreads-async via the driver and
    asserts the count of distinct `ring_<N>.push(img_in.clone())`
    matches in the host main() (regex-style match, robust to
    SeqTag reordering — cycle-101 review hardening for P2.2).

Side-effect of removing the gate: the
nucleus_compiler::trace::nuc_trace! macro now has zero in-source
callers. Filed as TASK-0280 (decide: keep / remove / re-purpose).

Side-effect audit: three stale e2e-matrix.toml skip reasons
cleaned up in the same cycle (all citing TASK-0117 + halo
synthesis which both landed long ago):
- 05-stencil/distributed × pthreads-sync → TASK-0042
  capability mismatch (async/buffer/notify=event).
- 05-stencil/distributed × mp-tcp-bufsync → same as
  pthreads-sync.
- 05-stencil/distributed × mp-tcp-event → TASK-0175
  (host-excluding barrier), corrected from the stale
  TASK-0181 (block_tag malformed projection — which TASK-0267
  itself closed by removing the upstream Wait-without-Push
  artifact).

Review gate: parallel read-only qa-test-runner + mped-architect
returned GO + GO.
- qa-test-runner: cargo test --workspace 804/0/3, clippy
  clean, just e2e 92/77/0/15/0 byte-stable over 2 runs, both
  negative gates bite (NUC_NONDET_PERTURBED_CELLS=77,
  NUC_XBACKEND_CORRUPTED_DETECTED=1), pin test green.
- mped-architect: P0/P1 = none. Four P2 forward-carry items
  applied in-thread (pin regex robustness, three stale skip
  reasons, NUC_TRACE follow-up filed as TASK-0280).

Forward-carried lessons (for TASK-0268 + future implementers):
- The `contains_block_inner` opacity gate predates TASK-0263's
  halo-aware tile rewrite. When BOTH machineries co-exist (the
  per-tile slice info IS available via partition_worker_ranges
  + halo_widths), opacity gates become redundant. Check if any
  similar opacity gate in sync_inject / petri_to_events /
  partition_* could be similarly subsumed by now.
- Probe methodology: a Petri-net diff between two variants of
  the same schedule (e.g., partition=workers vs partition=rows
  on otherwise-identical algorithm) is a powerful diagnostic
  tool — it isolates the specific transition that's missing.
  See cycle-85 notes in TASK-0266 for the exact recipe.
- The NUC_TRACE=1 env-gated trace facility is silent on the
  default path but lights up at deferral/skip decision sites.
  When a behaviour change involves removing a deferral
  facility, the trace coverage is a useful diagnostic before/
  after probe.
<!-- SECTION:NOTES:END -->
