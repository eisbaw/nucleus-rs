---
id: TASK-0156
title: Event contract must carry per-Fire value bindings (arg/output DataId+slice)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 09:41'
updated_date: '2026-05-18 16:44'
labels:
  - M2
  - compiler
  - backend
  - blocker
dependencies:
  - TASK-0150
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124. Event::Fire{kernel,tile} carries no argument/output bindings and no index expressions; acfg_to_events even hardcodes tile=empty. Value-correct codegen (bit-identical e2e) requires knowing, per firing, which (DataId, index-slice) feeds each kernel parameter and which (DataId, slice) it writes — currently only the AlgoIR call/index expressions have this (DataflowEdge::data_in is a bare Vec<DataId>). To let any backend consume ONLY the EventList (TASK-0124 AC#2), extend the Event/ACFG contract to carry per-Fire value bindings, which in turn needs index expressions plumbed through ACFG (TASK-0150). Decide: extend Event::Fire with an arg/out binding payload, or add a sidecar per-firing binding table keyed by (kernel,tile). Must keep determinism + bit-identical e2e.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Event (or a sidecar) carries, per Fire, the ordered input (DataId, slice) bindings and the output (DataId, slice)
- [x] #2 Index expressions survive ACFG->Event (coordinates with TASK-0150)
- [ ] #3 pthreads-sync can regenerate bit-identical code for examples 01/02/03/05/07 from EventList alone
- [x] #4 Determinism + bit-identical e2e preserved
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
DESIGN DECISION: extend Event::Fire with an ordered binding payload (NOT a sidecar). Rationale: at M2 the per-firing IterTile is empty (acfg_to_petri/petri_to_events unroll Repeats and discard the iter-coord), so a sidecar keyed by (kernel,tile) would collide across firings of the same kernel. Co-locating the binding on the Fire keeps each EventList self-contained (matches event.rs design rationale) and composable — the backend reads one event, not event+sidecar join. Cost: Event grows; acceptable, it is the presentation-layer contract and bindings are intrinsic to a firing.

1. event.rs: add FireBinding { inputs: Vec<ArgBinding>, output: Option<DataAccessBinding> } where ArgBinding is either a data read (DataId + index IrExpr list) or a scalar arith expr (IrExpr over iter vars/consts); DataAccessBinding = { data: DataId, indices: Vec<IrExpr> }. Reuse algo::IrExpr (already serde after TASK-0150) — single source of truth, no third expr type. Add `bindings: FireBinding` field to Event::Fire (or wrap). Manual Hash (IrExpr has no Hash) like IterTile.
2. petri_to_events emit_operation: build FireBinding from op.dataflow.edges[0] (data_in_access for reads in arg order; data_out_access for output). For non-DataRef scalar args we need the original IrExpr — extend DataflowEdge access capture (TASK-0150 only kept DataRefs); decide: add an ordered ArgBinding list on DataflowEdge that preserves EVERY kernel arg (data read OR scalar expr), not just data reads. This is the precise per-param binding AC#1 wants.
3. Keep determinism: no HashMap; BTreeMap/Vec order; IrExpr is deterministic data.
4. Tests: (a) event.rs unit test Fire carries ordered bindings; (b) reconstruct a kernel call string from EventList alone for 05-stencil and assert it matches the AlgoIR-walk rendering (proves AC#3 "enough info" WITHOUT switching the backend — that switch is TASK-0124, explicitly out of scope here).
5. Update petri_to_events module docs (the hardcoded empty-tile / no-binding note) and acfg.rs.
6. just test / e2e / determinism / clippy. 7 pass / 0 fail / 3 skipped bit-identical preserved.
7. Be explicit in notes: 0156 makes the EventList CARRY enough + demonstrates reconstruction; 0124 is the task that RIPS OUT the AlgoIR walk in the backend. Do not do 0124 here.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commits on master). DESIGN: inline FireBinding on Event::Fire (not sidecar) — at M2 IterTile is empty so a (kernel,tile)-keyed sidecar would collide; inline keeps each EventList self-contained/composable.

AC#1 DONE: FireBinding{inputs: Vec<ArgBinding>, output: Option<DataSlice>} on every projected Fire; ordered, per kernel parameter. ArgBinding = Data(DataSlice) | Scalar(IrExpr) | Nested{callee,args}.
AC#2 DONE: index exprs survive ACFG->Event verbatim (DataSlice.indices carries AlgoIR IrExpr; proven by eventlist_alone_reconstructs_stencil_kernel_call which rebuilds img_out[y][x] <-- blur3(img_in[(y-1)][(x-1)], ...) from the EventList ALONE).
AC#4 DONE: just test all green; e2e 7 pass/0 fail/3 skipped bit-identical; determinism-check 7/0/3 byte-identical; clippy clean.

AC#3 NOT CHECKED — honest scope boundary: AC#3 literally says "pthreads-sync CAN REGENERATE bit-identical code ... from EventList alone". I PROVED the EventList now CARRIES enough (reconstruction test reconstructs the exact kernel call from FireBinding + name tables, no AlgoIR statement walk) but I did NOT rip out the pthreads-sync AlgoIR walk and switch it to consume the EventList — that switch IS TASK-0124 (this task is its blocker/dependency). Per the workflow brief, for 0156 it is sufficient that the EventList carries enough and that this is demonstrated; the literal "pthreads-sync regenerates from EventList alone" is completed by TASK-0124. Leaving AC#3 unchecked rather than overclaiming.

NOTE/regression caught+fixed: example 14 (hearing-aid) has a nested kernel call in arg position (denoise(mix2(mic_in[frame], bt_in[frame]))). First impl panicked in build_acfg; that was a regression (build_acfg previously admitted ex14). Fixed by representing it faithfully as ArgBinding::Nested — build_arg_bindings is now total; the nested-call rejection stays in the backend (pthreads-sync render_call_arg), not duplicated into ACFG. Also collapsed acfg::DataAccess into a type alias of event::DataSlice (single source of truth).

Post-review hardening (mped-architect Q4.2, blocking before TASK-0124): petri_to_events::emit_operation used edges.first().map(...).unwrap_or_default() — a silent empty FireBinding that would defeat TASK-0156 (a backend would mis-codegen/fail far from cause). Replaced with a loud panic naming the kernel: build_acfg always emits exactly one edge per Operation, so a missing edge is a malformed ACFG, not a tolerable case. Verified green (test/e2e/determinism/clippy).

TASK-0124 follow-up (this session): AC#3 ("pthreads-sync CAN REGENERATE bit-identical code from EventList alone") remains UNCHECKED and TASK-0156 stays In Progress. TASK-0124 investigation found that the backend switch is blocked NOT by the value-binding contract (which THIS task closed and proved via eventlist_alone_reconstructs_stencil_kernel_call) but by two further contract gaps: loop-nest structure is destroyed by acfg_to_events unrolling (filed TASK-0159) and data ResolvedType + const values are absent from the EventList/sidecar (filed TASK-0160). The VALUE half of "regenerate from EventList alone" is done & proven here; the literal AC#3 (backend actually switched, byte-identical) now depends on TASK-0159+0160 then TASK-0124. Leaving AC#3 unchecked is the honest status — the EventList carries enough VALUE info but not enough STRUCTURE/TYPE info for a full byte-identical backend switch yet.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Extended the presentation-layer Event contract so a backend can compute a firing's VALUE from the per-worker EventList alone — unblocks TASK-0124.

Changes:
- event.rs: DataSlice{data,indices:Vec<IrExpr>}, ArgBinding(Data|Scalar|Nested{callee,args}), FireBinding{inputs,output}. Event::Fire gains an inline `bindings` payload (chosen over a sidecar: at M2 IterTile is empty so a (kernel,tile)-keyed sidecar collides; inline keeps each EventList self-contained). Event::fire/fire_bare helpers. Manual Hash so Event:Hash holds.
- acfg.rs: acfg::DataAccess collapsed to a type alias of event::DataSlice (single source of truth, TASK-0150+0156 needed the same struct). DataflowEdge gains positional args:Vec<ArgBinding> populated by build_acfg; build_arg_bindings is total and represents nested kernel calls (example 14) faithfully as ArgBinding::Nested instead of panicking — the nested-call rejection stays in the backend, not duplicated into ACFG construction.
- petri_to_events: emit_operation builds FireBinding from the Operation's DataflowEdge (positional args + data_out_access) and attaches it to every projected Fire.

DESIGN/scope boundary (explicit & honest): this task makes the EventList CARRY enough and PROVES it (eventlist_alone_reconstructs_stencil_kernel_call rebuilds the exact 05-stencil kernel call from the EventList + name tables, NO AlgoIR statement walk). It deliberately does NOT switch pthreads-sync off its AlgoIR walk — that is TASK-0124 (To Do, depends on this task). Therefore AC#1/#2/#4 are met and checked; AC#3 ("pthreads-sync CAN regenerate ... from EventList alone") is met in spirit (data present + proven) but its literal wording (backend actually switched) is completed by TASK-0124. TASK-0156 is left In Progress until TASK-0124 lands so AC#3 is not overclaimed.

Tests: 4 event.rs unit tests + 2 petri_to_events pipeline tests (the AC#3 reconstruction proof + all-e2e binding presence). just test all green; e2e 7 pass/0 fail/3 skipped bit-identical; determinism-check 7/0/3 byte-identical; clippy (workspace -D warnings) clean.

Regression caught & fixed during impl: example 14 nested-call arg crashed build_acfg in the first cut; fixed with ArgBinding::Nested (faithful representation). No e2e regression.
<!-- SECTION:FINAL_SUMMARY:END -->
