---
id: TASK-0384
title: >-
  08-histogram DISTRIBUTED native scatter (partitioned data-dependent WRITE +
  cross-worker bin fan-in)
status: To Do
assignee: []
created_date: '2026-05-31 05:03'
updated_date: '2026-05-31 13:47'
labels:
  - compiler
  - scatter
  - histogram
  - distributed
  - broaden
dependencies:
  - TASK-0376
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BROADEN follow-up to TASK-0376 (which landed the SINGLE-WORKER native scatter histogram[input[i]] <-- inc(histogram[input[i]]), 7 tier-1 backends bit-identical, e2e 329->336). The distributed step partitions `input` across workers, each worker scatters into a LOCAL partial histogram, then a cross-worker combine sums the partials element-wise into the final BINS-wide histogram.

This is the WRITE analog of the deferred 17-spmv DISTRIBUTED gather (whole-array broadcast). Two distinct hard problems vs the single-worker slice:
1. A data-dependent WRITE under a `partition=` schedule: the target bin `input[i]` is NOT statically known, so the transfer/halo inference cannot place writes to a partitioned `histogram` per-worker. Either (a) replicate the full `histogram` per worker (each worker scatters its input slice into a private full-width partial) + element-wise-sum combine on the host, OR (b) reject fail-loud under a partitioned schedule (today's behaviour — verify which guard fires: halo_inference data-dependent-stride rule, transfer_inject, or the scatter render path).
2. The cross-worker partial-histogram combine is the SAME overlapping-write accumulator fan-in TASK-0343 solved for 08-histogram/distributed (the masked variant): collect_accumulate_waits + render_wait_assign(accumulate) element-wise wrapping_add into a pre-initialised dest. Check whether the scatter variant's per-worker-partial combine reuses that helper or needs a new shape.

Scope: a distributed.scatter.sched.nuc + the partition-aware data-dependent-WRITE lowering/codegen; bit-identity vs reference.bin across the tier-1 backends where it compiles; honest [[skip]] for backends that can't. Companion to TASK-0044.04 / TASK-0341.03.02 (distributed gather).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0373 (distributed gather, landed b121365+14cc5b8). EMPIRICALLY VERIFIED which guard fires for the distributed scatter today (item #1): halo_inference DataDependentStride. The scatter RHS histogram[input[i]] is a data-dependent READ that halo_inference DOES walk (via visit_arg/process_call); the LHS write index input[i] is NOT walked by halo at all (halo only inspects kernel-call ARGS, never LHS). So the rejection point is the RHS RMW read, not the write.

TASK-0373 relaxed DataDependentStride to advisory ONLY for a PURE GATHER (affine LHS) and added an is_scatter_rmw flag (halo_inference.rs) that KEEPS a scatter RMW (data-dependent LHS) FATAL under partition. The flag is computed at collect_from_stmts: lhs.indices.any(expr_contains_dataref_or_call), threaded CallSite->IndexSite->DataDependentStride{is_scatter_rmw}. error_is_fatal_under_partition: is_scatter_rmw && any-scope-iv-partitioned. So today distributed scatter is rejected with: halo-inference error (under partitioned iv): kernel call inc reads histogram with a data-dependent index at axis 0.

CRITICAL GOTCHA for whoever lands TASK-0384: option (a) replicate-full-histogram-per-worker + element-wise-sum combine ALREADY WORKS END-TO-END if you simply flip is_scatter_rmw handling to advisory. I verified this: with a throwaway distributed_scatter.sched.nuc (partition=workers(i), input+histogram sync) and the unconditional-advisory halo relaxation, the emit was byte-identical to reference.bin on pthreads-sync — because each worker scatters its input-slice into a PRIVATE full-width histogram (vec![0;16]) and the TASK-0343 collect_accumulate_waits + render_wait_assign(accumulate) combine sums the partials element-wise at the host. So item #2 (cross-worker combine) is ALREADY HANDLED by the TASK-0343 accumulator helper — no new combine shape needed for option (a). The work for TASK-0384 is therefore mostly: (1) decide option (a) vs (b) [a works today, just gated off], (2) flip the is_scatter_rmw fatal gate to admit option (a) under partition, (3) confirm the input partition i-bands + histogram replicates-then-accumulates, (4) add distributed.scatter.sched.nuc + 7 cells. WATCH the FIFO send/recv order trap (TASK-0373 gotcha 2): for the scatter, input is the only host->worker broadcast and there is no gather-index array, so the order trap likely does NOT bite — but VERIFY bufsync/poll emit byte-identical (do not trust the 5 event backends passing; they mask order bugs per project-mp-tcp-event-vs-bufsync-safety-profile).

SOUNDNESS NOTE: option (a) replicate-per-worker is only correct because every input[i] is pre-clipped to a valid bin (prog.scatter.algo.nuc pre-condition) AND the partition is over input-index i, not over histogram bins. If a future schedule partitions over BINS, replicate-per-worker is wrong (each worker would only own a bin band). Keep is_scatter_rmw FATAL for any partition that is not the input-index-partition + full-histogram-replicate shape.
<!-- SECTION:NOTES:END -->
