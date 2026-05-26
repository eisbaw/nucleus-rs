---
id: TASK-0343
title: >-
  Cross-worker array-output accumulator: host combine is last-write-wins instead
  of element-wise sum (TASK-0044.04 cycle-186 AC#3 surface)
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-26 16:59'
updated_date: '2026-05-26 23:39'
labels:
  - compiler-bug
  - M6
  - codegen
  - accumulator
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Filed cycle 186 in response to TASK-0044.04 AC#3 empirical probe: distributed schedule on 08-histogram lowers cleanly across all 4 tier-1 backends but produces output.bin = [4,4,4,4,...,4,4] (sum=64) instead of the reference [25,25,24,24,24,23,23,9,9,10,10,10,10,10,10,10] (sum=256). The output is the last worker's STANDALONE partial histogram; the host combine code emits 'histogram = slot_N.wait()' for each N in 0..NUM_WORKERS sequentially — last write wins.

Algorithm-level diagnosis: histogram[b] is an accumulator over (i, b) where i is the partition variable but b is independent. Every worker w writes histogram[0..BINS] independently; the cross-worker fan-in must SUM element-wise per bin. Contrast 03-reduction's partials[w] — there w IS the partition variable, each worker owns one slot, and cross-worker combine is just concatenation (each slot filled by exactly one worker).

This is a substantively new ACFG / codegen shape vs the disjoint-write reductions in 03-reduction. The fix likely sits in transfer_inject / acfg / sync_inject — the cross-worker combine for an LHS-accumulator pattern where the LHS index is NOT the partition variable must materialise as an element-wise reduce, not a sequence of overwrites.

Cross-references:
- nuc-nucleus/examples/08-histogram/schedules/distributed.sched.nuc (the cell that surfaced the gap, committed cycle 186).
- 03-reduction/distributed.sched.nuc (the contrasting disjoint-write shape).
- 04-prefix-sum/prog.algo.nuc 'block_off' (a similar 'masked accumulator' shape, but single-worker so cross-worker combine never fires).
- TASK-0258 partition_rows (the partition-axis infrastructure that already routes input transfers correctly).
- memory project-cross-backend-differential (the bit-identical-across-backends invariant the fix must preserve).

Honest scope: this is a generalisation of the partial-combine machinery; the simplest fix would be to inject an explicit cross-worker reduce pass when the accumulator's LHS index is independent of the partition variable. The harder version would generalise the cross-worker combine to ANY associative algebraic-identity-bearing accumulator (sum, min, max — picking the right identity from kernel attributes). Cycle-186 scope: file the gap precisely; whoever lands the fix can pick the depth.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Algorithm shape that triggers the gap: rectangular LHS-indexed accumulator where the LHS index is NOT the partition variable. Concrete case: 08-histogram has 'histogram[b] <-- bin_inc(histogram[b], input[i], b)' with 'loop i : partition=workers' — every worker writes to histogram[0..BINS] independently; cross-worker combine MUST sum element-wise but emits 4 sequential overwrites in 08-histogram/distributed.sched.nuc currently
- [ ] #2 Compiler must distinguish (a) DISJOINT-write accumulator (03-reduction shape: 'partials[w]' where w IS the partition variable — each worker owns ONE slot; cross-worker combine is concatenation) from (b) OVERLAPPING-write accumulator (08-histogram shape: 'histogram[b]' where b is INDEPENDENT of partition variable — every worker writes the full output array; cross-worker combine is element-wise reduce by the accumulator's algebraic identity)
- [ ] #3 Bit-identical PASS for at least one tier-1 backend on 08-histogram/distributed: cell 'nuc-nucleus/examples/08-histogram' × 'distributed' × pthreads-sync (or whichever tier-1 path lands first) produces output.bin matching reference.bin (committed cycle 186)
- [ ] #4 Cross-backend differential: same cell PROMOTED to [[required]] in nuc-nucleus/e2e-matrix.toml across all 4 tier-1 backends bit-identical when the codegen lands
- [ ] #5 Symptom pin (regression-prevention test): the cycle-186 mismatch shape was output = [N/NUM_WORKERS] * BINS = 16 copies of N/(NUM_WORKERS*BINS-uniformity-per-partition) — i.e. one worker's standalone histogram. Add a per-backend negative test that bites if the host combine ever regresses to last-write-wins for an OVERLAPPING-write accumulator
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Plan (cycle 189 start)

Approach: codegen-side detection at the shared backend-common layer (Option B from the cycle-186 filing's 'simplest fix' branch). Concretely:

1. New helper `collect_accumulate_waits` in `backend-common/src/multi_worker_walker/collect.rs`:
   - Scan a worker's Event list (recursing into Loop bodies).
   - Group Waits by DataId.
   - For each data with N>=2 Waits where every tile is whole-array (empty OR every consulted axis covers full source range — same predicate `wait_slice` returns None for), emit (data, seq) into the accumulate set.

2. Extend `render_wait_assign` (and `wait_slice` indirectly via a new dispatch arm) to consume the accumulate set:
   - When (data, seq) is in the set, emit element-wise wrapping_add of `_tmp` into `name` (size = product of dims).
   - For non-integer scalar types: return EmitError naming this task as the unsupported-scalar carve-out (sum identity is well-defined only for integer types in v2 today; float bit-identity per PRD §10.1 is a separate concern).

3. WalkerCtx + render_worker_body: compute the accumulate set once per worker and thread it through (pthreads-sync, pthreads-async, mp-tcp-event). mp-tcp-bufsync's plan/events.rs caller also computes per-worker accumulate set and passes it to the shared render_wait_assign.

4. Regression test (AC#5): a per-backend test that bites if host-side combine ever regresses to last-write-wins for an overlapping-write accumulator. Pin the cycle-186 mismatch shape symbolically — assert the host emit contains element-wise accumulate, NOT bare `name = slot_N.wait();` for >=2 Waits.

5. Promote 08-histogram/distributed cells from [[skip]] to [[required]] in nuc-nucleus/e2e-matrix.toml across all 4 tier-1 backends. Cross-backend differential check (already a baseline gate) must stay green.

Honest scope:
- Sum identity (wrapping_add) is HARDCODED. Min/max/AND/OR accumulators are NOT supported by this slice. Filed as follow-up.
- Float/bool accumulators NOT supported. Float would also collide with PRD §10.1 bit-identity invariant (sum order). Filed as follow-ups.
- Detection is heuristic at the codegen layer: N>=2 whole-array Waits per data. No cross-check against the algorithm-level accumulator pattern (LHS appears in RHS). For 08-histogram this is observationally equivalent; an exotic schedule that emits multiple whole-array pushes for non-accumulator semantics would mis-combine. Filed as follow-up to add an algorithm-level cross-check.
- Iterative (Repeat-body) accumulator patterns are out of scope: Waits inside Loop bodies carry per-iter tiles (not whole-array), so the heuristic does not fire on them today. The current scope addresses precisely the top-level fan-in case 08-histogram/distributed exercises.

## Cycle 189 close — parallel review gate green, P3.2 in-cycle hardening, P3.1/P3.3 follow-ups filed

**Cycle 189 (orchestrator-led, in-thread per memory feedback-spawned-agents-refuse-code-edits)** landed AC#1-#5 of TASK-0343:

### What landed
- New helper backend-common/src/multi_worker_walker/collect.rs::collect_accumulate_waits — structural detection of N>=2 whole-array Waits per data (the overlapping-write fan-in pattern AC#1 + AC#2 specify).
- backend-common/src/multi_worker_walker/wait.rs::render_wait_assign extended with accumulate: bool param; new render_accumulate_assign + accumulate_op_for_scalar helpers emit element-wise wrapping_add for integer scalars OR typed EmitError::ContractGap for float/bool (sum identity is undefined / PRD §10.1 unsafe).
- backend-common/src/multi_worker_walker/ctx.rs: WalkerCtx.accumulate_waits + WalkerCtx::empty_accumulate_set() helper.
- All 4 tier-1 backends' Plan::build compute the per-(worker, data, seq) accumulate set; all 4 Wait emit sites consume it (3 via shared event walker, mp-tcp-bufsync direct in plan/events.rs).
- nuc-nucleus/e2e-matrix.toml: 4 [[skip]] → [[required]] promotions for 08-histogram/distributed × {pthreads-sync, pthreads-async, mp-tcp-bufsync, mp-tcp-event}.
- nucleus/backend-common/tests/wait_assign_accumulate.rs (NEW): 5 tests pinning AC#5 (the cycle-186 symptom shape).
- 3 existing tests (wait_assign_slice.rs, multi_worker_reuse_marker.rs, multi_worker_blocked_rebind.rs) updated to pass the new WalkerCtx field via empty_accumulate_set().

### AC coverage
- AC#1: classify the overlapping-write pattern — collect.rs::collect_accumulate_waits; tests/wait_assign_accumulate.rs::accumulate_emit_replaces_overwrite_for_array_fan_in covers detection + emit end-to-end.
- AC#2: distinguish disjoint-write from overlapping-write — tests/wait_assign_accumulate.rs::accumulate_detector_skips_disjoint_slice_paste pins the 03-reduction shape (per-worker slice tiles) does NOT classify; existing WaitSlice::Flat / Rows arms unchanged.
- AC#3: bit-identical PASS for at least one tier-1 backend on 08-histogram/distributed — verified across all 4 tier-1 backends (cross-backend bit-identical, exceeds AC#3 minimum).
- AC#4: cross-backend differential — all 4 cells PROMOTED to [[required]] in e2e-matrix.toml.
- AC#5: regression-prevention test — tests/wait_assign_accumulate.rs (5 tests; includes explicit symptom-pin asserting pre-cycle-189 'histogram = slot_N.wait();' shape MUST NOT re-appear).

### Gate numbers (architect+QA review gate, all green)
- cargo build --workspace: OK
- cargo clippy --workspace --all-targets -- -D warnings: 0 warnings
- cargo test --workspace (dev): 984/0/3 PASS (pre-cycle was 961/0/3 per memory — +23 includes 5 new + ~18 walker subtree growth)
- cargo test --workspace --release: 983/0/3 PASS
- cargo run --release --bin nucleus-e2e: 120/110/0/10/0 (PRE-CYCLE 112/101/0/11/0 → +8 total / +9 pass / -1 skipped; required-fail: 0)
- just check-textual-replace-on-codegen: OK
- just check-include-str-coverage: OK
- 08-histogram/distributed × {pthreads-sync, pthreads-async, mp-tcp-bufsync, mp-tcp-event}: ALL FOUR BIT-IDENTICAL vs reference.bin (orchestrator-self-run + QA-subagent-verified)

### Parallel review gate findings (P3 only — GO from both reviewers)
- **P3.1 (architect, filed TASK-0343.05)**: mp-tcp-bufsync/src/walkers.rs:370-394 silently shadows backend-common's collect_pre_init_sets — pre-existing duplicate, not introduced cycle 189, but matches the feedback-silent-sibling-defect class. Filed.
- **P3.2 (architect, FOLDED IN CYCLE)**: collect_accumulate_waits .unwrap_or(true) on missing pair_tiles entry → silent behaviour-change risk. Tightened to .unwrap_or(false) at collect.rs (conservative default: no evidence of whole-array → do NOT classify as accumulate; structurally unreachable for shipped schedules; pre-cycle-189 fallback behaviour preserved on the unreachable branch).
- **P3.3 (architect, filed TASK-0343.01 + .02 + .03 + .04)**: file follow-up tasks so ContractGap text points to live task IDs, not textual cookies. Filed 4 follow-ups (algebraic identity from kernel attribute, float/bool support, algorithm-level cross-check, iterative/Repeat-body accumulator); also updated wait.rs ContractGap messages to reference TASK-0343.01 / .02 explicitly.

### Honest scope / limitations (preserved from cycle-189 implementation plan; now backed by filed follow-ups)
- Sum identity hardcoded (wrapping_add). min/max/AND/OR/XOR → TASK-0343.01.
- Integer scalars only; float/bool → typed EmitError → TASK-0343.02.
- Structural detection only (no algorithm-level cross-check) → TASK-0343.03.
- Top-level fan-in only (in-Loop Waits carry per-iter tiles, naturally excluded) → TASK-0343.04.
- mp-tcp-bufsync silently-duplicates collect_pre_init_sets (pre-existing) → TASK-0343.05.

### Forward-carried lessons for follow-up implementers
- The 'overlapping-write' vs 'disjoint-write' distinction is structural at the codegen layer: whole-array tiles vs slice tiles. Detection in collect.rs is by predicate is_whole_array_tile, which mirrors wait_slice's None-arm condition (sibling file wait.rs). Any change to wait_slice's None-arm logic MUST mirror into is_whole_array_tile or the two will diverge silently.
- All 4 tier-1 backends share the helper at backend-common — fix-once, consume-everywhere is structurally guaranteed (not 4 parallel implementations). A 5th backend joining the matrix needs to (a) build the per-worker accumulate set in its Plan::build, (b) thread via WalkerCtx (if walker-using) or consult directly at its Wait emit site (mp-tcp-bufsync precedent in plan/events.rs).
- The 'L >= 2 + all whole-array' predicate is INTENTIONALLY conservative (cycle-189 P3.2 hardening): forward-compatibility favoured over an aggressive default. A more aggressive (algorithm-level cross-check) detector is TASK-0343.03.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 189 closed all 5 ACs.

**Compiler-bug fix**: overlapping-write accumulator fan-in (08-histogram/distributed shape: every worker pushes the FULL output array, host receives N>=2 whole-array Waits) now emits element-wise wrapping_add accumulate into the pre-initialised destination, NOT the pre-fix last-write-wins overwrite sequence.

**Implementation**: shared backend-common helper (collect_accumulate_waits + extended render_wait_assign) consumed by all 4 tier-1 backends (pthreads-sync, pthreads-async, mp-tcp-event, mp-tcp-bufsync). All 4 08-histogram/distributed cells PROMOTED to [[required]] and bit-identical against reference.bin.

**Gate**: dev 984/0/3, release 983/0/3, e2e 120/110/0/10/0 (no regressions; +9 pass / -1 skipped vs cycle-188 baseline 112/101/0/11/0), clippy 0-warn, structural checks OK. Parallel review gate (qa-test-runner + mped-architect) GO; architect P3.2 (.unwrap_or(true) → .unwrap_or(false)) folded in-cycle.

**Scope LIMITS** (filed as 5 follow-ups TASK-0343.01..05): sum identity only; integer scalars only; structural-only detection; top-level fan-in only; mp-tcp-bufsync local collect_pre_init_sets duplicate.

**Forward-carried lessons** (in Implementation Notes): is_whole_array_tile MUST stay in sync with wait_slice's None arm; the predicate is intentionally conservative for forward-compat; cross-backend bit-identity is structurally guaranteed by the shared helper.
<!-- SECTION:FINAL_SUMMARY:END -->
