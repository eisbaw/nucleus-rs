---
id: TASK-0169
title: >-
  NameSidecar must carry per-KernelId param/return ResolvedType for
  EventList-only scalar-arg casts
status: To Do
assignee: []
created_date: '2026-05-18 23:05'
labels:
  - M2
  - compiler
  - backend
dependencies:
  - TASK-0160
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124 full byte-identical. TASK-0160 landed NameSidecar with data_types (per DataId ResolvedType), consts, and loop_bounds — sufficient for vec! pre-init sizing, slot typing, and source-form loop bounds. But pthreads-sync render_call_args (backends/pthreads-sync/src/lib.rs ~600-640) still reads ctx.algo.kernels.get(callee).params (Vec<ResolvedType>) to decide scalar-arg casts (e.g. an iter-var-derived i64 fed to a usize-typed kernel param needs a cast; whole-array vs scalar param dispatch in render_call_arg). The EventList FireBinding (TASK-0156) carries the argument VALUES but not the kernel's declared param/return ResolvedTypes, and NameSidecar (TASK-0160) carries only DATA types, not KERNEL signature types. An EventList-only backend (TASK-0124) therefore cannot reproduce the exact cast/dispatch without AlgoIR kernels. Extend NameSidecar with kernel_sigs: BTreeMap<KernelId,{params:Vec<ResolvedType>, ret:Option<ResolvedType>}> (keyed by the same KernelId Event::Fire carries; built from linked.algo.kernels via acfg.name_kernels, exactly like data_types is built from algo.data). Additive; same no-fold/no-Net-touch discipline as TASK-0160. Prove sufficiency by reconstructing a scalar-cast call arg for the e2e set from sidecar+EventList alone (TASK-0156 style). NOTE: for the current e2e set 01/02/03/05/07 the only casts are iter-var i64->usize INDEX casts (not kernel-param), so TASK-0124 may be byte-identical WITHOUT this for those 5 — but the contract is not fully AlgoIR-free until kernel sigs are in the sidecar; verify during TASK-0124 whether any of 01/02/03/05/07 actually trips render_call_arg's param_ty path before deciding TASK-0124 ordering.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 NameSidecar carries per-KernelId param + return ResolvedType, keyed by the KernelId Event::Fire carries, deterministically (BTreeMap)
- [ ] #2 build_sidecar populates it from linked.algo.kernels via acfg.name_kernels (same pattern as data_types); eval_const/Net/Repeat.range untouched
- [ ] #3 Sufficiency test (TASK-0156 style): a scalar-arg cast call is reconstructed from NameSidecar+EventList alone, no AlgoIR kernels walk
- [ ] #4 Determinism + bit-identical e2e 01/02/03/05/07 preserved (no sidecar codegen consumer yet)
<!-- AC:END -->
