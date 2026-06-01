---
id: TASK-0360
title: >-
  ACFG + codegen: kernel-less identity-copy Operation (follow-out of TASK-0347
  link half)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-28 05:07'
updated_date: '2026-06-01 22:42'
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

=== Cycle-238 DESIGN SLICE outcome — option (c), CLOSED (orchestrator in-thread per feedback-spawned-agents-refuse-code-edits) ===

User chose the bounded design slice. Empirically RE-VERIFIED the two cycle-230 blockers (not just trusted): Operation.kernel / DataflowEdge.kernel (acfg/types.rs:51,157) + Event::Fire.kernel (event.rs:688) all non-optional KernelId; resolve_worker_set keys on a KERNEL name (build.rs); place_data D in REGION -> ResolvedPlaceData(memory region), not a worker set. Both confirmed.

DECISION = option (c): decline the kernel-optional refactor. Rationale (architect-confirmed GO): (i) ZERO in-tree demand (no example uses bare-LValue dataflow; 15-transpose uses explicit xpose, bit-identical); (ii) the refactor ripples ~17 files + 7 backends + Event contract = HIGH regression risk for a LOW node; (iii) option (b) [derive worker set from link consumer maps] is UNSOUND for codegen — those maps are advisory/over-reporting (multi-source RHS = last-insert-wins), so (b) inherits the same worker-set ambiguity (a) would solve with a real directive. The deferred clean path is filed as TASK-0416 (live trigger).

EMPIRICAL FINDING that reframed the slice: the bare-LValue path was a SILENT DROP, not a benign no-op. build_dataflow returned None -> build_stmt Ok(None) -> build_seq filtered it out. A SAME-WORKER copy (c <-- a, all on host) passes link (MissingCrossWorkerTransfer is cross-worker-only) and compiled to NOTHING — c stayed at its alloc default = silent wrong answer. Proven by a probe (build_acfg Ok, operation_count=2, copy missing). So option (c) honest form REQUIRES fail-loud.

DELIVERED (the concrete root-cause fix, commits 7a5bea2 + 08e4b6c):
- acfg/errors.rs: BuildAcfgError::KernelLessDataflowRhs { lhs, rhs } + Display (points at the explicit-kernel workaround).
- acfg/build.rs: build_dataflow returns Result; bare-LValue arm now Err; build_stmt collapsed Option -> non-Option (kills the silent-drop affordance at type level; architect P2-1); reject-site documents layer choice + declared-transfer caveat (P2-2/P2-3).
- acfg/mod.rs: module docstring corrected ("treats as no-ops" was a lie).
- 5 stale "still skips / emits nothing" doc-lies swept (architect P1-1): build.rs:170, link/dataflow.rs, 15-transpose prog.algo.nuc + kernels.rs + README.md (anchor acfg-skip->acfg-reject). This was TASK-0360 scope #4.
- tests/acfg.rs: 2 negative (same-worker copy + arithmetic) proving the guard bites + 1 positive control.

GATE (re-run after fold-back, qa-test-runner GO + mped-architect GO): build/clippy clean; test 1243/0/3; test-release 1242/0/3; e2e 385/328/0/57/0 (baseline held, qa re-ran x2 non-flake); 9 just-ci structural/doc fences OK.

COST CARRIED (architect): now that acfg REJECTS the bare-LValue form, the TASK-0347 link-half copy-edge analysis (propagate_copy_edges) has NO surviving production exerciser — reached only by its own unit tests. If TASK-0416 ever lands, it becomes live again. Left in place (it still gives a better/earlier diagnostic for the cross-worker-no-transfer sub-case; removing it is scope creep).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
CLOSED via design slice as option (c) (wont-implement-the-refactor + fail-loud guard). Original ACs #2/#3/#4 (kernel-optional Operation, cross-worker Xfer lowering, drop xpose with bit-identical output) are NOT built — explicitly DECLINED, not gamed: a kernel-less data-move IR node is disproportionate to a LOW node with zero in-tree demand, and worker-set derivation has no sound schedule-language home (option b unsound for codegen). What landed instead is the honest root-cause fix the decision implies: the previously SILENT bare-LValue drop (a same-worker copy compiled to nothing) is now a typed BuildAcfgError::KernelLessDataflowRhs, the dead Option silent-drop affordance is removed, and the 5 stale doc-claims (scope #4) are swept. Deferred clean path filed as TASK-0416. Gate: test 1243/0/3, test-release 1242/0/3, e2e 385/328/0/57/0; qa GO + architect GO.
<!-- SECTION:FINAL_SUMMARY:END -->
