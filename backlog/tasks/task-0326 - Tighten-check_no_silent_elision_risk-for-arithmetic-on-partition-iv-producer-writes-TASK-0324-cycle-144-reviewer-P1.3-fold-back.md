---
id: TASK-0326
title: >-
  Tighten check_no_silent_elision_risk for arithmetic-on-partition-iv producer
  writes (TASK-0324 cycle-144 reviewer P1.3 fold-back)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 14:19'
updated_date: '2026-05-25 21:45'
labels:
  - compiler
  - transfer_inject
  - validator-coverage
  - M6
  - forward-carried-from-TASK-0324
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 cycle-144 reviewer (mped-architect) P1.3: the per-axis discriminator in `check_no_silent_elision_risk` skips axes where the producer's write index is not a bare Ident — including the case where the producer writes with an ARITHMETIC expression involving a partition iv (e.g. `tmp[hy*2][hx]`, `tmp[hy+1][hx]`).

Semantically these accesses ARE partition-sliced (the worker writes only its own transformed range), but the structural check (`ident_iv_in_set` returns None on non-Ident) treats the axis as whole-array → no constraint on the consumer's axis-k read → potential silent elision the same way as the underlying TASK-0324 case.

## Cycle-144 disclosure (in-thread)

The validator at `transfer_inject.rs` (the `p_iv = ident_iv_in_set(...)` branch in the per-axis loop) carries an explicit `CONSERVATIVELY-NOT-REJECTED` comment naming this task. No in-tree schedule today exercises arithmetic producer-side indices on a partition iv.

## Acceptance criteria

### AC#1: enrich the discriminator

Extend `ident_iv_in_set` (or its consumer) to also detect arithmetic expressions involving a partition iv. The simplest sufficient extension: walk the `IrExpr` tree for any `IrExpr::Ident` referencing an iv in `partition_iter_vars`; if found, mark the axis as partition-sliced and require the consumer's axis-k read to ALSO contain only that same partition iv (i.e. same arithmetic shape — or at minimum, same set of referenced ivs).

### AC#2: positive + negative tests

- Positive fixture: producer writes `tmp[hy*2][hx]` (or similar) on {w0..w3} with hy partitioned; consumer (same set) reads `tmp[hy*2][hx]` → expect `Ok` (read shape matches write shape, worker reads its own slice).
- Positive fixture: producer writes `tmp[hy*2][hx]` on {w0..w3} with hy partitioned; consumer reads `tmp[other_iv][hx]` → expect `Err`.

### AC#3: alignment with halo machinery

Halo widths (TASK-0260) extend a worker's local tile by the kernel's access pattern. If the producer writes with arithmetic but the access stays within the halo-extended tile, the elision IS safe. The validator must NOT over-reject in that case — either honour halo widths in the discriminator OR document the conservative-reject path with a halo-aware escape valve.

## Dependencies

- TASK-0324 (cycle-144 base validator). This task tightens its discriminator on a path the cycle-144 implementer intentionally left under-conservative pending an in-tree need.

## Honest scope

