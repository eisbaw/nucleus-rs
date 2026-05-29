---
id: TASK-0281
title: >-
  sync_inject: Sequence-boundary skip inside partition is unconditional — add
  participants-aware guard for M6+ cross-partition reducers
status: Done
assignee:
  - '@claude'
created_date: '2026-05-24 14:16'
updated_date: '2026-05-29 22:23'
labels:
  - M6
  - compiler
  - sync_inject
  - tech-debt
  - forward-carried-from-TASK-0268
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as TASK-0268 cycle-102 architect P1 forward-carry ===

The cycle-102 fix to TASK-0268 introduced an unconditional skip of the
Sequence-boundary Sync rule inside a partitioned scope (sync_inject.rs
`inject_in_sequence`, the `if !inside_partition { ... }` guard). The
skip is sound for ALL shipped schedules today because:

1. PRD §6.2.1 single-assignment holds — no cross-iteration data
   dependency.
2. Every cross-worker dataflow edge crossing the partitioned-loop
   boundary is covered by the TASK-0117 fan-out Push/Wait pairs.
3. The shipped reduction (03-reduction/distributed) lives OUTSIDE
   the partitioned scope, not nested inside it.

The architect's cycle-102 P1 review flagged that a future M6+ schedule
could violate assumption (2): a cross-partition reducer (an inner
Sequence writing to a shared output region placed on a worker set
DIFFERENT from the partition's inner-body worker set, NOT covered by
the Push/Wait pair) would silently lose synchronisation.

## Acceptance criteria

