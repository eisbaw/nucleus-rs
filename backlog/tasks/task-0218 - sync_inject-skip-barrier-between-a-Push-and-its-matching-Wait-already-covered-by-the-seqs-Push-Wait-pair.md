---
id: TASK-0218
title: >-
  sync_inject: skip barrier between a Push and its matching Wait already covered
  by the seq's Push/Wait pair
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-21 14:54'
updated_date: '2026-05-22 12:28'
labels:
  - compiler
  - sync-inject
  - M4
  - latent
dependencies:
  - TASK-0213
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0213 cycle): the root reason TASK-0213's path-2 elision was needed at all is that sync_inject currently interposes a barrier between a Push and its matching Wait. The Push/Wait pair already supplies the rendezvous — the extra barrier is over-synchronisation that creates a structural dependency cycle in the analysis net (Push -> barrier -> Wait can't fire because buffer is full -> Push must fire first -> overflow). sync_inject.rs module doc at lines 39-47 acknowledges general over-syncing but does not call out this specific case. If sync_inject elides such barriers, the marking-aware firing-order in boundedness::derive_firing_order resolves example-13 directly, and path-2 elision in acfg_to_petri becomes unnecessary IR scaffolding.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Identify all (Push, Wait) pairs whose Repeat-scope and worker-set match the barrier between them; elide such barriers in sync_inject.
- [x] #2 Verify: with TASK-0218 landed, the path-2 elision in acfg_to_petri::emit_xfer can be reverted; example-13 pipeline_parallel still passes boundedness/deadlock via path-1 marking-aware derive_firing_order alone.
- [x] #3 Forward-carry: if TASK-0218 lands BEFORE TASK-0042.01 ships, the backend's IR view simplifies (no analysis-vs-runtime mismatch); update acfg_to_petri.rs module doc accordingly.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-40 implementation notes

**Option chosen: B (forbid the redundant barrier at injection time).**

Files modified:
- nucleus/compiler/src/passes/sync_inject.rs — added push_wait_pair_covers() helper (~70 lines incl. docstring); guard the Sequence-rule barrier emission with !push_wait_pair_covers(prev, &child). Updated module 'Honest limitations' section to acknowledge the partial fix.
- nucleus/compiler/tests/sync_inject.rs — rewrote sequence_boundary_injects_sync_between_cross_worker_writer_reader (now sequence_boundary_elides_sync_when_push_wait_pair_will_cover; asserts 0 syncs) and sequence_boundary_three_ops_two_syncs (now sequence_boundary_three_ops_two_syncs_all_elided_by_dataflow; asserts 0 syncs). Added a new positive test sequence_boundary_injects_sync_when_no_dataflow_between_ops that pins barrier-kept when shared-symbol condition fails.
- nucleus/backends/pthreads-sync/tests/multi_worker.rs — two_worker_pingpong_compiles_and_runs no longer asserts Barrier::new in the synthetic bare-Operation pingpong fixture (zero barriers post-fix); partial_nonuniform_barrier_multi_worker_lowers_correctly updated to assert 2 surviving barriers ({host,w0} + the critical host-excluding {w0,w1}) instead of 3, with an explicit absence assertion for the {host,w1} barrier.
- nucleus/backends/mp-tcp-bufsync/tests/pingpong.rs — dropped the wire::barrier_cross structural-smoke assertion on the same bare-Operation fixture. Coverage retained at the wire-library unit test level (mp-tcp-common::barrier_cross_two_party) and through real e2e cells (02-split-add__split__mp-tcp-bufsync via its Repeat-entry sync, 05-stencil__blocked__mp-tcp-bufsync).

**Elision condition (push_wait_pair_covers):** both prev and curr are bare ACFGNode::Operation AND data_out(prev) ∩ data_in(curr) is non-empty. The shape-restriction to bare Operations is conservative — for nested prev/curr (Sequence/Repeat) the Push and Wait don't sit immediately around the barrier and the simple elision argument doesn't apply. Repeat entry/exit barriers are also left untouched.

## AC#1 status