- LOW priority. Dormant path. Filed so the under-conservative comment at transfer_inject.rs in the cycle-144 fold-back has a tracker anchor (per TASK-0319 / future-audit discipline: every 'conservatively not rejected' code comment needs a tracker reference).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle-156 implementation plan (TASK-0326 AC#1+AC#2+AC#3):

AC#1 — tighten classifier:
1. Add helper expr_references_partition_iv(expr, set, name_iter_vars) -> bool right next to ident_iv_in_set in transfer_inject.rs. Recursive walk over IrExpr:
   - Ident(name): true iff name resolves via name_iter_vars to an iv in set.
   - IntLit: false.
   - Neg(e): recurse on e.
   - BinOp(_, l, r): recurse on l OR r.
   - DataRef(IndexedRef): recurse on every index (defensive — DataRef as producer index is upstream-rejected today, but the predicate stays sound).
   - Call{args}: recurse on every arg (same defensive note).
2. Rewrite the per-axis loop in same_set_elision_unsafe_reason (L3105–3148):
   - Replace 'let Some(p_iv) = ident_iv_in_set(...)' gate with the new tree predicate.
   - If producer's axis-k expr references any partition iv: require consumer's axis-k expr to be STRUCTURALLY EQUAL (IrExpr::PartialEq derives Eq + PartialEq) to producer's. If unequal → return Some(reason) naming both.
   - If producer's axis-k expr does NOT reference any partition iv: continue (whole-array axis on producer; consumer unconstrained).
   - This SUBSUMES the existing bare-Ident path: when producer is Ident(hy) and consumer is Ident(hy), structural equality holds → continue. When consumer is Ident(other) or IntLit(c), inequality → return Some.
3. Also tighten the any_partitioned_axis predicate at L3088–3091 (the whole-array-consumer-read clause) to use the new tree-walking predicate — same defect class on that branch.
4. Update the CONSERVATIVELY-NOT-REJECTED comment block at L3110–L3132 to document the new tighter rule + the option-B halo-aware escape valve (TASK-0326 follow-up if a real schedule trips over-rejection).
5. Audit both call sites (validator L2976 + emit-site L3263): no signature change, so both call sites pick up the new behaviour automatically (the silent-sibling defense per [[feedback-silent-sibling-defect]]).

AC#2 — fixtures in tests/transfer_inject.rs:
1. task0326_ac1_positive_arithmetic_matched_partition_iv: producer writes tmp[hy*2][hx], consumer reads tmp[hy*2][hx] on same set; expect Ok with cross-worker fan-out (or no-rejection — same shape as task0328 positive; the consumer is at top-level with same constant trigger so AC#3 lift gate emits 12 cross-pairs).
2. task0326_ac1_negative_arithmetic_mismatched_iv: producer writes tmp[hy*2][hx], consumer reads tmp[other_iv][hx]; expect Err(SameSetSilentElisionRisk) with message anchored on partition-sliced.
3. task0326_ac1_negative_arithmetic_mismatched_arithmetic: producer writes tmp[hy*2][hx], consumer reads tmp[hy + 1][hx]; expect Err.
4. Bare-Ident regression: existing task0328_ac2_positive_partition_producer_topfile_consumer must still pass (the bare-Ident case is subsumed by structural equality of Ident(hy)==Ident(hy)).

AC#3 — halo escape valve: option B chosen. Replace the CONSERVATIVELY-NOT-REJECTED comment block with documentation of the new tight rule + the halo-aware escape valve as an open follow-up path. Only file a follow-up tracker task if a specific in-tree schedule is identified as over-rejected (likely none — current schedules use bare-Ident partition writes).

Verification gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e'
Expected new test count: 899+3 = 902 (or 902 + however the bare-Ident regression-pin is structured). E2E baseline must hold: 112/96/0/16/0.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-156 implementation summary

### AC#1 — tightened classifier (LANDED)
- src/passes/transfer_inject.rs L3081–3168 ():
  - Per-axis check replaced bare-Ident gate with recursive tree-walking predicate. If producer's axis-k expression references any partition iv anywhere in the IrExpr tree, the consumer's axis-k expression must be STRUCTURALLY EQUAL (`IrExpr` derives PartialEq + Eq) — else `Some(reason)`.
  - `any_partitioned_axis` (the whole-array consumer-read clause at L3088–3103) also tightened to use the tree walker — fixes the silent-sibling defect on that branch.
  - Bare-Ident case is SUBSUMED: `Ident(hy) == Ident(hy)` is structurally equal; `Ident(hy)` vs `IntLit(5)` is structurally unequal. Existing tests pass unchanged.
- L3194–3237 ():
  - New recursive helper. Walks `Ident / IntLit / Neg / BinOp / DataRef / Call`. DataRef/Call recursion is defensive (upstream-rejected today; sound if the gate changes).
  - Dead bare-Ident-only `ident_iv_in_set` REMOVED (single source of truth — MPED principle).

### Both call sites picked up automatically (silent-sibling defense)
- Validator ( at L2976) and emit-site mirror (`build_waits_for_op` at L3263) both call `same_set_elision_unsafe_reason` unchanged signature. Tightening propagates to both by construction (the cycle-147 P2.1 symmetry fold-back invariant preserved).

### AC#2 — fixtures (LANDED)
tests/transfer_inject.rs L4257–5005, three new tests:
- `task0326_ac1_positive_arithmetic_matched_partition_iv`: producer `tmp[hy*2][hx]` + consumer `tmp[hy*2][hx]` on same set → Ok, zero cross-pairs (safe elision).
- `task0326_ac1_negative_arithmetic_mismatched_iv`: producer `tmp[hy*2][hx]` + consumer `tmp[other_iv][hx]` → Ok with 12 cross-worker pairs (classifier returns Some → cycle-147 AC#3 lift fan-out).
- `task0326_ac1_negative_arithmetic_mismatched_arithmetic`: producer `tmp[hy*2][hx]` + consumer `tmp[hy+1][hx]` → Ok with 12 cross-worker pairs. This is the case where the brief's 'minimum set-of-referenced-ivs' would have accepted — structural equality REJECTS (safer call per fail-loud bias).
- The bare-Ident regression pin `task0328_ac2_positive_partition_producer_topfile_consumer` continues to assert 12 cross-pairs (structural equality of `Ident(hy) == Ident(5)` fails → Some(reason) → cartesian fan-out). NO regression.

### AC#3 — option B (documented; option A deferred)
- Replaced CONSERVATIVELY-NOT-REJECTED comment block at the per-axis check with a new doc block (L3105–3155 in patched file) covering: (a) the tightened structural-equality rule, (b) safety direction (fail-loud bias), (c) the halo-aware escape valve as an open follow-up path (not filed — no in-tree schedule trips over-rejection; e2e baseline 112/96/0/16/0 holds).
- Empirical check: only `driver/tests/fixtures/task_0274_strided_reuse/prog.algo.nuc` uses arithmetic data indices in-tree; e2e is green → no over-rejection observable. Option A (halo-aware escape) deferred per brief; will be filed if a real schedule trips this.

### Gate (fresh run)
- just build: ok
- just clippy: ok
- just test: 902 / 0 / 3 (was 899/0/3 per cycle-155b → +3 from the three new TASK-0326 fixtures)
- just test-release: 902 / 0 / 3
- just e2e: total 112 / pass 96 / fail 0 / skipped 16 / required-fail 0 (BASELINE PRESERVED)
- just check-textual-replace-on-codegen: ok
- just check-include-str-coverage: ok

### Subtleties / gotchas hit
1. The classifier is used by TWO call sites (validator L2976 + emit-site L3263) — silent-sibling defense — verified both pick up the new behaviour without code change because they call `same_set_elision_unsafe_reason` directly.
2. Dead bare-Ident `ident_iv_in_set` was removed (no remaining callers post-rewrite). Two grep-anchored comments still mention the historical name to document the cycle-156 transition.
3. The `any_partitioned_axis` branch (whole-array consumer read) had the same bare-Ident-only defect class — tightened in lockstep. Silent-sibling per [[feedback-silent-sibling-defect]].
4. Defensive `DataRef`/`Call` recursion in `expr_references_partition_iv` — upstream-rejected today but adds safety-by-construction if the upstream gate ever changes.
5. Option-B choice rationale: no in-tree schedule uses arithmetic-on-partition-iv producer writes (verified by grep + e2e green); the halo-aware extension can wait for a real trigger. Filing a follow-up task with no concrete trigger would be premature.

### Limitations honestly disclosed
1. Structural-equality is strictly stronger than semantic equivalence. If a future kernel writes `tmp[hy + 0][hx]` and reads `tmp[hy][hx]`, structural-equality rejects despite the values being equal at every iv. Acceptable: fail-loud, user refactors to bare-Ident or files a follow-up.
2. The comment block's halo-aware escape valve is documentation only. No tracker task filed (no in-tree trigger). If a future schedule needs it, file the follow-up then.
3. The cycle-156 tightening only constrains the consumer's structural shape. It does NOT verify the producer's write actually covers the consumer's read range when the iv values diverge over time (e.g. a reduction across hy*2 and a partial read of hy*2). That is out of scope for this validator — handled by other passes.

## Cycle 156b — review-gate fold-back addendum

Both review agents returned GO on cycle 156 (commit 3a98e20, applied against baseline 356b843=cycle-155b). No P1/P2 findings; five P3 forward-carry items.

Source citation per cycle-155b hygiene rule (FIRST FIRING of the new rule applied retroactively, per architect P3.1 review observation): baseline e2e 112/96/0/16/0 from commit **356b843** (TASK-0333 cycle-155b closure); cycle-156 measured baseline 112/96/0/16/0 byte-identical across two qa-test-runner samples in the review-gate run.

Applied in-thread (cycle-156b):

- **qa P3 / architect P3.2 (actionable)**: tense-marked the two pure-history `ident_iv_in_set` docstring references at `nucleus/nucleus-compiler/tests/transfer_inject.rs` L3766+L3770 with explicit `pre-cycle-156` framing + a forward-reference to `expr_references_partition_iv` (cycle-156, commit 3a98e20). The other two references at L4259 and L4513 are intentional cycle-156 transition markers and were left as-is per architect's read.
- **architect P3.1**: cycle-155b hygiene rule (cite source commit hash, not bare 'cycle-155b' text) applied here retroactively in this addendum. First firing of the rule; no further fold-back possible on the cycle-156 commit message itself per project policy (don't amend landed commits). Forward-carried as a discipline anchor for future cycles.

Accepted as-is (no fold-back required):

- **architect P3.3**: no halo-aware-escape-valve tracker task filed — consistent with cycle-155b architect P3.1 grep-anchor precedent. No in-tree trigger.
- **architect P3.4**: negative-test assertions count Push pairs (12) rather than message substring. The classifier is private; exposing it test-only is a larger refactor. Acceptable.
- **architect P3.5**: over-rejection risk on syntactic-but-not-semantic differences (`hy*2` vs `2*hy`, `hy+0` vs `hy`). Verified by architect against `algo/lower.rs` L1140-1182: NO upstream constant-folding/canonicalisation, so the over-rejection is reachable in principle. No in-tree trigger today. Fail-loud bias dominates; user can refactor or file follow-up when triggered.

### Cycle-156b gate

- `just test`: 902/0/3 (preserved across docstring-only edit).
- `just clippy`: zero warnings.
- `just e2e`: 112/96/0/16/0 preserved (single sample post-edit; reviewer verified non-flake × 2 in cycle 156).

### Final closure

TASK-0326 stays Done. Validator + emit-site classifier tightened from bare-Ident to recursive-tree-walk + structural-equality. Bare-Ident path subsumed and removed. Three new fixtures pin the new behaviour. Halo-aware escape valve documented in-code as future work; no tracker task filed pending in-tree trigger.

### Forward-carried lessons (for future implementers)

- The cycle-155b new hygiene rule (cite source commit hash for baseline citations) was missed in the cycle-156 implementer brief AND the cycle-156 commit message. First-firing. The orchestrator's own brief discipline needs the same hygiene applied at brief-write time.
- Architect's empirical verification step (read `algo/lower.rs` to confirm no upstream constant-folding) was a load-bearing soundness check the orchestrator's brief did NOT explicitly request. Pattern: for tightening-the-discriminator-style cycles, the brief should include the upstream-canonicalisation question as an explicit verification step.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle-156 LANDED. AC#1+AC#2+AC#3 all closed.

AC#1: same_set_elision_unsafe_reason tightened from bare-Ident to recursive tree-walking expr_references_partition_iv + IrExpr structural-equality. Bare-Ident case subsumed (Ident(hy) == Ident(hy) trivially structurally equal). any_partitioned_axis branch tightened in lockstep (silent-sibling defense). Both classifier call sites (validator L2976 + emit-site L3263) pick up the new behaviour automatically (signature unchanged).

AC#2: three new fixtures in tests/transfer_inject.rs:
- task0326_ac1_positive_arithmetic_matched_partition_iv: tmp[hy*2][hx] -> tmp[hy*2][hx], same set, Ok, 0 cross-pairs (safe).
- task0326_ac1_negative_arithmetic_mismatched_iv: tmp[hy*2][hx] -> tmp[other_iv][hx], same set, Ok with 12 cross-pairs (cartesian fan-out via cycle-147 AC#3 lift, classifier returned Some).
- task0326_ac1_negative_arithmetic_mismatched_arithmetic: tmp[hy*2][hx] -> tmp[hy+1][hx], same set, Ok with 12 cross-pairs. SAFER call than the brief's set-of-ivs hint (structural-equality, fail-loud).
Existing bare-Ident regression pin task0328_ac2_positive continues to assert 12 cross-pairs.

AC#3: option B chosen. CONSERVATIVELY-NOT-REJECTED comment block replaced with structural-equality rule documentation + halo-aware escape valve as open follow-up path. No follow-up task filed (no in-tree schedule trips over-rejection; e2e baseline 112/96/0/16/0 preserved). Will file if a real trigger appears.

Gate (fresh): just test 902/0/3 (was 899/0/3, +3 from new fixtures). just test-release 902/0/3. just e2e 112/96/0/16/0 (baseline). just check-textual-replace-on-codegen ok. just check-include-str-coverage ok. just clippy ok.

Dead ident_iv_in_set helper removed (single source of truth). Defensive DataRef/Call recursion in the new helper (upstream-rejected today; sound by construction).
<!-- SECTION:FINAL_SUMMARY:END -->
