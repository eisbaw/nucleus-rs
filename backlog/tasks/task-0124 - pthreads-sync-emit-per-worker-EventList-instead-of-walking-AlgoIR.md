---
id: TASK-0124
title: 'pthreads-sync: emit per-worker EventList instead of walking AlgoIR'
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 02:13'
updated_date: '2026-05-18 22:00'
labels:
  - M2
  - backend
dependencies:
  - TASK-0159
  - TASK-0160
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0020 codegen walks AlgoIR statements directly because the ACFG strips index expressions and the per-worker EventList (Fire/Alloc/Push/Wait/Sync/Free) is not yet produced (waits on TASK-0027). Once TASK-0027 lands, the backend should consume per-worker EventLists rather than the AlgoIR. This unifies tier-1 backends around the EventList contract (PRD §7.4 / §8.3) and eliminates the LinkedIR dependency from the emit signature.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 emit() signature changes to (per_worker_event_lists: BTreeMap<WorkerId, Vec<Event>>, kernels_rs_path, out_dir, sidecar_name_map).
- [ ] #2 Codegen no longer references AlgoIR/LinkedIR; only Event-typed input.
- [ ] #3 All existing tier-1 backends agree on this contract before M3 lands.
- [ ] #4 Depends on TASK-0027.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
INVESTIGATION COMPLETE — gap is LOOP STRUCTURE, not value bindings.

Finding: acfg_to_events UNROLLS every ACFGNode::Repeat into N copies of Fire (proven by test repeat_unrolls_in_event_list; petri_to_events.rs module docs explicitly state this M2 trade). The current pthreads-sync backend emits ROLLED loops (verified: /tmp/nuc_master_05 main.rs line 12 `for y in (1_i64)..((16_i64-1_i64))`). An EventList-only walk would emit 196 unrolled blur3 statements for ex05 / 256 for ex01 -> NOT byte-identical to master, and emit.rs unit test asserts the rolled `256_i64` bound. TASK-0156 closed the VALUE gap (FireBinding); the REMAINING gap for byte-identical EventList-only codegen is that the Event contract discards loop nest structure (Repeat range + iter_var) and data types/const values needed for pre-init + slot typing.

DECISION: Implement AC#1 (signature change to EventList + NameSidecar) and AC#2 (codegen consumes only Event-typed input) ONLY IF byte-identical is preservable. It is NOT under the current Event contract without either (a) re-rolling unrolled Fires (lossy/heuristic, fragile, smuggled loop inference) or (b) extending the Event contract to carry loop structure (out of TASK-0124 scope; that is a contract change like TASK-0156 was).