1. **Discovery trigger**: when an M6+ schedule exercises a
   cross-partition reducer (inner Sequence writing to a shared
   region on different workers than the partition's inner body),
   make the skip conditional.
2. **Fix shape (option D from cycle-85 analysis)**: replace the
   unconditional `if !inside_partition { ... }` with a check that
   evaluates whether the Sequence boundary is ALREADY covered by a
   Push/Wait pair OR by the partitioned scope's once-per-iteration
   semantics. Equivalent to extending `push_wait_pair_covers` to
   "or partition-scope already provides equivalent synchronisation".
3. **Test**: add a fixture that lowers a synthetic cross-partition
   reducer schedule; assert sync_inject inserts the necessary
   Sync (or, equivalently, Petri net deadlock-checker passes).

## Dependencies

- Trigger: an M6 or later schedule that exercises a cross-partition
  reducer pattern. The 13-cnn-inference/batch_parallel cell is a
  candidate to inspect — if its reduction phase is partition-nested
  rather than placed OUTSIDE the partition.

## Honest scope

- This is a LATENT defect / envelope limit, NOT a current
  regression. The cycle-102 unconditional skip is sound for all
  currently-shipped schedules. The follow-up exists so a future
  silent-deadlock under M6+ schedules is caught by the existing
  architectural check, not by debugging.

- The code comment at `nucleus/nucleus-compiler/src/passes/
  sync_inject.rs:356-388` (post-cycle-102) cross-references this
  task as the trigger.

## Forward-carry context

- Memory: feedback-opacity-gate-rot (cycle-101 lesson — gates
  that predate newer machinery can quietly become wrong). This
  task is the converse precedent: a gate-removal that may need a
  partial restoration when newer schedules land.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Reachability finding: (b) IR/ACFG-constructible only, NOT expressible as a sound .nuc schedule today. A coherent cross-partition reducer inside a partitioned loop would need body ops on disjoint worker subsets; apply_partition_workers takes the UNION as body_workers and splits the range disjointly across all of them, mangling the reducer. No shipped example reaches it (13-cnn batch_parallel writes disjoint output[n], all body ops on the same {w0..w3}).

Fix shape (AC#2): convert the UNCONDITIONAL inside_partition skip into a FAIL-LOUD typed diagnostic. inject_syncs gains a Result<ACFG, SyncInjectError> return. When inside_partition AND a Sequence boundary has cross-worker write->read (w1 != w2, >=2 participants) NOT covered by push_wait_pair_covers, refuse with SyncInjectError::UncoveredCrossPartitionReducer (diagnosis-quality message naming the participant workers + TASK-0281 forward-link + the deferred option-D follow-up). This turns silent loss-of-sync into a loud compile error; strictly safer than silent miscompile, lower-risk than untested conditional-sync. Reserve panic! for genuinely-unreachable; this is a valid-input refusal so it gets a typed Error per panic-not-diagnostic discipline.

Deferred (AC#1 intent / AC#2 option D): the participant-correct conditional-Sync (partitioned_scope_covers) stays explicitly deferred behind a new follow-up task depending on TASK-0281, because a participant-safe in-partition barrier is genuinely hard (floor-with-spillover short-barrier deadlock per TASK-0268) and no schedule needs it.

Call-site impact: driver/src/main.rs:508 gains .map_err; pthreads-async + pthreads-sync test helpers + test-common gain .expect(); the e2e baseline (308/246/0/62/0, captured live) MUST hold byte-identical because shipped partitioned bodies are all single-worker-set (w1==w2, detection inert).

Test (AC#3): tests/sync_inject.rs synthetic fixtures — (1) cross-partition reducer shape (partitioned Repeat body, op_A on {w0,w1} writes D, op_B on {w2,w3} reads D, sidecar keyed on outer iv) asserts SyncInjectError fires; (2) the existing benign partitioned shape (all body ops one worker set) still returns Ok with zero body syncs (codegen-inert pin). Run full gate; re-run e2e >=2x for determinism.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED cycle (TASK-0281). Reachability finding: (b) IR/ACFG-constructible only, NOT a sound .nuc schedule today (apply_partition_workers takes the UNION of body worker sets and splits the range disjointly across all, so a coherent disjoint-subset cross-partition reducer is not expressible as a sound schedule; verified 13-cnn batch_parallel has all body ops on the same {w0..w3}, w1==w2, boundary rule never applies).

Fix shape (AC#2): converted the UNCONDITIONAL inside_partition Sequence-boundary skip into a FAIL-LOUD typed diagnostic. inject_syncs now returns Result<ACFG, SyncInjectError>. The new variant UncoveredCrossPartitionReducer fires when inside_partition AND a Sequence boundary has cross-worker write->read (w1 != w2, >=2 participants) NOT covered by push_wait_pair_covers. Chose fail-loud over option-D conditional-Sync because a participant-correct in-partition barrier is genuinely hard under floor-with-spillover unequal iter counts (TASK-0268 short-barrier deadlock) and NO schedule needs it; refusing is strictly safer than silent miscompile and lower-risk than untested conditional-sync. panic-not-diagnostic discipline honoured (typed Error, not panic!).

EMPIRICAL CORRECTION (cheap-verification discipline): my first test fixture had op_B read the SAME symbol op_A wrote -> push_wait_pair_covers returns TRUE -> guard correctly did NOT fire -> test FAILED. The real dangerous shape requires push_wait_pair_covers FALSE: op_B reads an UNRELATED symbol (no shared dataflow) or a non-bare neighbour. Corrected the fixture (op_A writes data 0 on {w1,w2}; op_B reads UNRELATED data 9 on {w3,w4}). Added a complementary precision pin proving the COVERED shared-symbol edge still lowers Ok (guard is NOT over-broad).

Deferred: option-D participant-correct conditional-Sync filed as TASK-0365 (depends on TASK-0281); referenced in code at the else-if guard site + variant doc + error message.

Call-site impact: driver/src/main.rs gains .map_err; ~45 test call sites + test-common + pthreads-async test helper gain .expect("inject_syncs"). Module idempotence doc + tests/sync_inject.rs total-pass doc lie both corrected.

Gate (all inside nix dev shell): build OK; clippy clean; just test DEV 1120 passed/0 failed/3 ignored; just test-release 1119 passed/0 failed/3 ignored (1-test dev/release delta is the expected debug_assert should_panic divergence); e2e 308/246/0/62/0 on 3 consecutive samples == pre-change live baseline (codegen-inert confirmed). check-textual-replace + check-include-str both OK.

AC status: AC#1 (discovery trigger) DISCHARGED-BY-SAFETY (latent case now loud regardless of trigger; literal trigger needs an M6+ reducer schedule = TASK-0365). AC#2 (fix shape) MET (fail-loud typed diagnostic, justified). AC#3 (test) MET (3 new synthetic fixtures: refusal fires on uncovered shape, covered shape lowers Ok, single-worker-set body lowers Ok + zero syncs; plus an outside-partition control).

CYCLE CLOSE (orchestrator review gate, both reviewers GO). Implementation commit 8ca0cd5; review fold-back eacb86b.

REVIEW GATE RESULT:
- qa-test-runner GO: build OK; clippy clean (-D warnings, no doc_lazy_continuation/dead_code/unused/unreachable_pub from the Result signature change); just test (dev) 1120/0/3; just test-release 1119/0/3 (delta = the documented TASK-0291 should_panic profile skew, not a hidden divergence); e2e 308/246/0/62/0 byte-identical x3 (codegen-inert confirmed); check-textual-replace-on-codegen + check-include-str-coverage PASS; all 4 new sync_inject fixtures run+pass (none ignored/filtered).
- mped-architect GO: no P1/P2. Verified guard fires on exactly the right set (mirrors the !inside_partition predicate; recursion visits every Sequence boundary at every depth inside a partition); reachability-(b) confirmed against partition_workers.rs:336-365 (apply_partition_workers takes the UNION of body op workers and splits the range disjointly across that union, so a coherent disjoint producer/consumer-subset reducer is NOT expressible as a sound .nuc schedule today); push_wait_pair_covers is conservative in the SAFE direction; the 2 non-test .expect() sites are both test-only (pthreads-async mod tests + lower_for_test helper) so no panic-on-valid-input production path; no stale TASK-0366 reference remains.

P3 fold-backs applied in-thread (commit eacb86b): P3.1 stale function-level idempotence docstring (was literally a type error post-Result -> reworded to Ok-subset form); P3.2 documented in Honest limitations that the fail-loud guard is Sequence-boundary-only while Repeat entry/exit remain silently elided inside a partition (sound today; lift alongside TASK-0365). P3.3a oracle-reaudit note appended to TASK-0365.

P3.3b HONESTY CORRECTION (architect, accepted): AC#2 was literally specified as option D (push_wait_pair_covers || partitioned_scope_covers). The shipped fix is a DIFFERENT, SAFER shape (fail-loud typed SyncInjectError on the uncovered case). So AC#2 is met by a safer alternative, NOT by the literal option-D wording -- the participant-correct option-D conditional-Sync is deferred to TASK-0365 (dependency edge in place). This is a deliberate, defensible scope split (a participant-correct in-partition barrier is hard under TASK-0268 floor-with-spillover unequal iter counts, and no shipped schedule needs it), recorded honestly rather than AC-gamed.

GOTCHA (forward lesson): the dangerous cross-partition-reducer shape requires push_wait_pair_covers to be FALSE (consumer reads an UNRELATED symbol, or a non-bare neighbour). A first-draft fixture that had the consumer read the SAME written symbol made the oracle return true => guard correctly did not fire => test failed. Any future test/fix in this area must construct the genuinely-uncovered edge, not a covered one.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE via fail-loud hardening (not the literal option-D conditional-Sync). The previously-UNCONDITIONAL inside_partition skip of the Sequence-boundary Sync rule (sync_inject.rs) now refuses an uncovered cross-partition cross-worker write->read boundary with a typed SyncInjectError::UncoveredCrossPartitionReducer instead of silently dropping the barrier; inject_syncs returns Result<ACFG, SyncInjectError>, propagated through driver + ~45 call sites. Reachability finding (b): the shape is IR/ACFG-constructible (the 4 new regression fixtures) but NOT expressible as a sound .nuc schedule today (apply_partition_workers unions body-op workers then splits disjointly). Chose fail-loud over a participant-correct in-partition barrier because (1) no shipped schedule reaches the gap and (2) an in-partition Sync short-barriers under TASK-0268 floor-with-spillover unequal iter counts -- panic-not-diagnostic discipline: typed EmitError, not panic!. AC#1 discharged-by-safety (latent case now loud regardless of trigger; literal M6+ trigger captured by TASK-0365). AC#2 met by safer alternative fix shape (option-D deferred to TASK-0365). AC#3 met (4 fixtures: refusal-fires / covered-edge-still-Ok / single-worker-set-body-Ok / outside-partition-control). Both review gates GO; gate green e2e 308/246/0/62/0 x3 codegen-inert. Commits 8ca0cd5 + eacb86b. Follow-up TASK-0365 (participant-correct option-D, depends on this).
<!-- SECTION:FINAL_SUMMARY:END -->
