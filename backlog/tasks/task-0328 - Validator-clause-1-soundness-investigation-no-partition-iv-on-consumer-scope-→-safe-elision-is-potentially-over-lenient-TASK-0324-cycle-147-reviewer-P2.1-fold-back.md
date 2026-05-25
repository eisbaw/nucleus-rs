---
id: TASK-0328
title: >-
  Validator clause (1) soundness investigation: 'no partition iv on consumer
  scope → safe elision' is potentially over-lenient (TASK-0324 cycle-147
  reviewer P2.1 fold-back)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 16:25'
updated_date: '2026-05-25 20:28'
labels:
  - compiler
  - transfer_inject
  - validator-coverage
  - latent-defect
  - forward-carried-from-TASK-0324
dependencies:
  - TASK-0324
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 cycle-147 architect-review P2.1 noted that the AC#2 validator's clause (1) at `transfer_inject.rs`'s `check_op_no_silent_elision_risk` short-circuits with 'safe elision' when no partition iv is active on the consumer's enclosing scope:

```rust
if enclosing_partition_ivs.is_empty() {
    continue;
}
```

The implementer's reasoning was 'no partition iv active on consumer scope → every worker owns the full data → safe elision'. But this reasoning is questionable when the PRODUCER wrote with a partition iv but the CONSUMER's enclosing scope has no partition iv:

- Producer: each worker w_i writes its band of `tmp` (partition iv hy).
- Consumer: same worker w_i, but no partition iv in its enclosing scope. Reads `tmp[5][3]` (constant indices).
- Reality: each worker has only its own band populated; `tmp[5][3]` may be OUTSIDE w_i's band → garbage.

The per-axis check at line ~2944 onwards WOULD catch this (producer's axis-0 = Ident(hy) = partition iv; consumer's axis-0 = IntLit(5); c_iv = None != Some(hy) → unsafe), but clause (1) fires FIRST and prevents the per-axis check from running.

## Honest exposure

LOW (dormant). No in-tree schedule exercises this shape today:
- 06/distributed2: consumer IS inside `for vy : partition=rows`, so `enclosing_partition_ivs` is non-empty.
- 13-cnn/batch_parallel: consumer IS inside `for n : partition=workers`, same.
- Every other distributed schedule similarly has the consumer in a partition scope.

But the under-conservative clause is a latent silent-miscompile waiting for a future schedule shape.

## Acceptance criteria

### AC#1: investigate clause (1)'s soundness

Determine whether the original 'every worker owns the full data' reasoning is sound under ANY same-set producer/consumer pairing, or whether it's an over-lenient short-circuit that should be removed. Two possible outcomes:

1. **Clause (1) is correct under SOME stronger invariant** (e.g. 'consumer in unpartitioned scope ⇒ producer is also unpartitioned'). Document the invariant, add an assertion if applicable, leave the clause in place.

2. **Clause (1) is over-lenient** (the analysis above is correct). Remove it; let the per-axis check fire for these cases. Update the emit-site clause-1 mirror added cycle 147 (P2.1 fold-back) to be removed in lockstep.

### AC#2: defensive test

Add a fixture exercising the asymmetry case: producer writes `tmp[hy][hx]` on {w0..w3} with hy partitioned; consumer reads `tmp[5][hx]` at TOP-LEVEL (no enclosing partition iv). Assert the validator's behaviour (Err under AC#1 outcome 2, Ok under AC#1 outcome 1).

### AC#3: documentation

Update the module-doc bullet about the AC#2 validator + the inline comment at clause (1) to reflect AC#1's conclusion.

## Cross-reference

