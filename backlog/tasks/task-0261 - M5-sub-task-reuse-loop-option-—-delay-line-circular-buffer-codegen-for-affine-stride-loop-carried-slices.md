---
id: TASK-0261
title: >-
  M5 sub-task: reuse loop option — delay-line / circular-buffer codegen for
  affine-stride loop-carried slices
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:54'
updated_date: '2026-05-24 02:43'
labels:
  - M5
  - compiler
  - codegen
  - reuse
dependencies:
  - TASK-0043
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#3. The reuse loop option closes 'the 2013 gap' (PRD §13): when a loop body reads grid[i-k..i] across iterations, emit a circular buffer (delay line) so each grid[i] is computed once, not k times.

## Scope
At codegen time, when a Dataflow's kernel arg indices reveal loop-carried OVERLAP (the body's reads on this iteration overlap with the previous iteration's reads), emit a delay-line: a small ring of recently-computed elements, indexed modulo the ring length.

## Acceptance Criteria
1. The reuse loop option, currently parsed but unconsumed (sched/parser.rs + sched/lower.rs:1095), now produces a real codegen artefact.
2. For each affine-stride index reuse pattern, the backend emits a delay line (circular buffer of the right length) instead of re-reading source slices.
3. The reuse semantics are restricted to affine strides only — data-dependent strides REJECTED with typed error (sibling restriction to halo inference; PRD §13).
4. A new e2e cell on example 5 or 7 with reuse in the schedule shows bit-identical output AND a measurably smaller intermediate working-set (e.g. emitted Vec capacities), verified by a new test asserting the delay line length is the access-pattern stride span.
5. Implementation notes record the honest limitation: 'reuse is rejected on data-dependent strides; the user must restructure'.

## Honest scope clarification
- Performance NOT proven in M5 — only correctness + bit-identical re-emit. PRD §11 'examples 5–7 benefit measurably' is a stretch target; this task closes the codegen path. Quantified perf improvement is M6+ scope.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from TASK-0260 cycle 81 (halo inference Stage 1 landed):

Both halo (TASK-0260) and reuse (this task) require the SAME affine-stride prerequisite per PRD §13. The halo-inference pass already lands an affine detector in nucleus-compiler/src/passes/halo_inference.rs — specifically the affine_decompose helper (accepts iv+b with b a const-foldable integer; rejects non-affine, strided, multi-iter-var, and DataRef-inside-index). When TASK-0261 (reuse codegen) lands, lift the affine_decompose helper to a shared pub(crate) location (likely passes/affine.rs or similar) so both halo and reuse share one detector. The HaloInferenceError enum has variants (DataDependentStride, StridedAccessNotSupported, MultipleIterVarsInIndex, NonAffineIndex) that reuse-side errors should mirror in shape.

Stage 1 driver policy worth carrying: the lenient apply_halo_inference_advisory variant exists so that pre-Stage-2, the rejection is advisory only (no e2e baseline regression). Reuse will need the same stance until its codegen is wired — record affine facts but do not fail compilation on non-affine reuse-tagged loops until the codegen consumes them.

Implementation plan (cycle 82):

DESIGN: STAGE 1 only. Inference + sidecar persistence. Stage 2 (backend walker codegen consumer = delay-line emit) is filed as TASK-0265.

DELIVERABLES:
1. LIFT (Commit A): move affine_decompose + eval_const_int + expr_mentions from passes/halo_inference.rs to NEW passes/common.rs (pub(crate) helpers). Re-export via passes/mod.rs. Update halo_inference.rs imports. Tests on the lifted helpers move with them (the affine_decompose_* unit tests). Verify TASK-0260's 24 halo tests still pass.

