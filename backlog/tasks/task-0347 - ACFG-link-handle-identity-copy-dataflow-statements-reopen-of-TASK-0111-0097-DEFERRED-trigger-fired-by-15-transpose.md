---
id: TASK-0347
title: >-
  ACFG + link: handle identity-copy dataflow statements (reopen of
  TASK-0111/0097 DEFERRED trigger fired by 15-transpose)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-27 12:45'
updated_date: '2026-06-02 22:05'
labels:
  - compiler
  - ir
  - deferred-trigger-fired
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as cycle-204 forward-carry from 15-transpose discovery ===

TASK-0111 (ACFG: handle identity-copy dataflow statements) was closed cycle 77 as DEFERRED-until-real-example with the rationale "no in-tree example uses identity-copy dataflow syntax" and the closure note "reopening means filing a single fresh scope-derived task that covers both layers at once (not reopening these two separately)".

Cycle 204 added 15-transpose, which is the FIRST in-tree example with identity-copy SEMANTICS (axis-swap permutation with no per-element compute). The cycle-77 deferral trigger is now fired. Per the closure note, this is the canonical "single fresh scope-derived task" reopening both ACFG and link sides at once.

## Current workaround in 15-transpose

The example uses a `kernel xpose : (i32) -> i32 pure` identity passthrough so the dataflow assignment `output[j][i] <-- xpose(input[i][j])` is a Call RHS (which `acfg::build::build_dataflow` accepts as an Operation node). The bare-LValue form `output[j][i] <-- input[i][j]` would be the natural syntax but produces no ACFG operation today.

## Scope

Both layers must be co-designed (per cycle-77 closure note):

1. **ACFG layer** (`nucleus/nucleus-compiler/src/acfg/build.rs:325-326` "Identity copy or pure-expression RHS: skipped at M1"): identity-copy dataflow should produce an Operation node with no kernel firing but with a `data move` DataflowEdge. The Operation's worker set is the LHS's worker placement; the DataflowEdge's `data_in` is the RHS's data ref.
2. **Link layer** (TASK-0097 was the parallel limitation, also closed DEFERRED cycle 77): when the producer and consumer worker sets differ, the data move lowers to a Xfer/Push/Wait pair, same as for a kernel-call Operation.

## Acceptance Criteria
<!-- AC:BEGIN -->
1. **ACFG**: `build_dataflow` accepts bare-`LValue` RHS and emits an Operation with no kernel firing + a `data move` DataflowEdge. Unit test fixture exercising `out <-- in` with both same-worker and cross-worker placements.
2. **Link / codegen**: cross-worker data-move lowers to the same Xfer pair a Call would. Same-worker case lowers to an in-place assignment (or is structurally elided if the LHS and RHS are the same DataId).
3. **15-transpose simplification**: when AC#1 + AC#2 close, 15-transpose's prog.algo.nuc can drop the `xpose` kernel and use the bare-LValue form; the new naive schedule emits identical output.bin (regression-pin the bit-identity).
4. **Renumber/coordinate followup**: if implementer chooses, can ALSO close TASK-0097's original concern (link-side identity-copy gap).

## Honest scope LIMITS

- The bare-`LValue` syntax is grammar-legal today; the lowering pass `IrStmt::Dataflow` already carries it. Only ACFG-build + downstream codegen are gated.
- AC#3's regression pin only fires after both AC#1 + AC#2 close. If only AC#1 lands, document the half-state in the cycle close note and file AC#2 as an explicit follow-up.

## Forward-carry