- transfer_inject.rs `check_op_no_silent_elision_risk` clause (1) (around the `if enclosing_partition_ivs.is_empty() { continue; }` line).
- transfer_inject.rs `build_waits_for_op` same-set short-circuit (cycle-147 P2.1 fold-back added the matching clause-1 gate; if AC#1 outcome 2 lands, both sites lose the gate together).
- Architect's full P2.1 finding: cycle 147 parallel review gate.

## Honest scope

- LOW priority. Dormant defect, no in-tree trigger.
- Trigger to escalate: a future schedule with a same-set producer/consumer pairing where the consumer is OUTSIDE any partition iv scope.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Cycle-NNN implementation plan

**Investigation methodology** (per AC#1): write the AC#2 prediction-first fixture, run it against the CURRENT code (with clause (1) in place) to empirically determine the validator's behaviour on the asymmetry case. Two possible outcomes:

- **Validator returns Ok** → clause (1) IS over-lenient → AC#1 outcome 2: remove it at both sites (validator + emit-site mirror).
- **Validator returns Err** → some other check catches the case → AC#1 outcome 1: document the invariant + add an assertion.

**Steps**:

1. Add the AC#2 POSITIVE fixture (`task0328_ac2_positive_partition_producer_topfile_consumer`) to `nucleus/nucleus-compiler/tests/transfer_inject.rs`: producer writes `tmp[hy][hx]` on {w0..w3} with `hy : partition=rows`, consumer reads `tmp[5][hx]` at TOP-LEVEL (no enclosing partition iv), same worker set. ASSERT `Err(SameSetSilentElisionRisk { data: D_TMP, .. })`.

2. Run the fixture against CURRENT code — record empirical result.

3. **If validator returned Ok** (over-leniency confirmed):
   a. Remove clause (1) at validator site (transfer_inject.rs:2935).
   b. Remove clause (1) at emit-site mirror (transfer_inject.rs:3222).
   c. Re-run the AC#2 fixture; it should now PASS.
   d. Add AC#2 NEGATIVE fixture (`task0328_ac2_negative_no_partition_anywhere`): producer NOT in partition, consumer NOT in partition, same workers, same reads. ASSERT validator returns Ok (per-axis check correctly does not reject because no axis is partition-sliced).
   e. Update transfer_inject.rs module-doc + inline comment at the removal sites.
4. **If validator returned Err** (clause (1) sound under some invariant):
   a. Document the invariant (in the validator's docstring).
   b. Add an assertion at clause (1) that the invariant holds.
   c. Skip the removal steps.

5. Full verification gate: `nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"`. Baseline 895/0/3 dev + release, e2e 112/96/0/16/0 — neither should regress.

6. Parallel read-only review gate (qa-test-runner + mped-architect).

7. Tracker notes + commit.

**Honest scope**:

- AC#1 outcome decision is the load-bearing deliverable. AC#2 + AC#3 are immediate fold-back.
- If AC#1 outcome 2 (removal) lands, the cycle counts as resolving the dormant unsoundness — no follow-up task needed for the investigation itself.
- If AC#1 outcome 1 (sound + assertion), the cycle ends with an assertion + docstring + the AC#2 test serves as a regression pin for the invariant.

**Forward-carried from TASK-0330 cycle 153**: P3.2 sibling-walker audit pattern — when adding (or removing) a guard, grep for structurally identical sites that might share the issue. The emit-site mirror at line 3222 is the structural sibling here; removing clause (1) MUST happen in lockstep at both sites.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 154 — landed

### Investigation outcome

**AC#1 = OUTCOME 2 CONFIRMED**: Clause (1) is empirically over-lenient.

The investigation methodology was prediction-first: wrote the AC#2 positive fixture (`task0328_ac2_positive_partition_producer_topfile_consumer`) BEFORE making any code change, predicting that the current validator would Ok the consumer-at-top-level asymmetry case. Ran it against current code → confirmed: validator Ok, emit elides cross-worker pairs.

### What landed (cycle 154)

1. **Validator site** (`transfer_inject.rs::check_op_no_silent_elision_risk` ~line 2900): the `if enclosing_partition_ivs.is_empty() { continue; }` short-circuit REMOVED. The `enclosing_ivs` parameter (and the threading at `check_no_silent_elision_risk_inner`) dropped end-to-end (cycle-154 architect P2.1 fold-back): no consumer in this function family consults the enclosing-iv stack after clause (1) removal.

2. **Emit-site mirror** (`transfer_inject.rs::build_waits_for_op`'s same-set branch ~line 3222): the `if !consumer_has_partition_iv_in_scope { continue; }` short-circuit REMOVED. The emit now always reaches `same_set_elision_unsafe_reason` and, when unsafe, falls through to cartesian-product fan-out.

3. **Cycle-154 architect P1.1 fold-back**: the emit-site removal comment was rewritten to correctly identify the fall-through path as the PRIMARY path for the silent-miscompile shape (the cycle-147 AC#3 lift makes the validator Ok on same-set + unsafe specifically so the emit site here can emit cross-worker pairs). The original comment misdescribed it as a defensive backstop.

4. **Module-doc**: TASK-0328 paragraph added to `transfer_inject.rs` near the AC#2/AC#3 narrative (line ~169) describing the removal, the load-bearing classifier (`same_set_elision_unsafe_reason`), and the two test pins.

### Test pins (3 total, all PASS)

- `task0328_ac2_positive_partition_producer_topfile_consumer`: producer at `for hy:partition=rows` writes `tmp[hy][hx]`; consumer at TOP-LEVEL reads `tmp[5][3]` (IntLit branch). Asserts 12 cross-worker pairs emitted post-fix (was 0 pre-fix → silent miscompile).
- `task0328_ac2_negative_no_partition_anywhere`: same shape but no partition iv anywhere. Asserts 0 cross-worker pairs (safe elision still happens via the per-axis check returning None on every axis).
- `task0328_ac2_positive_topfile_consumer_nonpartition_iv` (cycle-154 architect P2.2 fold-back): consumer at top-level reads `tmp[k][j]` where k, j are non-partition outer ivs. Asserts 12 cross-worker pairs — exercises the OTHER branch of `ident_iv_in_set` (Ident matching non-partition iv → None, vs IntLit → None).

### Composition correctness (architect-traced)

Verified statically across all in-tree shapes:

- **06/distributed2** (consumer in partition scope, name-mismatch axis-0): validator Ok via AC#3 lift, emit reaches same_set_elision_unsafe_reason → unsafe → 12 pairs. **UNCHANGED** by cycle-154 (clause (1) didn't fire pre-cycle-154 either — consumer had partition iv in scope).
- **13-cnn/batch_parallel** (reader-iv == partition-iv): same-set branch, same_set_elision_unsafe_reason returns None → elide. **UNCHANGED**.
- **TASK-0328 fixture** (consumer top-level constants): unsafe_reason Some → fall through → 12 pairs. **NEW correct behaviour** (was 0 pre-cycle-154).
- **No-partition-anywhere**: `partition_iter_vars` empty → unsafe_reason None on every axis → safe → elide. 0 pairs. **UNCHANGED**.

### Cycle-154 architect P2.3 fold-back

The cycle-148 paired-lift discipline ([[feedback-silent-sibling-defect]]) requires auditing structurally identical siblings. The **partial-overlap arm** (`if src == dst` inside the cartesian-product fan-out) is the sibling for the cycle-154 clause (1) removal. Filed as **TASK-0333** (LOW, dormant — no in-tree schedule exercises partial-overlap today). The audit's outcome will be one of:

- Validator's existing partial-overlap rejection (line ~3013, SameSetSilentElisionRisk) is sound — load-bearing — no fix needed.
- Or it's the same class of over-lenience — needs cycle-154-style fix.

### Gotchas / subtleties for future maintainers

1. **The validator's same-set + unsafe Ok return is INTENTIONAL** (cycle-147 AC#3 lift). It is NOT a bug to fix — it relies on the emit-site fan-out to do the right thing. Cycle-154 strengthened that emit-site behaviour by removing the elision short-circuit.

2. **The investigation surface flipped**: my initial prediction was "validator rejects under outcome 2". The empirical truth is "validator continues to Ok (via AC#3 lift), but emit-site removal is what prevents the silent miscompile". This was a load-bearing pivot — the AC#2 test was re-targeted from validator-rejection to emit-shape assertion mid-cycle.

3. **`enclosing_ivs` was end-to-end dead** after the leaf-site removal. The cycle-154 P2.1 fold-back removed the threading entirely (parameter + caller + inner-walker arg). The inner walker is simpler now: `Repeat` arm just descends without building a nested vec.

4. **The IntLit branch and the non-partition-iv branch of `ident_iv_in_set`** are structurally different though both return None. The cycle-154 base positive test exercised IntLit (constant index); the architect P2.2 sibling pin exercises the non-partition-iv branch. Both must remain covered.

### Forward-carried lessons (to TASK-0333)

- **The partial-overlap arm audit** when worked: follow the cycle-154 prediction-first methodology (write the synthetic fixture before changing code; observe validator + emit behaviour; pivot the test target if the empirical truth differs from the prediction). Document the actual defect-surface (validator vs emit) in the fix narrative.

- **Forward-carry from TASK-0330 cycle 153 P3.2 audit pattern STILL VALID**: when adding (or removing) a guard, grep for structurally identical sibling sites. Cycle-154 ran this discipline successfully (emit-site mirror removed in lockstep with validator removal). The same pattern should fire on the cycle-154 work itself, prompting TASK-0333's partial-overlap sibling audit.
<!-- SECTION:NOTES:END -->
