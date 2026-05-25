---
id: TASK-0328
title: >-
  Validator clause (1) soundness investigation: 'no partition iv on consumer
  scope → safe elision' is potentially over-lenient (TASK-0324 cycle-147
  reviewer P2.1 fold-back)
status: To Do
assignee: []
created_date: '2026-05-25 16:25'
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
