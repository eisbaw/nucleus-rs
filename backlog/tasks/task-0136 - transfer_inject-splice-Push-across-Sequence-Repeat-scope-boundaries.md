---
id: TASK-0136
title: 'transfer_inject: splice Push across Sequence/Repeat scope boundaries'
status: Done
assignee:
  - '@mark'
created_date: '2026-05-18 03:50'
updated_date: '2026-05-18 08:33'
labels:
  - M2
  - compiler
  - bug
  - critical-path
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0027 surfaced this gap. The current `splice_pushes_for_waits` walks one Sequence's children and inserts a Push only when the Wait's producing data symbol is in the same sequence's `local_producer_idx`. Cross-scope Waits (inside a Repeat body whose enclosing Sequence holds the producer) therefore never trigger a Push insertion.

This is visible in example 02-split-add: host produces `a` and `b` via load_input/load_input_b at the top level, then enters `for i { add(a[i], b[i]) }` on w0. The injected ACFG ends up with two Waits per iteration on w0 but zero Pushes on host. The Petri net's buffer place gets only the consumer-side arc — the producer-side TtoP arc is missing.

The pthreads-sync backend currently masks this by consuming the ACFG directly with shared-memory shortcuts. TASK-0124 (backend consumes EventLists) cannot be done cleanly until this is fixed.

Fix shape: thread a `cross_scope_producer_idx: BTreeMap<DataId, (Vec<*mut ACFGNode>, usize)>` (or equivalent) through the walk so a child sequence can register a Push for the outer scope to splice on emit. Alternatively: do a two-pass — first walk records every Wait and its data symbol; second walk inserts Pushes after each Operation whose `data_out` matches a recorded Wait, irrespective of scope. The two-pass form is cleaner and matches v2's "no clever single-pass" preference.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Producer Operation in outer Sequence + consumer inside a Repeat body yields a Push placeholder on the producer side and a Wait on the consumer side.
- [x] #2 Example 02-split-add (split.sched.nuc) projected to EventLists has matched Push/Wait pairs (one Push on host per declared transfer, N Waits on w0 inside the for loop). Test in tests/petri_to_events.rs upgraded to assert pushes.is_empty() is false.
- [x] #3 Idempotence preserved: re-running inject_transfers does not duplicate the spliced Push.
- [x] #4 All existing acfg_to_petri and petri_to_events tests still green.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Pass A hoist_invariant_waits: lift cross-worker Waits out of plain Repeat bodies when data not produced in body; dedup to single Wait in enclosing seq, tile:=enclosing.
2. Add Pass B splice_pushes_global: seq-idempotent finalizer; whole-symbol Push placement (after producer Op, or after producer-enclosing Repeat when consumer is outside it).
3. Gate both on inner_block_iter_vars.is_empty() (non-blocked path = M2 acceptance / example 02 split). Block-inner per-tile push left to TASK-0149/0150 (cells skipped).
4. Flip deadlock.rs:205 expect_err -> Ok. Strengthen petri_to_events AC#2 (pushes non-empty).
5. just test + just e2e; qa-test-runner + mped-architect before commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ROOT CAUSE (verified by parent-Claude code read, fires 12-13):

splice_pushes_for_waits (transfer_inject.rs:633) only inserts a Push when
the Wait's data is in the SAME sequence's local_producer_idx. Cross-scope
case (producer in outer Sequence, Wait inside a Repeat body) never gets a
Push. Confirmed in example 02-split: load_input/load_input_b produce a,b at
top level; `for i { add(a[i],b[i]) }` Wait sits inside the Repeat body.

DEEPER FINDING (changes the fix shape): acfg_to_petri.rs:237-245 unrolls a
Repeat by walking its body `count` times. buffer_places is keyed by SeqTag
(acfg_to_petri.rs:185). So even WITH a spliced Push, a Wait left INSIDE the
Repeat body unrolls into N wait_seqS transitions all consuming from ONE
seq-S buffer place, while one Push deposits ONE token -> deadlock at
iteration 2. petri.rs arcs are consuming-only (PtoT/TtoP); no read/test arc
to model 'data present, read N times without consume'.