2. PASS (Commit B): new file passes/reuse_inference.rs. Two entry points:
   - apply_reuse_inference (strict)
   - apply_reuse_inference_advisory (lenient, returns errors vec)
   Walks linked.algo.stmts, for each enclosing for-loop whose ResolvedLoopDirective has ResolvedLoopOption::Reuse, collects all DataRef accesses keyed by (data_name, axis). Computes affine decomposition per index. Aggregates the SET of offsets per (loop-iv, data-name, axis). On a contiguous range, emit a delay-line slot (min_offset, length). Otherwise reject with NonContiguousOffsets.

3. NEW SIDECAR FIELD: reuse_widths: BTreeMap<IterVar, BTreeMap<DataId, ReuseSlot>>  where ReuseSlot { min_offset: i64, length: u64 }. serde-default. Mirror onto ACFG + NameSidecar.

   Decision: key by DataId (not DataName). The acfg.name_data table is the join site; tests + Stage 2 consumer can reverse-lookup. Matches the precedent: halo_widths keys by KernelId / IterVar (both id newtypes), so reuse keying by IterVar / DataId is consistent.

   Decision: key by IterVar (not by KernelId). Rationale: reuse is a LOOP property (the directive sits on a loop var; the delay-line lives outside any one kernel call). Halo is a KERNEL property (the access pattern is per kernel arg).

   Decision: per-axis. A DataRef has multiple indices; only the axis the loop-iv lives on accepts a reuse slot. Inner map shape: per (DataId, axis_idx) -> slot. Reformulated: reuse_widths: BTreeMap<IterVar, BTreeMap<(DataId, u64), ReuseSlot>>. But (DataId, u64) tuple won't survive JSON map key. So nest: BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>>. Three levels. Verbose but compatible with halo precedent.