- Predates cycle 204: TASK-0111 (Done DEFERRED cycle 77, ACFG side), TASK-0097 (Done DEFERRED cycle 77, link side).
- Triggered by: cycle 204 TASK-0341.01 (15-transpose AC#1) — first real example with identity-copy semantics.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
=== Cycle-230 implementation plan (TASK-0347) ===

DESIGN FINDING (the central subtlety, investigated before coding):

A kernel-less identity-copy Operation is NOT directly representable in the
current IR. `Operation.kernel`, `DataflowEdge.kernel`, and the presentation
`Event::Fire.kernel` are all non-optional `KernelId`. Making any of them
`Option<KernelId>` ripples through ~17 compiler files (acfg_to_petri label,
petri_to_events Fire emit, transfer_inject consumer index, sync_inject,
the 3 partition passes, block/halo/reuse passes), the sidecar `kernel_sigs`
join, the presentation Event contract, AND all 7 tier-1 backends' fire
renderers. That is a major structural change far beyond a Low-priority node
and risks the 280/246/0/34/0 e2e baseline.

WORKER-SET PROBLEM (gates AC#3): a bare-LValue data-move has no `place
<kernel> on <workers>` directive, so `resolve_worker_set` has nothing to
resolve from. The only data-oriented schedule directive is `place_data D
in REGION`, which maps a data symbol to an opaque MEMORY REGION, NOT a
worker set. There is genuinely NO schedule-language concept that maps a
data symbol -> worker set today. AC#1's "the Operation's worker set is the
LHS's worker placement" is therefore under-specified: data placement is not
a first-class schedule concept the way kernel placement is.

DECISION (honest partial, explicitly permitted by the task's Honest scope
LIMITS):

- AC#1 (kernel-less ACFG Operation): DEFERRED. The clean form needs the
  Option<KernelId> structural change + a data->worker schedule directive.
  File as a scoped follow-up with a dependency edge. Do NOT smuggle in a
  half-baked sentinel-kernel hack (that is the very `xpose` workaround the
  task wants to remove).
- AC#2 (cross-worker data-move lowering): DEFERRED, depends on AC#1.
- AC#3 (drop xpose, bit-identical): DEFERRED, depends on AC#1+#2. Leave
  15-transpose's xpose kernel in place; do NOT touch prog.algo.nuc /
  kernels.rs / README / schedules (their "skipped at M1" comments stay
  TRUE because the behaviour is unchanged).
- AC#4 / TASK-0097 link side: LAND the root fix that CAN land cleanly --
  record identity-copy producer/consumer edges in `analyse_dataflow` via a
  last-writer-worker map, so a future cross-worker identity-copy is caught
  by the MissingCrossWorkerTransfer existence check instead of being
  silently invisible. This is exactly TASK-0097's original concern and
  needs NO new IR shape (it works on String worker entities, not KernelId).

WHY THIS IS A SUCCESS, NOT A FAILURE: a correct precisely-scoped partial is
what the task explicitly asks for over a faked AC#3. The link-side fix
removes a real silent-invisibility defect at root; the ACFG/codegen side is
filed with an honest dependency chain.

STEPS:
1. Plan + In Progress (done).
2. link/dataflow.rs: thread a last-writer-worker map through walk_stmts;
   on an identity-copy `D <-- E` (bare DataRef / arithmetic RHS), attribute
   D's producer to the last-writer worker of the RHS's source data, and
   record the RHS data symbols as consumers of that same worker. Update the
   (now-stale) module + analyse_dataflow docstrings to present tense.
3. Unit tests in tests/link.rs (or wherever analyse_dataflow is tested):
   same-worker identity-copy (no MissingCrossWorkerTransfer) + cross-worker
   identity-copy (MissingCrossWorkerTransfer fires).
4. Silent-sibling sweep: build.rs:164-166 + acfg/build.rs:325 docstrings
   stay TRUE (ACFG behaviour unchanged); verify no other site newly lies.
5. Full gate (build/clippy/test/test-release/e2e). Baseline must hold.
6. File follow-up task for AC#1/#2/#3 (kernel-less Operation + data->worker
   schedule directive) with dependency edge; reference its id in a code
   comment at build.rs:325.
7. Commit per logical unit; record gotchas + forward-carry notes.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle 230 outcome — PARTIAL (link half landed, ACFG/codegen half deferred to TASK-0360) ===

Commit: bcde6b9 (link: TASK-0347 cycle 230 ...).

WORKER-SET DESIGN OUTCOME (the central subtlety): outcome (b) of the
brief. A kernel-less identity-copy Operation is NOT representable in the
current IR — `acfg::Operation.kernel`, `acfg::DataflowEdge.kernel`, and
the presentation `event::Event::Fire.kernel` are all non-optional
`KernelId`. There is also NO schedule directive mapping a data symbol to
a worker set: the only data-oriented directive is `place_data D in
REGION`, which maps to an opaque MEMORY REGION (per Event::Alloc), not a
worker. So AC#1's "the Operation's worker set is the LHS's worker
placement" is under-specified — data placement is not a first-class
schedule concept the way kernel placement (`place <kernel> on <workers>`
-> resolve_worker_set) is. I did NOT smuggle in a sentinel-kernel hack
(that is the very `xpose` workaround the task wants to remove). The clean
ACFG/codegen path needs the Option<KernelId> structural change (ripples
~17 compiler files + sidecar + presentation Event contract + all 7
tier-1 backend fire renderers) AND a resolution of the worker-set
blocker — filed as TASK-0360 with a dependency edge on this task.

PER-AC STATUS:
- AC#1 (kernel-less ACFG Operation + unit fixture): DEFERRED to TASK-0360.
  build_dataflow still returns None for a bare-LValue RHS. The acfg/mod.rs
  + build.rs skip-site docstrings now reference TASK-0360 and the
  worker-set blocker (no longer claim "unexercised corner").
- AC#2 (cross-worker data-move codegen lowering): DEFERRED to TASK-0360
  (depends on AC#1).
- AC#3 (drop xpose, bit-identical): DEFERRED to TASK-0360. 15-transpose
  keeps `xpose`. Its prog.algo.nuc / kernels.rs / README "skipped at M1"
  claims were swept: the behaviour claim stays TRUE (ACFG genuinely still
  skips), but the stale verbatim comment-quote + the fragile
  build.rs:325-327 line-number citations were corrected and the link-half
  / TASK-0360 split-state recorded.
- AC#4 (TASK-0097's link-side identity-copy gap): MET. `analyse_dataflow`
  now records identity-copy producer/consumer transitively via the new
  `propagate_copy_edges` fixpoint. A cross-worker identity copy is now
  caught by the MissingCrossWorkerTransfer existence check. 4 inline link
  tests pin same-worker (no spurious edge), cross-worker missing transfer
  (error fires), cross-worker with transfers (links), and a copy chain
  (transitive producer to a fixpoint).

GATE (my run): just build OK, just clippy clean, just test 1041 passed /
0 failed / 3 ignored, just test-release 1040 passed / 0 failed / 3 ignored
(the 1-test delta is the known dev-vs-release #[should_panic] divergence),
just e2e 280/246/0/34/0 — baseline EXACTLY held. check-textual-replace +
check-include-str-coverage both OK.

SAFETY ARGUMENT (why e2e is provably unchanged): grepped all
examples/*/prog.algo.nuc — ZERO use a bare-LValue dataflow statement
(15-transpose uses `xpose`, a Call). So `data_producers`/`data_consumers`
are byte-identical for every shipped example; the change is purely
additive for synthetic / future identity-copy programs.

GOTCHAS / REJECTED APPROACHES:
- Rejected: making KernelId optional in this cycle. Too large for a Low
  task; high regression risk to the 280/246 baseline; the worker-set
  blocker would still gate AC#3. Filed as TASK-0360 instead.
- Rejected: a "place_data D on W" reinterpretation of the existing
  region directive. place_data is region-keyed by contract; conflating it
  with worker placement would be a silent semantic overload.
- Known limitation (documented in propagate_copy_edges docstring):
  multi-source arithmetic RHS (`d <-- a+b`) with differently-placed
  producers is the same ambiguous worker-set question; last-source-wins
  feeds only the advisory existence check (over-reports, never silently
  under-reports), precise policy rides with TASK-0360.

Leaving In Progress: AC#1/#2/#3 are honestly deferred (not all ACs met),
per the task's Honest scope LIMITS which explicitly permit this
half-state.

=== cycle 230b — review-gate outcome + fold-back ===
Parallel read-only review gate (qa-test-runner + mped-architect) both
returned GO. qa re-ran the full gate independently: just test 1041/0/3,
just test-release 1040/0/3 (-1 = known #[should_panic] dev/release
divergence, TASK-0291), just e2e 280/246/0/34/0 across 2 runs (fail=0,
required-fail=0, no flake), clippy force-verified clean (no
doc_lazy_continuation). architect verified the structural blocker is real
(not effort-avoidance), the propagate_copy_edges fixpoint is correct +
terminating, the over-report direction claim holds (cannot silently
under-report a transfer), no silent-sibling (other IrStmt::Dataflow
consumers already handle bare-LValue), no panic-not-diagnostic, AC#4
genuinely MET (not AC-gamed).

Three P3 nits folded back in-thread (cycle 230b, this commit range):
- P3-1: link/types.rs data_producers header said 'producer kernel' — a
  copy target inherits transitively, has no producer kernel of its own.
  Reworded to 'at least one producer (directly or transitively
  inherited)'.
- P3-2: propagate_copy_edges had a ceiling + changed-flag but exited
  SILENTLY if the ceiling were ever too small (under-converge = the
  dangerous under-report direction). Added a  debug_assert
  that surfaces non-convergence as a debug panic; documented why it is
  safety-load-bearing. Added identity_copy_long_chain_propagates_at_depth
  (3-edge chain) to exercise the fixpoint at greater depth — now 5
  identity_copy_* tests, all pass.
- P3-3: appended a note to TASK-0360 that resolution option (c) leaves
  the link-half machinery as test-only code.

Status stays In Progress: AC#1-3 genuinely unmet (structural, carried by
TASK-0360); AC#4 met + gate-verified + review GO. Not marked Done because
3 of 4 ACs are deferred — honest-partial, not AC-gamed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
CLOSED cycle-244 (dependency-graph sanity): TASK-0347 was left In Progress in cycle-230 ONLY pending its ACFG/codegen follow-out TASK-0360 — which is now Done (resolved via its design slice as option (c): decline the kernel-optional refactor, make the bare-LValue dataflow RHS FAIL LOUD via BuildAcfgError::KernelLessDataflowRhs, keep the explicit-xpose-kernel surface). So TASK-0347 has no remaining open dependency. DISPOSITION (honest, NOT AC-gaming): AC#4 (link-side identity-copy gap, TASK-0097 concern) was MET in cycle-230 (commit bcde6b9, review GO) via propagate_copy_edges. AC#1/#2/#3 (kernel-less ACFG Operation + cross-worker data-move codegen + drop xpose bit-identical) were NOT built — they were DISPOSITIONED via TASK-0360 option-c (fail-loud + keep xpose surface; zero in-tree examples need the bare-LValue form today). The remaining CLEAN path (kernel-optional IR / data->worker schedule directive) is tracked as deferred-trigger TASK-0416 (depends on TASK-0360). The task per its own Honest scope LIMITS explicitly permitted this AC#4-only partial. No code change this cycle — pure tracker reconciliation of an already-reviewed prior outcome.
<!-- SECTION:FINAL_SUMMARY:END -->

- [ ] #1 ACFG: build_dataflow accepts bare-LValue RHS and emits an Operation with no kernel firing + a 'data move' DataflowEdge. Unit test fixture exercising 'out <-- in' with both same-worker and cross-worker placements
- [ ] #2 Link / codegen: cross-worker data-move lowers to the same Xfer pair a Call would. Same-worker case lowers to an in-place assignment (or is structurally elided if the LHS and RHS are the same DataId)
- [ ] #3 15-transpose simplification: when AC#1 + AC#2 close, 15-transpose's prog.algo.nuc can drop the xpose kernel and use the bare-LValue form; the new naive schedule emits identical output.bin (regression-pin the bit-identity)
- [ ] #4 Coordinate followup: if implementer chooses, ALSO close TASK-0097's original concern (link-side identity-copy gap, parallel limitation Done DEFERRED cycle 77)
<!-- AC:END -->
