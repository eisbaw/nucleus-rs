---
id: TASK-0343
title: >-
  Cross-worker array-output accumulator: host combine is last-write-wins instead
  of element-wise sum (TASK-0044.04 cycle-186 AC#3 surface)
status: To Do
assignee: []
created_date: '2026-05-26 16:59'
labels:
  - compiler-bug
  - M6
  - codegen
  - accumulator
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Filed cycle 186 in response to TASK-0044.04 AC#3 empirical probe: distributed schedule on 08-histogram lowers cleanly across all 4 tier-1 backends but produces output.bin = [4,4,4,4,...,4,4] (sum=64) instead of the reference [25,25,24,24,24,23,23,9,9,10,10,10,10,10,10,10] (sum=256). The output is the last worker's STANDALONE partial histogram; the host combine code emits 'histogram = slot_N.wait()' for each N in 0..NUM_WORKERS sequentially — last write wins.

Algorithm-level diagnosis: histogram[b] is an accumulator over (i, b) where i is the partition variable but b is independent. Every worker w writes histogram[0..BINS] independently; the cross-worker fan-in must SUM element-wise per bin. Contrast 03-reduction's partials[w] — there w IS the partition variable, each worker owns one slot, and cross-worker combine is just concatenation (each slot filled by exactly one worker).

This is a substantively new ACFG / codegen shape vs the disjoint-write reductions in 03-reduction. The fix likely sits in transfer_inject / acfg / sync_inject — the cross-worker combine for an LHS-accumulator pattern where the LHS index is NOT the partition variable must materialise as an element-wise reduce, not a sequence of overwrites.

Cross-references:
- nuc-nucleus/examples/08-histogram/schedules/distributed.sched.nuc (the cell that surfaced the gap, committed cycle 186).
- 03-reduction/distributed.sched.nuc (the contrasting disjoint-write shape).
- 04-prefix-sum/prog.algo.nuc 'block_off' (a similar 'masked accumulator' shape, but single-worker so cross-worker combine never fires).
- TASK-0258 partition_rows (the partition-axis infrastructure that already routes input transfers correctly).
- memory project-cross-backend-differential (the bit-identical-across-backends invariant the fix must preserve).

Honest scope: this is a generalisation of the partial-combine machinery; the simplest fix would be to inject an explicit cross-worker reduce pass when the accumulator's LHS index is independent of the partition variable. The harder version would generalise the cross-worker combine to ANY associative algebraic-identity-bearing accumulator (sum, min, max — picking the right identity from kernel attributes). Cycle-186 scope: file the gap precisely; whoever lands the fix can pick the depth.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Algorithm shape that triggers the gap: rectangular LHS-indexed accumulator where the LHS index is NOT the partition variable. Concrete case: 08-histogram has 'histogram[b] <-- bin_inc(histogram[b], input[i], b)' with 'loop i : partition=workers' — every worker writes to histogram[0..BINS] independently; cross-worker combine MUST sum element-wise but emits 4 sequential overwrites in 08-histogram/distributed.sched.nuc currently
- [ ] #2 Compiler must distinguish (a) DISJOINT-write accumulator (03-reduction shape: 'partials[w]' where w IS the partition variable — each worker owns ONE slot; cross-worker combine is concatenation) from (b) OVERLAPPING-write accumulator (08-histogram shape: 'histogram[b]' where b is INDEPENDENT of partition variable — every worker writes the full output array; cross-worker combine is element-wise reduce by the accumulator's algebraic identity)
- [ ] #3 Bit-identical PASS for at least one tier-1 backend on 08-histogram/distributed: cell 'nuc-nucleus/examples/08-histogram' × 'distributed' × pthreads-sync (or whichever tier-1 path lands first) produces output.bin matching reference.bin (committed cycle 186)
- [ ] #4 Cross-backend differential: same cell PROMOTED to [[required]] in nuc-nucleus/e2e-matrix.toml across all 4 tier-1 backends bit-identical when the codegen lands
- [ ] #5 Symptom pin (regression-prevention test): the cycle-186 mismatch shape was output = [N/NUM_WORKERS] * BINS = 16 copies of N/(NUM_WORKERS*BINS-uniformity-per-partition) — i.e. one worker's standalone histogram. Add a per-backend negative test that bites if the host combine ever regresses to last-write-wins for an OVERLAPPING-write accumulator
<!-- AC:END -->