THEREFORE the correct fix is NOT just 'splice a Push'. It is:
  1. For a cross-worker Wait whose data D is loop-invariant w.r.t. an
     enclosing Repeat (D not produced inside that Repeat body), HOIST the
     Wait to immediately before the Repeat in the enclosing sequence
     (dedup to a single Wait, not per-iteration). This generalises the
     existing TASK-0143 HoistSink machinery (currently gated on
     inner_block_iter_vars) to ALL Repeats, not just block-inner ones.
  2. Splice the matching single Push after the producer Operation, even
     across scope (the two-pass finalizer the task body suggested).
  3. Net effect for example 02-split: one Push on host after load_*, one
     Wait on w0 before `for i`, both seq=S; inside the loop NO transfer
     (w0 reads its local copy). Deadlock-free: 1 token, 1 consumer.

This also matches what pthreads-sync multi_worker.rs already does at
runtime (whole-symbol slot, transferred once) — so fixing this CLOSES the
analysis/codegen divergence the MPED review flagged, and unblocks
TASK-0124 (backend consumes EventList) honestly.

ACCEPTANCE SIGNAL: nucleus/compiler/tests/deadlock.rs:205
(e2e_example_02_split_currently_deadlocks_on_wait_without_push) flips from
expect_err to expect Ok(()). Also flip the analogous expect_err in
petri_to_events.rs / boundedness.rs if present.

SCOPE: ~100-150 LOC in transfer_inject.rs (generalise HoistSink trigger
from 'inner_block_iter_var' to 'any Repeat where data is loop-invariant';
add cross-scope Push finalizer). TASK-0139 and TASK-0149 are the SAME fix
viewed from boundedness/nested-sequence angles — implement together, close
all three. Labelled critical-path; this is the M2 deliverable, blocks
TASK-0034 (M2 acceptance) and TASK-0036 (second backend).

IMPLEMENTED (parent-Claude, verified). Two post-passes in transfer_inject.rs: Pass A hoist_invariant_waits (lift loop-invariant cross-worker Waits out of plain Repeat bodies, dedup by (src,dst,data), tile:=enclosing) and Pass B splice_pushes_global (seq-idempotent whole-symbol Push placement: after producer Op, or after the producer-enclosing Repeat when the consumer is outside it). Gated on inner_block_iter_vars.is_empty().

Review fixes applied: mped-architect found Pass B (src,dst,data) skip key would suppress a second distinct same-symbol/same-dst transfer in a different scope -> removed; seq alone is the correct idempotence key (Pass A collapses regenerated duplicates before Pass B). Added structural-idempotence test (once==twice==thrice) on the ungated Repeat path + example-02-shape matched-pair test. P2/P3 findings filed as TASK-0151/0152/0153.

Verified: 315+ compiler tests pass, strict clippy clean, e2e 7/7 required PASS (02-split bit-identical). deadlock.rs:205 flipped expect_err->Ok. petri_to_events AC#2 strengthened (pushes non-empty).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fix cross-scope Push/Wait pairing in transfer_inject (the M2 deliverable).

Problem: the single-pass injector only rendezvoused a Push with a Wait inside one Sequence. A consumer inside a `for` loop reading data produced outside it (example 02-split: load_input on host, add on w0 inside `for i`) got a Wait with no Push; since acfg_to_petri unrolls Repeat bodies against a single seq-keyed buffer place, the net deadlocked.

Fix: model whole-symbol transfer. Pass A hoists loop-invariant input Waits out of the loop (one crossing, not one per iteration); Pass B places the matching Push after the producer, or after the producer-enclosing loop when the consumer reads the loop output afterwards (e.g. c read by save_output). Gated on no-block-transform; block=N per-tile cross-scope Push remains TASK-0149/0150 (those cells are skipped).

User impact: example 02-split is now deadlock-free and bounded by construction; the analysis path agrees with codegen, unblocking TASK-0124 (backend consumes EventList) and TASK-0034 (M2 acceptance).

Tests: deadlock.rs e2e flipped to expect Ok; petri_to_events asserts Pushes present and matched; new structural-idempotence + matched-pair tests on the ungated path. 315+ unit tests, strict clippy, 7/7 e2e all green. Review: qa-test-runner GO; mped-architect GO after P1 (idempotence proof) closed and latent multi-consumer bug fixed; P2/P3 filed as TASK-0151/0152/0153.
<!-- SECTION:FINAL_SUMMARY:END -->
