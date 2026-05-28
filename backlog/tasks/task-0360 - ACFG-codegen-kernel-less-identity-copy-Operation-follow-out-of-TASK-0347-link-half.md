---
id: TASK-0360
title: >-
  ACFG + codegen: kernel-less identity-copy Operation (follow-out of TASK-0347
  link half)
status: To Do
assignee: []
created_date: '2026-05-28 05:07'
updated_date: '2026-05-28 05:42'
labels:
  - compiler
  - ir
  - deferred-trigger-fired
dependencies:
  - TASK-0347
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed cycle 230 as the ACFG/codegen follow-out of TASK-0347 ===

TASK-0347 landed the LINK-LAYER half of identity-copy support: `analyse_dataflow` now records identity-copy (`D <-- E`, bare-LValue RHS, no kernel) producer/consumer edges via transitive value-flow propagation (`propagate_copy_edges`), so a cross-worker identity copy is caught by the MissingCrossWorkerTransfer existence check. That closed TASK-0097's original concern.

What TASK-0347 did NOT do (this task): the ACFG + codegen side. `acfg::build::build_dataflow` still skips a bare-LValue RHS (`_ => None` at acfg/build.rs:325) because a kernel-less Operation is NOT representable today:

- `acfg::Operation.kernel` and `acfg::DataflowEdge.kernel` are non-optional `KernelId`.
- The presentation `event::Event::Fire.kernel` is a non-optional `KernelId`.
- The sidecar `kernel_sigs` join, `acfg_to_petri` transition label, `petri_to_events` Fire emit, `transfer_inject` consumer index, and all 7 tier-1 backend fire renderers assume a concrete kernel.

Making the kernel optional ripples through ~17 compiler files + the sidecar + the presentation Event contract + all 7 backends.

## Blocker that must be resolved first: worker-set derivation

A bare-LValue data-move has NO `place <kernel> on <workers>` directive, so `resolve_worker_set` has nothing to resolve from. The only data-oriented schedule directive is `place_data D in REGION`, which maps a data symbol to an opaque MEMORY REGION, not a worker set. There is genuinely NO schedule-language concept that maps a data symbol -> worker set today. TASK-0347 AC#1's "the Operation's worker set is the LHS's worker placement" is under-specified: data placement is not a first-class schedule concept the way kernel placement is.

Plausible resolutions (pick one in a design slice before coding):
  (a) Add a data->worker schedule directive (e.g. `place_data D on W` distinct from the region form), making data placement first-class.
  (b) Derive the data-move's worker set from the LHS's transitive consumer set (the link-side last-writer/consumer maps TASK-0347 now compute), making the ACFG worker set a DERIVED quantity rather than a directive read.
  (c) Keep the identity kernel convention (`xpose`) as the canonical surface and close this as wont-fix, documenting that the bare-LValue grammar form is link-validated but not codegen-supported.

## Scope (when picked up)
1. Resolve the worker-set blocker (design slice).
2. Make Operation/DataflowEdge/Event::Fire kernel-optional OR introduce a dedicated DataMove node, threaded through acfg_to_petri / petri_to_events / sidecar / all 7 backends.
3. Cross-worker data-move lowers to the same Xfer pair a Call would (was TASK-0347 AC#2).
4. 15-transpose: drop the `xpose` kernel + use bare-LValue; emit bit-identical output.bin on naive + distributed-rows (was TASK-0347 AC#3). Sweep prog.algo.nuc / kernels.rs / README / both schedules' `place xpose` lines + the "skipped at M1" doc-claims for consistency.

## Pointers
- acfg/build.rs:325 (`_ => None`, "Identity copy ... skipped at M1") — carries a `// TASK-0360` reference.
- link/dataflow.rs `propagate_copy_edges` + `analyse_dataflow` docstring (the landed link half).
- nuc-nucleus/examples/15-transpose/* (the example to simplify).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== forward-carried from TASK-0347 (cycle 230) ===

TASK-0347's link half landed (commit bcde6b9): identity-copy
producer/consumer attribution via `propagate_copy_edges`. This task
(TASK-0360) carries the ACFG/codegen half. Two empirically-verified
blockers from the cycle-230 investigation:
1. `acfg::Operation.kernel`, `acfg::DataflowEdge.kernel`, and
   `event::Event::Fire.kernel` are ALL non-optional KernelId. A
   kernel-less move needs either Option<KernelId> (ripples ~17 compiler
   files + sidecar kernel_sigs join + presentation Event contract + all 7
   tier-1 backend fire renderers) or a dedicated DataMove node.
2. Worker-set derivation has NO schedule-language home: `place_data D in
   REGION` maps to an opaque memory region, NOT a worker set. Resolve
   this in a design slice BEFORE coding (options (a)/(b)/(c) in the task
   description). build_dataflow's `_ => None` skip site
   (nucleus-compiler/src/acfg/build.rs) carries a // TASK-0360 reference.

=== cycle-230b architect review fold-back (P3-3) ===
Resolution option (c) (keep the xpose identity-kernel convention, close
kernel-less codegen as wont-fix) has a cost worth stating: it leaves the
TASK-0347 link-half machinery (propagate_copy_edges + the identity-copy
CopyEdge path) as TEST-ONLY code — no in-tree example exercises a
bare-LValue RHS, so the only callers are the synthetic identity_copy_*
tests in nucleus-compiler/tests/link.rs. Options (a)/(b) give the link
half a production exerciser; (c) does not. Weigh that when choosing.
<!-- SECTION:NOTES:END -->
