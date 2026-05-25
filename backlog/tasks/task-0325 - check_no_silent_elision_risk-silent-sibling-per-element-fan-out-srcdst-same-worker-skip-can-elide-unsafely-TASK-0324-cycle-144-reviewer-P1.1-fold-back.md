---
id: TASK-0325
title: >-
  check_no_silent_elision_risk silent sibling: per-element fan-out (src,dst)
  same-worker skip can elide unsafely (TASK-0324 cycle-144 reviewer P1.1
  fold-back)
status: In Progress
assignee:
  - orchestrator-self
created_date: '2026-05-25 14:18'
updated_date: '2026-05-25 14:47'
labels:
  - compiler
  - transfer_inject
  - silent-sibling
  - silent-miscompile
  - M6
  - forward-carried-from-TASK-0324
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 cycle-144 reviewer (mped-architect) P1.1 finding: the validator `check_no_silent_elision_risk` only fires when `producer_workers == &consumer_workers` (set equality at the line-2501 short-circuit). The structurally-identical sibling at the per-element `if src == dst { continue; }` inside the fan-out loop (`build_waits_for_op`, currently at transfer_inject.rs:2994 — search for `if src == dst` inside the cartesian-product loop) can elide unsafely for partially-overlapping (not equal) worker sets.

## Scenario (not currently exercised in-tree)

producer_workers = {w0, w1, w2, w3}, consumer_workers = {w0, w1, w2, w3, w4}. The set-equality check at line 2501 does NOT fire (sets differ). The fan-out emits 5×4 = 20 (src, dst) pairs, of which 4 are eliminated by `src == dst` (the (w_i, w_i) self-pairs for i in 0..4). The remaining 16 emit cross-worker pairs.

Each (w_i, w_i) self-skip is the same structural pattern as the line-2501 elision: a same-worker on both sides where the consumer's read could reach outside the local producer's slice. The line-2501 validator does NOT cover this case because the test is at the WHOLE-SET level (BTreeSet equality), not at the PER-ELEMENT (src, dst) level.

## Why this is a recurring silent-sibling per feedback-silent-sibling-defect

cycle-128/138/140/141/142/142b/143/144 has fired the silent-sibling meta-rule 8 times now. The cycle-144 architect caught this on read-only review — the cycle-144 implementer (orchestrator) did not search for the per-element analogue of the set-equality short-circuit before claiming closure of AC#2.

## Acceptance criteria

### AC#1: detection

Extend `check_no_silent_elision_risk` (or add a sibling helper) to walk the cartesian product of producer × consumer worker sets and apply the same per-axis discriminator to every same-worker (src == dst) pair in the cartesian product, NOT just the whole-set case.

### AC#2: positive + negative tests

- Positive fixture: producer={w0..w3}, consumer={w0..w3, w4}, consumer reads at a non-aligned slice on a partition axis → expect `Err(SameSetSilentElisionRisk)` (the same variant; the elision is structurally identical to the set-equality case).
- Negative fixture: same shape but consumer's read iv matches producer's partition iv on every partitioned axis → expect `Ok`.

### AC#3: matrix audit

No in-tree schedule today places a kernel on partially-overlapping worker sets. Audit the 06/07/13 schedules + any new schedule landing alongside this task. If none is exercising the pattern, AC#3 is met. Document the dormant-but-defended state in the validator's docstring.

## Dependencies

- TASK-0324 (cycle-144 same-set elision validator) — this task generalises the validator to the per-element analogue.

## Honest scope

- LATENT defect. No in-tree schedule today exercises partially-overlapping placements. The line-2994 silent skip is dormant. Filing the gap so a future M6+ schedule landing this pattern is caught by the existing architectural check.

- Reviewer P1.1 was specifically noted as a recurring silent-sibling firing. Memory entry `feedback-silent-sibling-defect` should be updated with cycle-144 P1.1 as the 8th firing.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 145 — orchestrator self-implemented (mped-architect implementer refusal mitigation per CLAUDE.md memory)

### What landed (AC#1 + AC#2 + AC#3 docstring audit)

