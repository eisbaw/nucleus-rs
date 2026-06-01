---
id: TASK-0416
title: >-
  kernel-less data-move IR node — deferred clean path (TASK-0360 option-c
  closure successor)
status: To Do
assignee: []
created_date: '2026-06-01 22:41'
labels:
  - compiler
  - ir
  - deferred-trigger
  - tech-debt
dependencies:
  - TASK-0360
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Successor / re-open trigger for TASK-0360, which was CLOSED via its design slice as option (c): decline the kernel-optional refactor and instead make the kernel-less dataflow RHS FAIL LOUD (BuildAcfgError::KernelLessDataflowRhs). This task carries the DEFERRED clean path so it stays tracked and actionable if demand arises.

TRIGGER (pick up only when one of these fires):
- A real in-tree example genuinely needs the bare-LValue data-move form codegen'd (today ZERO examples do; 15-transpose uses the explicit `xpose` identity kernel, which is the canonical v2 surface and is bit-identical).
- A schedule-language design decision makes data placement first-class.

SCOPE (when picked up):
1. Resolve the worker-set blocker FIRST (design slice). A bare data-move has no `place <kernel> on <workers>` directive; resolve_worker_set keys on a KERNEL name. place_data D in REGION maps data to a MEMORY REGION, not a worker set. Options:
   (a) Add a first-class data->worker schedule directive (e.g. `place_data D on W` distinct from the region form).
   (b) Derive the move's worker set from the link-side consumer maps (link::dataflow::propagate_copy_edges). CAUTION (architect cycle-238 finding): those maps are ADVISORY / over-reporting by design — the multi-source RHS case (D <-- A + B, A/B on different workers) is resolved last-insert-wins, which is fine for an existence check but UNSOUND for codegen worker-set derivation. (b) cannot be lifted to codegen without first solving the same ambiguity (a) solves with a real directive.
   (c) was the chosen disposition (keep explicit-kernel surface; closed on TASK-0360).
2. Make acfg::Operation.kernel / acfg::DataflowEdge.kernel / event::Event::Fire.kernel kernel-OPTIONAL (or add a dedicated DataMove node) — ripples ~17 compiler files + sidecar kernel_sigs join + presentation Event contract + all 7 tier-1 backend fire renderers. HIGH regression risk to the e2e baseline — its own focused session.
3. Cross-worker data-move lowers to the same Xfer pair a Call would.
4. 15-transpose: drop `xpose`, use bare-LValue, emit bit-identical output.bin on naive + distributed-rows. Remove the now-obsolete KernelLessDataflowRhs guard + the 5 'rejected' doc-claims it added.

NOTE: once acfg rejects the bare-LValue form (TASK-0360), the TASK-0347 link-half copy-edge analysis (propagate_copy_edges) has NO surviving production exerciser — it is reached only by its own unit tests. If this task lands, that analysis becomes live again.

Pointers: nucleus/nucleus-compiler/src/acfg/build.rs (build_dataflow _ => Err arm + KernelLessDataflowRhs); acfg/errors.rs (the variant); link/dataflow.rs (propagate_copy_edges). The code 'see TASK-0360' pointers are the decision record; this task is the live deferred work.
<!-- SECTION:DESCRIPTION:END -->
