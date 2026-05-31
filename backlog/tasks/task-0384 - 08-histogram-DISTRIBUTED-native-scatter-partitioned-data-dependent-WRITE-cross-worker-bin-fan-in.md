---
id: TASK-0384
title: >-
  08-histogram DISTRIBUTED native scatter (partitioned data-dependent WRITE +
  cross-worker bin fan-in)
status: To Do
assignee: []
created_date: '2026-05-31 05:03'
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
