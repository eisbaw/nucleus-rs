---
id: TASK-0432
title: >-
  TASK-0430 follow-up: distributed textbook scatter + dedicated
  unconstrained-input fixture
status: To Do
assignee: []
created_date: '2026-06-02 23:43'
labels:
  - compiler
  - scatter
  - histogram
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two BROADEN follow-ups to TASK-0430 (X1 pure-call-in-index, single-worker 08-histogram/textbook). (1) DISTRIBUTED textbook scatter: partition the input i loop across workers, per-worker private partial histograms (whole-array replicate, the data-dependent write index marks histograms dim OPAQUE), host element-wise-sum combine - the analog of distributed.scatter.sched.nuc (TASK-0384) but with the bucket() call in index position. Soundness same as the bounded distributed scatter (partition over input index i, never over bins). (2) DEDICATED UNCONSTRAINED-INPUT FIXTURE: the landed textbook example shares 08-histograms input.bin/reference.bin (one oracle per example dir), which is pre-clipped to [0,BINS) so bucket(v)==v at RUNTIME for that fixture - the modulo is a no-op, so the unconstrained-input strength is only demonstrated at the algorithm-surface/codegen level, not at runtime. A truly-unconstrained-input demonstration needs its OWN example dir (its own input.bin with values outside [0,BINS) + its own reference.bin computed through the modulo bucket) so the bucket() does real work at runtime. Keep separate to avoid perturbing the shared 08 oracle.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 distributed textbook scatter schedule bit-identical across the applicable tier-1 backends (input-index partition; whole-array histogram replicate; host element-wise-sum combine)
- [ ] #2 a dedicated example dir ships truly-unconstrained input (values outside [0,BINS)) with a reference.bin computed through the modulo bucket, so bucket() does real runtime work; bit-identical PASS
<!-- AC:END -->
