---
id: TASK-0290
title: >-
  M5 Stage 3 follow-up: bit-identical e2e cell for halo-strip synthesis under
  partition=blocks2d (TASK-0289 AC#2 + AC#4)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-24 20:24'
updated_date: '2026-05-24 23:42'
labels:
  - M5
  - compiler
  - halo
  - partition
  - stage-3
  - e2e
  - forward-carried-from-TASK-0289
dependencies:
  - TASK-0289
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0289 splits naturally into two sub-cycles. Cycle A (this task's parent, TASK-0289) lands the halo-strip Push/Wait synthesis (AC#1) + verifies the existing matrix stays green (AC#3). Cycle B (this task) lands the FIRST e2e fixture exercising the new synthesis bit-identical against a hand-written reference oracle, and bumps the matrix baseline.

## Acceptance criteria

1. New schedule on 05-stencil (or a new example) with `partition=blocks2d` on a 2D-divisible image — likely H=16 W=16 stays compatible (inner ranges 1..15 length-14, NOT divisible by 2 cleanly) so a NEW example or a modified H/W is the right move. Pick consciously and write the rationale into the task notes.
2. Hand-written reference oracle (`reference/` Rust crate) that produces the bit-identical expected output for the new image dimensions + 2x2-grid distribution.
3. New `[[required]]` cell in `nuc-nucleus/e2e-matrix.toml`: pthreads-async × the new schedule. Optional: pthreads-sync (single-worker arm via the shared single-worker renderer would be byte-identical iff the schedule's distributed placement reduces to single-worker — likely NOT the case for blocks2d). Document the SKIP rationale for backends that cannot lower this cell (mp-tcp-bufsync, mp-tcp-event likely SKIP on the same w↔w-mesh basis as today's 05-stencil/distributed — TASK-0175 lineage).
4. e2e baseline bumps to at least 93/80/0/13/0 (the new cell adds +1 total + +1 pass).
5. just determinism-check stays green on the new cell.

## Dependencies

- TASK-0289 cycle A (synthesis pass landed) — this cycle exercises it end-to-end.
- TASK-0260 (halo inference) — needed for halo_widths populated on the new schedule's stencil access pattern.
- TASK-0263 (transfer_inject halo extension) — composes with the cycle-A synthesis.

## Honest scope

- Cleaner if 05-stencil's H/W changes to 16x16-with-padding or a new example variant lives at `examples/05b-stencil-2d-grid` rather than modifying 05-stencil's image (which would change every existing cell's reference.bin — a much bigger blast radius).
- Cycle A's synthesis pass will likely have only a unit/integration test on synthetic ACFG by the time this cycle starts. This cycle is the FIRST true bit-identical confidence on the new machinery.

## Forward-carried from TASK-0289 cycle A scope-split

Filed at TASK-0289 implementer briefing time to bound cycle A's scope. The parent's task brief already prescribes the design picks for AC#1; this task only needs to drive them through a real fixture.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Scope this cycle (cycle 114b): placement fix + single-pass distributed-2d e2e cell

This cycle does TWO things tightly coupled (each forces verification of the other):

1. **Placement fix** for `prepend_strip_pairs` (architect P1 from cycle 114a). Option (i) from the forward-carried notes: walk the parent Sequence's children, find the index of the producing `Operation` for the halo data symbol, insert the synthesised pairs AFTER that Operation (before the outer Repeat).
2. **New e2e cell**: `05-stencil/distributed-2d.sched.nuc` with `partition=blocks2d` on the outer y-loop, 4 workers in a 2x2 grid. Reuses the existing `05-stencil/reference.bin` (bit-identical per forward-carried lesson #5 — a single-pass stencil with host-broadcast img_in produces the same output whether halo strips fire or not, because each worker already has the full image via the existing `extend_xfer_tiles_for_halo` path).

The meaningful-halo / multi-pass / time-step stencil (where the halo strips carry unique semantic value) is **SPLIT to a new follow-up TASK-0294** — that requires a new example or substantially modified 05-stencil with a time-step loop + the multi-pass placement extension (forward-carried lesson #2).

## Per-AC mapping

- **AC#1** (synthesis pass + N/S/E/W neighbour resolution + corners excluded): **already DONE** in cycle 114a.
- **AC#2** (bit-identical e2e cell + reference oracle): **THIS CYCLE** via the distributed-2d schedule. Reference oracle = the existing 05-stencil/reference.bin (single-pass equivalence per forward-carried lesson #5; THE REFERENCE IS THE SAME — only the partition shape differs, the algorithm output is identical).
- **AC#3** (existing matrix green): **THIS CYCLE** must hold across placement fix.
- **AC#4** (baseline bump): **THIS CYCLE** — 92/79/0/13/0 → 93/80/0/13/0.

## Work breakdown

### Step 1 — placement fix

Edit `prepend_strip_pairs` in `nucleus/nucleus-compiler/src/passes/transfer_inject.rs` (currently lines ~2480-2537). Instead of prepending the synthesised `Xfer` placeholders BEFORE any direct-child Repeat whose iter_var matches, INSERT them AFTER the FIRST direct-child Operation whose `dataflow.edges` produces the halo data symbol. The data symbol is known per-strip (each synthesised XferPlaceholder carries `data: DataId`).

Algorithmic shape:
- For each direct-child position, scan for Operations producing each halo data symbol in the group.
- For each halo data symbol, find the LAST producer index in the children (so we insert AFTER ALL producers).
- Group the synthesised pairs by their target insert-position, then splice them in.

Edge case: if NO producer is found in the immediate parent Sequence, fall back to today's prepend behaviour (with a comment explaining). This handles synthetic test fixtures where the data is implicitly provided.

### Step 2 — update existing unit tests

The unit tests in `tests/halo_strip_synth.rs` use a synthetic ACFG with NO producer Operation (cycle-114a-hardening removed the dead `load_op`). They WILL fall through the fallback prepend path. Add a NEW unit test `positive_placement_after_producing_op` that builds a fixture WITH a producer Operation and asserts the synthesised pairs land AFTER it.

### Step 3 — new schedule + matrix wiring

Create `nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc`:

```
schedule for "../prog.algo.nuc" {
    workers = { host, w0, w1, w2, w3 };
    place load_image on host;
    place save_image on host;
    place blur3      on { w0, w1, w2, w3 };

    // 2x2 grid. y range 1..H-1 = 1..15, divisibility:
    // decompose_grid(4) = (2, 2); 14 % 2 == 0 on both axes ⇒
    // each worker owns a 7x7 cell.
    loop y : partition=blocks2d;

    transfer img_in  : sync;
    transfer img_out : sync;
}
```

Wire into `nuc-nucleus/e2e-matrix.toml` as a `[[required]]` cell on `pthreads-async` (M5). `pthreads-sync` is optional informational. `mp-tcp-bufsync` + `mp-tcp-event` SKIP on the same w↔w-mesh lineage as today's 05-stencil/distributed (TASK-0175). Document SKIP reasons inline.

### Step 4 — verify bit-identical against reference.bin

The new cell must produce output byte-identical to `05-stencil/reference.bin`. If it doesn't, that's either a placement-fix bug, a synthesis bug, or a real semantic gap — diagnose, don't ship.

### Step 5 — gate + tracker + memory

- `just ci` green (92 → 93 total; 79 → 80 pass).
- Per-AC notes on TASK-0290 + final summary.
- File TASK-0294 for the multi-pass / meaningful-halo cell.
- TASK-0289 can finally close (AC#1+3 cycle 114a + AC#2+4 this cycle).

## Honest limits

- The new cell is "structurally passes" rather than "semantically meaningful" — the halo strips carry data the host broadcast already provides, so the bit-identical reference is degenerate. THIS IS DOCUMENTED in commit + matrix comment + TASK-0294's brief. The cell still proves: (a) partition=blocks2d lowers end-to-end, (b) synthesis doesn't BREAK existing behaviour, (c) the placement fix is correct.
- The idempotence break (cycle 114a forward-carried lesson #4) is NOT addressed this cycle. Single inject_transfers call per build = unobservable in practice. Defer to TASK-0294 or beyond.
- The multi-pass placement extension (forward-carried lesson #2) is NOT addressed this cycle. Deferred to TASK-0294 which needs it for meaningful-halo testing.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Forward-carried from TASK-0289 cycle 114a (2026-05-24)

These are the gotchas + design constraints the TASK-0290 implementer needs to know before wiring up the bit-identical e2e cell.

### What's already implemented (cycle 114a — commit f8d58ea)

`inject_halo_strip_xfers` lives at the tail of `inject_transfers`'s finalisation chain in nucleus-compiler/src/passes/transfer_inject.rs. It:
- Short-circuits on empty partition_pairs (AC#3 additive-only guard — every shipped schedule today has empty pairs).
- For each (outer_iv, inner_iv) pair, walks consumers map + halo_widths to find data symbols needing strip transfers, computes per-worker (row, col) inversion using grid_shape_for_outer_iv, and synthesises Push+Wait pairs for N/S/E/W neighbours (corners excluded per task brief).
- Tile shape: N/S strips are [(outer_iv, y_lo-h..y_lo or y_hi..y_hi+h), (inner_iv, x_lo..x_hi)]; W/E strips swap axes.
- Both endpoints pre-paired with shared SeqTag from the global state counter (NOT routed through splice_pushes_global).
- Pairs PREPENDED to the parent Sequence containing the outer Repeat — for a single-pass stencil this lands them at top-level before load_op.

### Unit tests (tests/halo_strip_synth.rs — landed cycle 114a)
- positive_3x3_halo_1_per_worker_pair_counts
- positive_3x3_halo_1_center_worker_w5_pair_shapes (exact strip tiles for N/S/W/E)
- positive_2x2_halo_1_corner_pair_shapes
- empty_partition_pairs_emits_zero_halo_strip_xfers (AC#3 contract)
- halo_strip_synthesis_is_deterministic_across_runs

### Gotchas the TASK-0290 implementer needs to know

1. **05-stencil image dimension is 16x16**, inner loops are 1..H-1 = 1..15 (length 14) on each axis. partition=blocks2d on this NEEDS a grid (R, C) such that 14 % R == 0 AND 14 % C == 0 AND R*C == num_workers. 14 factors as 1*14, 2*7. With 4 workers, decompose_grid returns (2, 2) — fails NonDivisible because 14 % 2 == 0 actually works. Wait — 14/2 = 7, that's fine. So 4 workers should partition 1..15 into rows 1..8, 8..15 — that's 7 each. Actually y_lo=1, y_hi=15, slice=(15-1)/2=7, so worker 1 owns y=1..8, worker 2 owns y=8..15. Confirmed divisible.

   **HOWEVER**: a 2x2 grid means halo_y = 1 and the strip neighbours include the BOUNDARY rows (1..H-1 covers the interior; the worker reading [1..8] needs halo at row 0 from outside the loop range). The clamp in extend_xfer_tiles_for_halo allows ranges to extend by halo beyond the source loop range, so this should compose. Verify on the actual fixture.

2. **Multi-pass / time-step stencils need different placement.** Today's placement (prepend to parent Sequence of outer Repeat) puts the synth pairs at the same level as the partitioned loop. For a single-pass stencil this fires the strip transfers ONCE before the work loop — which is correct given img_in is loaded once by host and the strip is read-only.

   If TASK-0290 adds a multi-pass / time-step stencil (e.g., iterative blur), the strip transfers need to fire ONCE PER TIMESTEP — meaning placement must be 'inside the timestep Repeat body, before the partitioned outer Repeat'. The current implementation does NOT do this. Adding a multi-pass example requires extending prepend_strip_pairs to walk for the enclosing time-step Repeat and prepend inside its body.

3. **IDEMPOTENCE IS BROKEN** when partition_pairs is non-empty. Two compounding issues:
   (a) rewrite_partition_tiles clobbers halo-strip tiles on a re-run (its compute-worker rule replaces strip with src's full partition slice when both endpoints are partitioned).
   (b) splice_pushes_for_waits (inside inject_in_sequence at the root Sequence) splices a new Push for every halo-strip Wait it sees in the root sequence, because the existing Pushes from the first pass sit BEFORE load_op (outside the immediate-successor dedupe window at transfer_inject.rs line ~990).
   
   This does NOT affect the e2e gate (driver pipeline calls inject_transfers once). But if TASK-0290 wants to re-pin full structural idempotence, the fix is one of:
   (i) make rewrite_partition_tiles SKIP Xfers where both endpoints are partitioned workers (the halo-strip signature). Verify against the pre-existing transfer_fanout_composes_with_partition_sidecar test to make sure no fan-out shape relies on the dual-partition rewrite path.
   (ii) widen splice_pushes_for_waits's dedupe to scan the full sequence (not just the slot immediately after the producer).
   (iii) tag synthesised halo-strip Xfers structurally (e.g., a new XferPlaceholder field 'is_halo_strip: bool') so rewrite + extend + splice can pass them through.
   
   (iii) is cleanest semantically but touches the data model. (i) + (ii) is the smallest blast radius and is what I'd recommend trying first.

4. **Backend support**: mp-tcp-bufsync + mp-tcp-event will LIKELY SKIP the new cell on the same w↔w-mesh basis as today's 05-stencil/distributed (TASK-0175 lineage — neither backend can lower a host-excluding barrier or a w-to-w transfer that's not via the host star). pthreads-sync + pthreads-async are the bit-identical targets.

5. **Reference oracle for 2D-blocks2d stencil**: needs to mirror the 2D-grid partition assignment EXACTLY (BTreeSet numeric WorkerId order ⇒ row-major (row=i/cols, col=i%cols) assignment). For a 2x2 grid on 4 workers, w1=(0,0), w2=(0,1), w3=(1,0), w4=(1,1). The halo exchange between neighbours is structurally redundant FOR A SINGLE-PASS stencil with host-broadcast img_in (each worker already gets the full image with halo extension via the existing extend_xfer_tiles_for_halo path). So the bit-identical reference output for a SINGLE-PASS stencil should match today's 05-stencil/distributed reference.bin EXACTLY — the new halo-strip transfers carry the same data the existing host broadcast already provides. This is a degenerate case for AC#2.

   To make the e2e cell MEANINGFUL (i.e., a case where the halo-strip synthesis carries unique semantic value), TASK-0290 should consider a MULTI-PASS stencil where each worker owns its tile of img_buf in-place across timesteps. That's the natural shape that needs cross-worker halo exchange. But that requires items #2 (multi-pass placement) AND a new example (or a substantially modified 05-stencil with a time-step loop).

   Alternative honest move: ship the SINGLE-PASS bit-identical cell as AC#2/AC#4 confirmation that the synthesis at least DOESN'T BREAK output, even if it doesn't add new semantic value yet. File the multi-pass / true-halo-test as a separate follow-up.

6. **Composition with partition_workers / partition_rows**: partition_blocks2d's writer writes BOTH outer_iv and inner_iv entries into partition_worker_ranges. If a schedule combines partition=blocks2d on one nest with partition=workers/rows on a sibling, the outer/inner key sets are disjoint by grammar construction (at most one partition= per loop) — already pinned by partition_blocks2d::composition_does_not_trample_prior_partition_entries. The TASK-0290 fixture should NOT need to test compositions; a single blocks2d directive is sufficient.

### Cross-reference for the implementer
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs::inject_halo_strip_xfers + prepend_strip_pairs (the synthesis)
- nucleus/nucleus-compiler/tests/halo_strip_synth.rs (the unit tests + AC#3 contract pin)
- nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs (the sidecar writer; row-major assignment)
- nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc (the closest existing schedule shape)
- nuc-nucleus/e2e-matrix.toml (where the new cell will be wired)

=== TASK-0289 cycle 114a review-hardening (orchestrator-applied) — REFINEMENT of forward-carried lesson #2 ===

The cycle-114a implementer flagged "placement = parent Sequence of outer Repeat" as a problem ONLY for multi-pass / time-step stencils. Read-only architect review of cycle-114a (architect agent) corrected this: the placement defect applies to SINGLE-PASS stencils too.

Concrete restatement:
- `prepend_strip_pairs` inserts halo-strip Push/Wait pairs at the FRONT of the root Sequence — i.e. BEFORE any `load_image` Operation lowered to the same root Sequence.
- On the receiving worker, the synthesised `Wait` is scheduled before the host's `load_image` has fired → the neighbour worker's matching `Push` references data that has not yet been produced on the host. Order-of-emit in the EventList per worker comes from the source Sequence order, so this is a real ordering defect, not just an aesthetic concern.
- For 05-stencil/distributed-2d (TASK-0290 AC#2), the bit-identical reference will diverge from the synthesised output the FIRST time the worker reads the halo strip — unless the synthesis is moved to land AFTER the producing Operation.

Fix candidates for TASK-0290's cycle, ranked:
- (i) place halo-strip pairs AFTER the producing Operation in the root Sequence — walk the children of the root Sequence to find the index of the first `Operation` that produces the halo data symbol; insert the Xfers between that Operation and the outer Repeat.
- (ii) alternative: place inside the outer Repeat body's pre-children prefix (the "before-the-inner-Repeat" position). Cleaner for multi-pass schedules but requires deciding whether the halo strip fires per-outer-iteration (correct for time-step) or once at top-level (correct for single-pass).
- (iii) richer alternative: emit a synthetic dataflow edge so transfer_inject's existing splice machinery places the Push/Wait pair via the same path as host->worker fan-outs; this would inherit hoisting, partition-tile-rewriting, and pipeline-depth annotation for free, and is the architecturally cleanest path but requires more plumbing.

Also REFINEMENT of forward-carried lesson #4 (idempotence break):
The cycle-114a hardening also confirmed the architect's earlier static trace was correct: the `load_op` in the cycle-114a test fixture (`tests/halo_strip_synth.rs`) was UNNECESSARY for any panic the implementer described. It has been REMOVED in the hardening commit; the fixture is now minimal. The empirical check pinning this (running the 5 tests without `load_op` and observing all green) is also documented in-line in the test file.

Forward-carried tactical note: the hoist-escape panic site in `transfer_inject.rs` (~line 358, ~line 1451) IS still a real panic-on-valid-input risk for any FUTURE schedule where the halo data symbol has no top-level producing Operation; that risk should be ruled out (or converted to a typed error) when TASK-0290's e2e cell lands.

## Cycle 114b implementer notes (2026-05-25)

### Outcome summary
- AC#1 (synthesis pass): DONE in cycle 114a (inherited; verified green via halo_strip_synth's 5 existing tests).
- AC#2 (bit-identical e2e cell + reference oracle): **NOT achieved as bit-identical PASS**; cell is SKIP'd on TASK-0294 prerequisite. Schedule + matrix wiring landed.
- AC#3 (existing matrix green): DONE — e2e baseline 92/79/0/13/0 -> 96/79/0/17/0 (+4 total, all 4 are new SKIPs; zero pre-existing cells regressed).
- AC#4 (baseline bump to 93/80/0/13/0): **NOT achieved**; baseline is 96/79/0/17/0 instead. The +1 to total/pass that AC#4 anticipated did not materialize because the new cell SKIPs on TASK-0294 (semantic codegen gap discovered during this cycle).

### What WAS achieved structurally
- Placement fix for prepend_strip_pairs landed (architect P1 from cycle 114a): synthesised halo-strip Push/Wait pairs now place AFTER the last producing Operation in the parent Sequence, not at index 0. Fallback to index 0 preserved for synthetic fixtures with no top-level producer. Unit test positive_placement_after_producing_op pins the new behaviour; 5 existing halo_strip_synth tests stay green via the fallback path.
- New schedule file nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc with partition=blocks2d on a 2x2 grid landed.
- New matrix entries: pthreads-async SKIP'd on TASK-0294; pthreads-sync + mp-tcp-bufsync SKIP'd on capability (same as row-band sibling); mp-tcp-event SKIP'd on TASK-0175+0294.

### What FAILED bit-identical and why (honest diagnosis)
Running the cell in isolation reveals real semantic divergence at byte 68 (element 17 = row 1, col 1 = first compute pixel). Root cause: backend-common's leading_axis_slice (HONEST-PARTIAL ASSUMPTION from TASK-0117) ignores the inner axis of 2D partition tiles. Each worker's per-tile transfer for img_out has bounds [(y, y_lo..y_hi), (x, x_lo..x_hi)] but the host gather lowers as a 1D y-band slice — worker w0 (y=1..8, x=1..8) pastes its WHOLE row 1..8 buffer (including unwritten x=8..15 columns of default-zero) over the image, then w1 (y=1..8, x=8..15) overwrites with its WHOLE row 1..8 buffer (including default-zero x=0..8). Net: each worker's actual rectangle gets clobbered. Forward-carried lesson #5 (single-pass equivalence) was correct in SEMANTICS but wrong about CODEGEN — a real 2D slice-paste codegen path is needed.

### Prerequisite filed
TASK-0294 'extend leading_axis_slice (or design 2D slice-paste) to handle partition=blocks2d's 2D per-worker tiles' captures the root cause + diagnostic evidence + AC's needed to promote the cell.

### Gate numbers (this run)
- just build: OK
- just clippy: OK (zero warnings under -D warnings)
- just test (dev): all green; 6/6 halo_strip_synth tests including the new positive_placement_after_producing_op.
- just test-release: all green.
- just check-textual-replace-on-codegen: OK.
- just check-include-str-coverage: OK.
- just e2e: 96/79/0/17/0 (was 92/79/0/13/0; +4 total +4 SKIPs from the 4 new entries).
- determinism check: 96/79/0/17 green.

### Honest gotchas / forward-carried to TASK-0294 (or the next cycle)
1. The placement fix is correct AND verified, BUT in this cycle it only fires in the new SKIP'd cell — its e2e benefit is only realised once TASK-0294 unblocks the bit-identical promotion. The existing 05-stencil/distributed cell (partition=rows) has empty partition_pairs so the placement fix never fires there.
2. The placement fix walks all DataflowEdges to find producers (not just edges[0]); this generalises beyond output_data()'s 'first edge' helper. Documented in the doc comment.
3. The matrix entry for distributed-2d × mp-tcp-event cites BOTH TASK-0175 and TASK-0294 — the w↔w mesh AND the 2D slice-paste are both required for promotion. The 2D codegen comes first chronologically (you can run it on pthreads-async with TASK-0294 alone; mp-tcp-event needs both).
4. The cycle 114a re-run / idempotence break (forward-carried lesson #4) is NOT addressed this cycle. Single-call-per-build means it's still unobservable in production.
5. Multi-pass / time-step stencil for true meaningful-halo testing is NOT filed in this cycle — leave to the orchestrator to decide whether to file as a separate task or fold into TASK-0294's follow-up.

### Closure status
TASK-0290 stays In Progress pending TASK-0294. AC#1+#3 closed; AC#2+#4 blocked on TASK-0294. TASK-0289 umbrella stays In Progress for the same reason.

### Architect-review-mark
Per the architect's cycle 114a forward-carried review note: the placement defect was real (sibling worker reads halo-strip data before host has produced it) AND now verifiably fixed. The new cell's emitted main.rs (target/e2e-matrix scratch) shows the host's load_image() call BEFORE the host's ring_*.push(img_in.clone()) broadcasts to the workers — the correct producer-then-broadcast order. Pre-fix this would have been reversed.
<!-- SECTION:NOTES:END -->