PLAN:
1. Build current master generated trees for 01/02/03/05/07 into /tmp (DONE for 01/02/05) as the byte-identical oracle.
2. Confirm gap precisely: also inspect 03/07 + the data-type/const/loop-bound info the EventList lacks (types for vec![0;N] pre-init, ScalarType casts, symbolic loop bounds, const values). The EventList carries NONE of these.
3. Honest determination: TASK-0124 AC#2 (codegen references ONLY Event + sidecar, no AlgoIR/LinkedIR) is NOT achievable byte-identically without a loop-structure + type/const extension to the Event/sidecar contract. Per the brief and CLAUDE.md (no workarounds, no fake success): do NOT smuggle AlgoIR back nor heuristically re-roll.
4. File precise follow-up task(s) for the missing contract pieces (loop structure on Event or sidecar; data-type/const sidecar). Reference them in code + notes.
5. Land what IS safely landable: introduce NameSidecar struct + change emit() signature to additionally accept per_worker EventList + sidecar WITHOUT removing the AlgoIR walk yet (so AC#1 infra lands, determinism/e2e/byte-identical preserved), with an honest in-code note that AC#2 full switch is blocked on the filed contract task. OR, if even the signature change cannot be done without faking AC#2, leave TASK-0124 In Progress with the precise blocker. Decide after step 2.
6. Gates before any commit: just test, just e2e (7/0/3), just determinism-check (7/0/3), determinism-check-negative, clippy -D warnings. Preserve NUC_NONDET_TEST nonce.
7. Update the stale BLOCKED note on TASK-0124; set TASK-0156 AC#3 status honestly.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
STALE "BLOCKED" note (value-binding gap) is now SUPERSEDED: TASK-0150 (index exprs through ACFG) and TASK-0156 (Event::Fire carries FireBinding) ARE landed on master. The VALUE gap that note described is closed — proven by eventlist_alone_reconstructs_stencil_kernel_call (rebuilds the exact blur3 call from the EventList ALONE). However, full-pipeline investigation (this session) found a SECOND, distinct blocker the original note did not identify: LOOP STRUCTURE + TYPES/CONSTS, not values.

INVESTIGATION (this session, against master, gates verified green 7/0/3 before/after — NO code changed):

Built master generated trees for 01/02/03/05/07 into /tmp/nuc_master_NN as the byte-identical oracle. Master pthreads-sync emits ROLLED loops with symbolic bounds:
- ex01: `for i in (0_i64)..(256_i64)`
- ex05: `for y in (1_i64)..((16_i64 - 1_i64))` (note: H-1 kept UNEVALUATED — lives only in AlgoIR loop bounds; ACFG Repeat.range is already 1..15)
- ex03/07: nested rolled `for w/i`, `for i/j/k`
- ex02 (multi-worker): rolled loops on BOTH host & w0 + Slot<Vec<i32>> typing + barrier-in-loop; multi_worker.rs DELIBERATELY ignores ACFG Xfer nodes and synthesises from LinkedIR data_producers/consumers (documented cross-scope Push/Wait imbalance in petri_to_events.rs).

acfg_to_events UNROLLS every ACFGNode::Repeat into N flat identical Fire copies (test repeat_unrolls_in_event_list; petri_to_events.rs + acfg_to_petri module docs state this M2 trade explicitly). Therefore an EventList-only walk emits 256 / 196 / etc UNROLLED kernel statements, NOT the rolled for-loop -> NOT byte-identical to master, AND emit.rs::main_rs_calls_every_kernel asserts the rolled `256_i64` bound (would fail).

Additionally the per-worker EventList + the proposed NameSidecar carry NO data ResolvedType (needed for `vec![0; 256]` pre-init sizing, `Vec<i32>` slot type, scalar-arg casts) and NO const values / unevaluated const bound exprs. DataSlice = DataId + index IrExprs only.

CONCLUSION (honest, per CLAUDE.md no-workaround / no-fake-success): TASK-0124 AC#1+AC#2 as specified (drop acfg/linked from emit(); codegen consumes ONLY Event + sidecar) is NOT achievable byte-identically under the current Event contract. This is the loop-structure/type analogue of the value-binding gap TASK-0156 fixed. Re-rolling unrolled Fires heuristically, or keeping an AlgoIR walk behind a new signature, would be a workaround / fake AC#2 — explicitly refused.

Deliberately did NOT introduce a NameSidecar + dual signature while still walking AlgoIR: that satisfies NEITHER AC#1 (acfg/linked not actually removed) NOR AC#2 (still AlgoIR), churns the API, and risks the byte-identical invariant for zero real AC progress. Honest partial > fake complete.

Filed precise blockers: TASK-0159 (Event contract must carry loop-nest structure / stop blanket unroll) and TASK-0160 (NameSidecar must carry per-DataId ResolvedType + const values). TASK-0124 needs BOTH landed first, THEN the emit() switch is mechanical (the value half is already proven by TASK-0156). Added deps task-0159, task-0160. No code committed (none written); backlog-only changes.

Forward-carried from TASK-0142: the reason this migration is non-trivial — render_main_rs currently emits loops by walking LinkedIR::algo source IrStmt directly (lib.rs ~25-33,78-81), NOT the ACFG/Event stream. acfg_to_petri/petri_to_events unroll Repeat by range length, so the Event stream has no rolled-loop / iter-var / symbolic-bound info (depends on TASK-0159/0160). When EventList-only codegen lands, block=N tiling (TASK-0142) will need to become visible in emitted code: today 05-stencil/blocked passes only because single-worker codegen ignores the tiled ACFG and the result is schedule-independent. A correct EventList path must emit the (tile-loop, intra-tile-loop) nest INCLUDING the trailing partial tile (static Sequence[full-nest, partial-tile] shape produced by block_transform) — verify the per-worker EventList projection preserves that structure rather than re-flattening it.
<!-- SECTION:NOTES:END -->