1. **AC#1 detection extension** at `transfer_inject.rs:2789-2812`: replaced the set-equality short-circuit `if producer_workers != &consumer_workers { continue; }` with a non-empty-intersection test `if (producer_workers ∩ consumer_workers).is_empty() { continue; }`. The downstream per-axis check is unchanged — it operates on producer write pattern vs consumer read pattern, independent of worker-set membership.

2. **Error message specialisation** at lines 2906-2941: split the message into a set-equality arm (existing language preserved) and a partial-overlap arm (new — names `same_worker_set` explicitly). Both arms include forward-links to TASK-0324 and TASK-0325. Variant kept as `SameSetSilentElisionRisk` because the underlying defect class is identical.

3. **Validator docstring rewrite** at lines 2603-2657: replaced the cycle-144 set-equality framing with a two-elision-sites framing (line-2501 whole-set, line-3048 per-element). Cross-references both TASK numbers and the cycle-145 generalisation. Module-level header at lines 119-148 reframed similarly with a new TASK-0325 paragraph.

4. **AC#2 fixtures** at `tests/transfer_inject.rs:2599-2895`:
   - `task0325_ac2_positive_partial_overlap_non_aligned_read`: producer={w0..w3}, consumer={w0..w3, w4}, consumer reads `tmp[vm][vx]` (06/distributed2-style non-aligned). Expects `Err(SameSetSilentElisionRisk { data: D_TMP, .. })` AND message contains TASK-0324, TASK-0325, 'overlap', 'partition-sliced'.
   - `task0325_ac2_negative_partial_overlap_aligned_read`: producer={w0..w3}, consumer={w0..w3, w4}, both read/write `feat1[n]` under `loop n : partition=workers`. Expects `Ok`.

5. **AC#3 matrix audit**: no in-tree schedule places a kernel on partially-overlapping worker sets today (05/06/07/13 + all distributed schedules use set-equal placements or host-↔-worker shapes). The dormant-but-defended state is documented in the validator's docstring (lines 2618-2640) and the module-level header (lines 130-146).

### Verification gate (cycle-145 self-run)

- `just check`: clean.
- `just clippy --all-targets -D warnings`: clean.
- `just test` (dev profile): all pass (counts unchanged from cycle 144 + 2 new fixtures).
- `just test-release`: all pass.
- `just e2e`: 112/92/0/20/0 — IDENTICAL to pre-cycle-145 baseline.
- `just check-textual-replace-on-codegen`: OK.
- `just check-include-str-coverage`: OK.
- `just ci` (full hard gate): green including all 4 negative/determinism arms.

### Gotchas + forward-carries

1. **Generalisation soundness**: the downstream per-axis check is independent of worker-set membership (it discriminates on the access pattern, not on the worker), so simply broadening the entry predicate from set-equality to non-empty-intersection is correct without further changes.

2. **AC#3 dormant-but-defended audit method**: `grep -rE 'workers = .*\{[^}]*\}' nuc-nucleus/examples/*/schedules/` — every shipped schedule uses set-equal worker placements per kernel. No partial-overlap placement exists yet. Filed as part of validator docstring.

3. **Test fixture template**: the cycle-144 AC#5 positive/negative pair was reused as the structural template for cycle-145 — only the worker-set membership differs. This pattern will recur if future shapes are filed as TASK-0326's arithmetic-on-partition-iv producer-writes are tested.

4. **TransferInjectError #[non_exhaustive] is load-bearing now**: cycle-144 added it defensively; cycle-145 didn't add new variants but the partial-overlap fixture's match-arm uses the wildcard pattern — future variants (AC#3 lift, TASK-0326) land without breaking either fixture's match.

5. **TASK-0325 was filed AS the silent-sibling of the cycle-144 implementer's blind spot** (cycle-144 architect P1.1). Cycle 145 closes that gap. The meta-lesson (per [[feedback-silent-sibling-defect]] cycle-144 update): when writing a new validator/guard against a defect class, enumerate every structural variant of the class in the codebase BEFORE writing the validator. This time the line-2501 vs line-3048 enumeration is now anchored in the docstring (lines 2618-2640).
<!-- SECTION:NOTES:END -->