DONE. Bare-Operation Push/Wait pairs whose worker-set matches the Sequence-rule barrier between them are now elided. Verified on:
- example-13 pipeline_parallel: bar_2 ({w_stage1,w_stage2} between conv_block_1↔conv_block_2) and bar_3 ({w_stage2,w_stage3} between conv_block_2↔classifier) are elided. 6 barriers → 4 barriers in emitted main.rs.
- example-02 split: unchanged (3 barriers) — the Sequence rules there involve a Repeat as prev/curr.

Scope honest-limit: the elision does NOT cover the cases where prev or curr is a Sequence/Repeat (e.g. the outer Sequence-rule barrier between produce_op and a for-loop body that reads the produced data — bar_0 in pipeline_parallel, bar_0 in 02-split-add). Those barriers would also be over-syncing under TASK-0218's reasoning, but eliding them safely requires walking through the nested structure to confirm Push/Wait coverage is complete. Filed as a follow-up if a real example demands it.

## AC#2 status

DONE. The path-2 TtoP-arc elision in acfg_to_petri::emit_xfer has been reverted: every Push emits a real TtoP arc; the head-start D credits live only on the buffer place's initial marking. Verified:
- e2e_example_13_pipeline_parallel_passes_boundedness_and_deadlock PASSES with path-1 (marking-aware derive_firing_order) alone.
- Full e2e 66/55/0/11 unchanged.
- Determinism positive + negative falsifiers green.
- xbackend negative falsifier green.

Removed: NetBuilder.push_count_per_seq field (and its bookkeeping at emit_xfer). Updated acfg_to_petri module doc 'Path-2 TtoP-arc elision' section to mark it as REVERTED with full provenance. Updated tests/acfg_to_petri.rs e2e_example_13... test comment to reflect path-1-only resolution.

## AC#3 status

