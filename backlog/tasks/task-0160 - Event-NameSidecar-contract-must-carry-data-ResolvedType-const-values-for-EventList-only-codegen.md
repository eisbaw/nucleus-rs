---
id: TASK-0160
title: >-
  Event/NameSidecar contract must carry data ResolvedType + const values for
  EventList-only codegen
status: To Do
assignee: []
created_date: '2026-05-18 16:43'
labels:
  - M2
  - compiler
  - backend
  - blocker
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124 AC#2 byte-identical. The pthreads-sync backend emits pre-init allocations (let mut c = vec![0; 256]; let mut img_out = vec![0; 256]) and multi-worker Slot types (Arc<Slot<Vec<i32>>>) by reading AlgoIR/LinkedIR ResolvedType (dims product for the vec! length, ScalarType for the element zero literal and arg casts) and AlgoIR consts (const N=256). The per-worker EventList + the proposed NameSidecar (name_kernels/name_data/name_workers) carry NONE of this: DataSlice has only DataId+index IrExprs, no element type, no shape. A backend consuming ONLY the EventList+sidecar cannot size vec![0; N] nor type the slot nor cast scalar args -> cannot produce byte-identical (or even compilable) code without AlgoIR. Decide: extend NameSidecar with a per-DataId {ResolvedType} table and a const name->value table (committable, deterministic, BTreeMap), OR add the element type/shape onto DataSlice. Prefer the sidecar (keeps Event lean; types are schedule-pass-available metadata, like the name tables). Couple with TASK-0159 (loop structure) since TASK-0124's full AC#2 switch needs BOTH.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 NameSidecar (or Event) carries, per DataId, the ResolvedType (dims + ScalarType) sufficient to emit vec![zero; product(dims)] and the Rust element/slot type, deterministically
- [ ] #2 Const name->value (and/or pre-resolved loop-bound) info reaches the contract so the backend renders bounds without AlgoIR consts
- [ ] #3 Determinism + bit-identical e2e for 01/02/03/05/07 preserved
- [ ] #4 Coupled with TASK-0159; together they unblock TASK-0124 AC#2 full switch
<!-- AC:END -->