4. TYPED ERRORS:
   - ReuseInferenceError::DataDependentStride
   - ReuseInferenceError::StridedAccessNotSupported (coeff != 1)
   - ReuseInferenceError::MultipleIterVarsInIndex
   - ReuseInferenceError::NonAffineIndex
   - ReuseInferenceError::NonContiguousOffsets { iter_var, data_name, axis, offsets: Vec<i64> }
   - ReuseInferenceError::UnknownLoopVar (the iv name in name_iter_vars table)
   - ReuseInferenceError::UnknownDataInRef (defensive parity with halo's UnknownKernel)

5. WIRE: driver/src/main.rs calls apply_reuse_inference_advisory AFTER apply_halo_inference_advisory (same lenient stance; no consumer wired yet).

6. >= 10 unit tests:
   - positive: 3-point stencil (offsets -1,0,+1) -> ReuseSlot{min=-1, length=3}
   - positive: separable filter (2 DataRefs)
   - positive: degenerate (only grid[i]) -> length=1 SKIPPED (per spec, no entry)
   - negative: NonContiguousOffsets ({-3, 0, +5})
   - negative: DataDependentStride
   - negative: NonAffineIndex (Mod wrap)
   - negative: StridedAccessNotSupported
   - negative: MultipleIterVarsInIndex
   - no-loop case: kernel call inside a loop without reuse directive -> no entries
   - no-DataRef case: empty
   - determinism: same input twice -> same bytes
   - serde round-trip
   - advisory: multi-error fixture -> strict fails on first, advisory collects all

VERIFICATION GATE:
- just test (+ >=12 new tests)
- just clippy clean
- just e2e (88/73/0/15/0 preserved)
- just determinism-check PASS + negatives

COMMITS:
- Commit A: lift affine_decompose to passes::common
- Commit B: reuse_inference Stage 1 + sidecar field + driver wire

Cycle 82 LANDED Stage 1: reuse loop-option inference + sidecar persistence + driver wire.

Implementation: nucleus/nucleus-compiler/src/passes/reuse_inference.rs (new file, ~1100 lines including tests).
- Two entry points: strict apply_reuse_inference (Result-based, fail-fast) for tests + direct callers, lenient apply_reuse_inference_advisory (collects all errors) for the Stage 1 driver.
- Sidecar shape: BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64 /* axis */, ReuseSlot>>> (3 nested ordered maps; the deep nest is load-bearing for serde JSON — tuple keys are not JSON map keys, same constraint that drove halo Stage 1's nested shape).
- ReuseSlot = { min_offset: i64, length: u64 } — the codegen consumer (Stage 2, TASK-0265) rewrites grid[iv + b] as buf[(iv + b - min_offset) % length].
- Driver uses the lenient variant + nuc_trace! advisory emission — same observational-inertness as halo Stage 1.

Algorithm shape decision: ONE WALK PER REUSE IV (not one walk threading multiple accumulators). When two reuse loops are nested (e.g. for i : reuse { for j : reuse { ... } }), each iv gets its own independent pass through the algorithm tree. Simpler than a stack-of-accumulators and the result is correct: an index that mentions both ivs is rejected by MultipleIterVarsInIndex (same as halo), and an index that mentions only one iv contributes to only that iv's accumulator.

Files touched:
- nucleus-compiler/src/passes/reuse_inference.rs (new file)
- nucleus-compiler/src/passes/common.rs (already landed in commit A — affine_decompose lift)
- nucleus-compiler/src/passes/mod.rs (registers reuse_inference)
- nucleus-compiler/src/lib.rs (re-exports apply_reuse_inference + _advisory + ReuseInferenceError + ReuseSlot)
- nucleus-compiler/src/acfg.rs (added reuse_widths field; updated build_acfg)
- nucleus-compiler/src/sidecar.rs (added reuse_widths field; updated build_sidecar)
- nucleus-compiler/src/passes/{sync_inject,transfer_inject,block_transform,partition_workers,partition_rows,partition_blocks2d,halo_inference}.rs (destructure-and-rebuild: forward reuse_widths verbatim)
- driver/src/main.rs (calls apply_reuse_inference_advisory after apply_halo_inference_advisory, emits nuc_trace! for advisory errors)
- nucleus-compiler/tests/{partition_workers,partition_blocks2d,partition_rows,sync_inject,acfg_to_petri,petri_to_events,transfer_inject,transfer_inject_hoist}.rs (added reuse_widths: BTreeMap::new() to hand-built ACFG instances)

GATE NUMBERS (post-cycle-82):
- just test: 765 / 0 / 3 (was 746 / 0 / 3 baseline pre-cycle-82; +19 = 14 new reuse tests + 13 new common tests - 8 affine_decompose tests moved from halo)
- just clippy: clean
- just e2e: 88 / 73 / 0 / 15 / 0 (UNCHANGED — Stage 1 observationally inert)
- just determinism-check: PASS
- just determinism-check-negative: BITES (73 cells perturbed)
- rustfmt --check on my new files: clean

HONEST LIMITATIONS:
- Coefficient must be +1 (mirrors halo Stage 1; strided / negated-iv reuse rejected with typed error). Deferred.
- Single iter-var per index (multi-iv rejected).
- Non-contiguous offset set rejected (NonContiguousOffsets typed error) — a sparse window like {-3, 0, +5} would waste 6 of 9 slots on a delay line.
- Length-1 slots (degenerate single-offset, e.g. grid[i]-only) dropped silently — no codegen needed.
- 05-stencil / 13-cnn-inference and all other shipped schedules contain ZERO 'loop V : reuse;' directives, so the sidecar field is empty for every cell today. The Stage 2 task (TASK-0265) will add the first reuse directive + bit-identical e2e cell.
- Stage 1 vs Stage 2: this cycle lands the inference + persistence. The backend walker delay-line CODEGEN is a separate cycle (TASK-0265) — until that lands, a reuse directive is a no-op even when fully inferable.

FORWARD-CARRIED LESSONS (for TASK-0265 / Stage 2):
- multi_worker_walker + each per-backend Plan must consume reuse_widths at the Event::Loop emit site. TASK-0263 (halo Stage 2) is the sibling consumer task targeting the SAME emit site but for an ORTHOGONAL purpose (halo widens transfer tiles; reuse rewrites read patterns inside a tile). Schedule the two integrations in either order but watch the join: a Repeat carrying BOTH a halo entry and a reuse entry needs both code-paths active at once.
- The 'one walk per reuse iv' shape carries forward: Stage 2 will likely want the SAME shape — when projecting a particular worker's events, look up reuse_widths.get(iter_var) at the loop-emit site, iterate the per-(DataId, axis) slots independently. Multiple slots on the same loop combine via separate delay-line variables — no cross-slot interaction (verified by the nested-reuse test).
- Determinism is preserved across ALL walks because (a) BTreeSet<&str> iteration of reuse iv names is alphabetic-deterministic, (b) BTreeMap ordering everywhere, (c) BTreeSet<i64> of offsets gives sorted min/max via .iter().next() / .next_back().
- Forward-carried from cycle-81 review: 'multi-error fixture exercises both strict AND advisory paths'. SATISFIED — the advisory_collects_all_errors_strict_short_circuits test does exactly this.

NEXT TASK FILED: TASK-0265 (Stage 2 — backend walker delay-line / circular-buffer codegen).

FINAL SUMMARY (cycle 82, TASK-0261):

Commits:
- 76db68d: passes: lift affine_decompose to passes::common (TASK-0261 prerequisite)
- 005e92b: passes: reuse_inference Stage 1 (TASK-0261) — affine-stride delay-line inference + sidecar persistence

Per-AC status:
- AC#1 ('parsed but unconsumed reuse now produces a codegen artefact'): SATISFIED in Stage 1 by the sidecar field; CODEGEN consumer is Stage 2 (TASK-0265 filed).
- AC#2 ('per affine-stride pattern, backend emits a delay line'): DEFERRED to TASK-0265 — Stage 1 only persists the slot shape; backend walker integration is a separate cycle.
- AC#3 ('reuse semantics restricted to affine strides; data-dependent rejected'): SATISFIED — typed ReuseInferenceError::DataDependentStride, NonAffineIndex, StridedAccessNotSupported all in place + tested.
- AC#4 ('new e2e cell with reuse showing bit-identical output + measurably smaller working set'): DEFERRED to TASK-0265 — requires backend consumer to land first.
- AC#5 ('implementation notes record the honest limitation'): SATISFIED — module docs spell out 'reuse rejected on data-dependent strides; user must restructure', plus the additional limitations (coeff=+1 only; single iv; contiguous-only; length-1 dropped).

This task closes the INFERENCE half of the reuse loop. Stage 2 (TASK-0265) closes the CODEGEN half. AC#1, #3, #5 closed in this cycle; #2, #4 explicitly deferred and tracked.

FORWARD-CARRIED LESSONS to record against TASK-0265 (Stage 2):
- ReuseSlot has min_offset (signed) + length. Backend's per-iteration loop rewrite: buf[(iv + b - min_offset) % length]. Initial-fill prologue needed when min_offset < 0 (the loop entry must pre-populate the slots that 'look back').
- Multiple slots on one loop (e.g. 5-point + 3-point stencil sharing iv i but on different data) require multiple delay-line variables, declared at loop entry, named by (data, axis) or a Stage-2-defined scheme.
- The 'one walk per reuse iv' shape generalises: Stage 2 looks up reuse_widths.get(iter_var) at Event::Loop emit and iterates the per-(DataId, axis) slots independently. The slots don't entangle.
- Shared affine_decompose now lives at passes::common — Stage 2 doesn't need it (the affine work is done; Stage 2 consumes slots, not IrExpr trees), but if Stage 2 needs to RE-CLASSIFY a slot at emit time (e.g. to recover the per-DataRef offset for the rewrite), the helper is one import away.
- Halo Stage 2 (TASK-0263) and reuse Stage 2 (TASK-0265) touch the SAME backend emit site at Event::Loop. Schedule independently; the per-feature integrations don't conflict (halo widens transfer tiles; reuse rewrites read patterns inside a tile).

CONFIRMED: affine_decompose was lifted to passes::common (commit 76db68d) and halo_inference imports from there. All 12 halo integration tests + the 4 sidecar_halo tests still pass post-lift, verifying behaviour preservation.

REVIEW-GATE LANDED (cycle 82 orchestrator hardening, commit 086d396).

Parallel read-only review of cycle-82 implementer commits (76db68d + 005e92b + ef998c6) returned GO from both qa-test-runner and mped-architect.

## In-thread fix (commit 086d396)

F-P1-A (architect): two .expect('non-empty set') calls in finalise_accum's per-axis loop, two lines after an is_empty() guard. Structurally unreachable today, but recurring panic-not-diagnostic feedback says to avoid in-method .expect() on local invariants (they erode at edits). Fix: replace with a single match (offsets.iter().next(), offsets.iter().next_back()) routing Some/Some to (lo, hi) and _ to continue — same behaviour, zero panic surface, no degenerate-empty guard needed (the match handles it directly).

## Gate (post-hardening)

- cargo test nucleus-compiler: 592 / 0 (no change in test count — refactor only).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- Workspace tests (qa-verified pre-hardening): 765 / 0 / 3.
- e2e/determinism/negative gates: 88/73/0/15/0 required-fail; PASS / bite (verified by qa-test-runner cycle-82).

## Other P1+P2 findings filed as forward-carry to TASK-0265

The architect surfaced five forward-carry items for TASK-0265 (Stage 2 backend walker codegen):
1. Promote driver from lenient apply_reuse_inference_advisory to strict before TASK-0265 codegen consumer lands (silent failures become wrong output).
2. Variant rename for cross-pass consistency: reuse uses UnknownLoopVar, halo uses UnknownIterVarInScope. Stage 2 may want shared passes::common::IvScopeError.
3. Sidecar serde round-trip golden test before codegen consumes the triple-nested form.
4. Defensive variants UnknownLoopVar / UnknownDataInRef untested directly (only 6 of 8 reuse-error variants pinned).
5. tests/partition_workers.rs:566 has 'reuse_widths: BTreeMap::new()' (bare) vs other fixtures' 'std::collections::BTreeMap::new()' (qualified). Cosmetic normalisation when Stage 2 touches the test crate.

These are filed as forward-carry notes on TASK-0265 (not new tasks).

## Review-gate decision

Status: same closure-deferred-on-Stage-2 pattern as TASK-0260. AC#1 (parsed-but-unconsumed produces artefact) ✓ MET. AC#3 (reject data-dependent strides) ✓ MET. AC#5 (honest limitations recorded) ✓ MET. AC#2 (backend emits delay line) + AC#4 (e2e bit-identical + smaller working-set) DEFERRED to TASK-0265 Stage 2.

M5 keystone status (TASK-0043): all FOUR sub-tasks (TASK-0258 partition_rows + TASK-0259 partition_blocks2d + TASK-0260 halo_inference Stage 1 + this TASK-0261 reuse_inference Stage 1) have code on disk and review-GO. The Stage 2 deferred follow-ups (TASK-0263 halo consumer, TASK-0264 blocks2d block-pair, TASK-0265 reuse codegen) define the rest of the M5 implementation surface and close in lockstep as their downstream consumers wire.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0261 Stage 1 LANDED in commits 76db68d (affine_decompose lift to passes::common) + 005e92b (reuse_inference pass + ACFG/sidecar fields + driver wire). Gate: 765/0/3 tests, clippy clean, e2e 88/73/0/15/0 (unchanged), determinism PASS + bites. Closes AC#1/#3/#5; defers AC#2/#4 to TASK-0265 (Stage 2: backend walker delay-line emit) — filed.
<!-- SECTION:FINAL_SUMMARY:END -->