DONE for the doc cleanup half (acfg_to_petri.rs module doc updated to reflect path-1-only resolution and 'analysis net IS a one-to-one runtime token trace' post AC#2). The 'forward-carry to TASK-0042.01' clause is moot because TASK-0042.01 shipped first; the simplification is documented anyway for whatever lands next.

## Verification gate

- nix develop -c just test : 596 PASS / 0 FAIL / 3 IGN.
- nix develop -c just clippy : clean.
- nix develop -c just e2e : 66/55/0/11 unchanged.
- nix develop -c just determinism-check : 66/55/0/11.
- nix develop -c just determinism-check-negative : OK (55 of 66 cells perturbed, falsifier bit correctly).
- nix develop -c just xbackend-check-negative : OK (16 mp-tcp cells corrupted, 1 detected by differential, falsifier bit correctly).

## Pre/post emitted main.rs (the prompt asked for byte-identical — IT IS NOT, honest deviation)

The prompt's 'emitted main.rs must remain byte-identical' verification was incompatible with TASK-0218's mandate (ACFG Sync nodes flow straight to Event::Sync flow straight to backend Barrier::wait calls; eliding the Sync at sync_inject IS visible in emitted code). The TRUE invariants (e2e correctness, cross-backend bit-identity, determinism, falsifier bites) are all preserved; the emitted code is slightly LEANER (fewer Barrier constructions) but correctness holds. Pre/post snapshots:
- 13-cnn-inference/pipeline_parallel/pthreads-async: 6 → 4 barriers ({w_stage1,w_stage2} and {w_stage2,w_stage3} elided — exactly the Push→barrier→Wait sandwiches the task description targets).
- 02-split-add/split/pthreads-async: 3 → 3 barriers unchanged (nested prev/curr cases out of TASK-0218 elision scope).

## Honest limits / follow-ups

- TASK-0218 elision is conservative: covers only bare Operation → bare Operation Sequence-rule barriers. Nested prev/curr Sequence-rule barriers and Repeat entry/exit barriers are NOT elided even when their dataflow handoffs are fully Push/Wait covered. A future cycle could broaden the elision after walking nested structures to verify total Push/Wait coverage; not blocked by any current example (e2e + falsifiers green at the current scope).
- TASK-0218 changes the synthetic-fixture barrier coverage in pthreads-sync and mp-tcp-bufsync unit tests. Coverage of Barrier::new emission preserved via partial_nonuniform_barrier_multi_worker_lowers_correctly (host-excluding {w0,w1} barrier still exercised) and real e2e cells.

NOT marking Done. Reviewer gate next.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 40 (2026-05-22) — closed. All 3 ACs met.

**AC#1 (sync_inject elides barriers between Push/Wait pairs)**: Option B chosen — forbid the redundant barrier at injection time. New helper push_wait_pair_covers(prev, child) in nucleus/compiler/src/passes/sync_inject.rs guards the Sequence-rule barrier emission at line ~298. Guard fires ONLY when both prev AND child are bare Operation nodes sharing a dataflow symbol (true producer→consumer edge via dataflow.edges.data_out ∩ dataflow.edges.data_in). Conservative scope: nested Sequence/Repeat boundaries are NOT touched; same-worker and single-party cases are already filtered upstream.

**AC#2 (acfg_to_petri path-2 elision REVERTED)**: TtoP-arc elision in emit_xfer removed; push_count_per_seq bookkeeping deleted; every Push now emits a real TtoP arc — the analysis net is once again a one-to-one trace of runtime token deposits. boundedness::derive_firing_order's marking-aware path-1 resolves example-13 directly without the IR-layer compensation that was needed pre-fix. Module doc honestly names TASK-0213 as the structural compensation that has just become unnecessary.

**AC#3 (forward-carry doc update)**: acfg_to_petri.rs module doc rewritten to explain the path-1-only resolution post-TASK-0218.

Concrete impact: 13-cnn-inference/pipeline_parallel × pthreads-async emit dropped from 6 to 4 Barrier::new constructions:
- ELIDED: {w_stage1,w_stage2} + {w_stage2,w_stage3} — the two Push→barrier→Wait sandwiches between consecutive stages (the targeted over-syncing).
- SURVIVED: 3× 4-party {host,w_stage1..3} entry/exit/output barriers (Repeat boundaries — out of elision scope) + 1× 3-party host-excluding {w_stage1..3} mid-pipeline barrier (TASK-0172 partial-non-uniform; out of elision scope).

Runtime bit-identical to reference.bin oracle (sha256 d893337208d7b46923581ecdea8e326e07e8c7e1204a13d867807d6795f7b861) — confirming the surviving 4 barriers are sufficient and the elided 2 were genuinely redundant.

Gate (cycle 40): just test 596 PASS / 0 FAIL / 3 IGN; just clippy clean; just e2e 66/55/0/11 unchanged; just determinism-check-negative OK PERTURBED=55; just xbackend-check-negative OK APPLIED=16 DETECTED=1.

Three tests reshaped because their synthetic fixtures relied on the over-syncing being there (two_worker_pingpong_compiles_and_runs, partial_nonuniform_barrier_multi_worker_lowers_correctly, mp-tcp pingpong_matches_pthreads_sync_bit_for_bit). Reshaping was strengthening, not weakening — the partial_nonuniform test added explicit NEGATIVE assertions confirming the elided barrier is absent.

Honest limit (filed by implementer): elision is conservative — fires only for Operation→Operation siblings. Nested prev/curr and Repeat entry/exit cases (also over-syncing per the original task description) are NOT touched. Broadening can be a follow-up cycle if a future schedule demands it; today's e2e cells don't.

Review-gate (parallel read-only): both qa-test-runner + mped-architect GO. Architect noted the elision guard is correctly conservative (low regression risk by construction — requires actual dataflow producer→consumer edge), the acfg_to_petri revert is genuine + well-documented, and the test reshapes strengthened (negative assertions on the elided cases) rather than lobotomised. Headline 13/pipeline_parallel bit-identical to oracle is the load-bearing semantic check; passed. Implementer-side correction to my brief: TASK-0218 was NEVER going to preserve byte-identical emit (the whole point is fewer barriers); the TRUE invariant is bit-identical reference.bin runtime, which DID hold.
<!-- SECTION:FINAL_SUMMARY:END -->
