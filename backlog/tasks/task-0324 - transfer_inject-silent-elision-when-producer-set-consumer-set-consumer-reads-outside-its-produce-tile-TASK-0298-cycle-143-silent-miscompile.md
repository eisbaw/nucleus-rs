---
id: TASK-0324
title: >-
  transfer_inject silent elision when producer-set == consumer-set + consumer
  reads outside its produce-tile (TASK-0298 cycle-143 silent-miscompile)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 13:05'
updated_date: '2026-05-25 15:01'
labels:
  - compiler
  - transfer_inject
  - silent-miscompile
  - panic-not-diagnostic
  - M6
  - forward-carried-from-TASK-0298
dependencies:
  - TASK-0298
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0298 cycle 143 investigation: a both-passes-distributed
schedule for 06-separable-filter
(`nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc`)
exposed a SILENT MISCOMPILE in transfer_inject.

## Schedule shape (the reproducer)

- pass 1 `hblur_acc` placed on `{w0..w3}`, `loop hy : partition=rows;`
- pass 2 `vblur_acc` ALSO placed on `{w0..w3}`, `loop vy : partition=rows;`
- `transfer tmp : sync;` declared.

## Algorithm-level data dependency

Pass 1 writes `tmp[hy][hx]` for hy in worker w_i's row-band.
Pass 2 reads `tmp[vm][vx]` where vm sweeps `0..H` for every (vy, vx).

Each consumer worker w_j (owning row-band [vy_lo..vy_hi]) needs
the ENTIRE `tmp` matrix to compute its output rows — the vm sweep
reads rows OUTSIDE w_j's own producer row-band.

## What transfer_inject emits (the defect)

`grep -n 'slot_' /tmp/task0298_pthreads_sync/src/main.rs` confirms
**zero slots allocated for `tmp`**. Only 8 slots total: 4 for
`in_arr` (host → workers) + 4 for `out` (workers → host). No
worker → worker `tmp` transfers exist in the emit; each worker's
`tmp` Vec holds only its own hy row-band, and pass 2's vm sweep
silently reads zeros from the non-owned rows.

The runtime artefact: `cmp output.bin reference.bin` reports
divergence at cmp 1-based byte 129 (== 0-based offset 128), the
first byte of row 2. With H=W=16 and i32 (4 bytes), row stride =
64 bytes → row 2 starts at offset 128. Row 2 is the first output
row whose vertical taps reach row 4 — outside w0's row-band 0..4.

## Root cause (PRECISE — cycle-143 architect P2-1 correction)

The elision happens at a `BTreeSet` set-equality short-circuit,
BEFORE any tile or reader-iv analysis. From
`nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2501-2503`:

```
if producer_workers == &consumer_workers {
    continue; // Same entity — intra-worker dataflow.
}
```

The `continue` skips the entire TASK-0117 fan-out cartesian-
product loop at lines 2544-2559. The pass NEVER REACHES tile
construction or reader-iv inspection for this case. Cycle-143's
initial framing ("the per-pair-tile machinery treats each
worker's access as same-worker without checking whether the
consumer's read tile fits the producer's write tile") was
imprecise — there is no tile-aware code path here at all to
"not check"; the path is `continue; no transfer`.

Why the existing 05-stencil/distributed schedule does NOT
trigger this: 05's halo source (host load_image) and dest
(workers) DIFFER as sets, so line 2501 never fires for img_in;
likewise img_out (workers → host save_image). 06/distributed2
is the first in-tree schedule with producer-set == consumer-set
AND the consumer reading outside its own slice.

## Silent sibling: 13-cnn-inference/batch_parallel
(cycle-143 architect P2-2 — currently MASKED, latent footgun)

`nuc-nucleus/examples/13-cnn-inference/schedules/batch_parallel.sched.nuc:17-22`
places `conv_block_1`, `conv_block_2`, `classifier` ALL on
`{w0..w3}` with `loop n : partition=workers;`.
`nuc-nucleus/examples/13-cnn-inference/prog.algo.nuc:58-60`:

```
feat1[n]  <-- conv_block_1(input[n]);
feat2[n]  <-- conv_block_2(feat1[n]);
output[n] <-- classifier(feat2[n]);
```

Producer-set == consumer-set on `feat1` AND `feat2`. The
line-2501 continue fires for both — IDENTICAL code path to 06/
distributed2. Today this is correctness-safe because reader iv
`n` IS the partition iv (`partition=workers` on `n`), so each
consumer reads exactly its own slice. BUT:

1. The silent-elision code path fires identically.
2. The current `[[skip]]` for 13-cnn/batch_parallel (e2e-matrix.toml
   ~lines 464-499) cites an UNRELATED reason (TASK-0042 partition=
   workers gap), so the silent-elision class is double-masked.
3. Any future shift / halo / cross-batch reuse variant on
   13-cnn would silently miscompile with no e2e signal.
4. Once TASK-0042 lifts (unblocks 13-cnn/batch_parallel), the
   latent unguarded path becomes a hidden footgun whose
   correctness depends on a coincidence between reader-iv and
   partition-iv.

This is the cycle-128/138/140/141/142/142b/143 silent-sibling
meta-rule firing for the SEVENTH time. The cycle-143 implementer
did NOT search for siblings before filing TASK-0324; the gap was
caught by the architect's read-only review.

## Acceptance criteria

### AC#0: doc-lie fix (cycle-143 architect P2-3)

Fix the doc-lie at `transfer_inject.rs:82-90`:

```
//! - **N-to-M fan-out** (both sides multi-worker, e.g. an all-to-all
//!   shuffle) falls back to the "compute worker = dst" convention
//!   when constructing per-pair tiles.
```

The "compute worker = dst" fallback DOES NOT EXIST for this case.
The structural code path is line 2501-2503's `continue; no
transfer` — the pass never reaches per-pair-tile construction.
Cycle-143 commit body called this "off by direction"; architect
P2-3 correction: the doc fabricates a fallback that does not
exist. Rewrite the paragraph to honestly describe the actual
short-circuit + cite the line numbers + cross-reference
TASK-0324.

### AC#1: detection logic

Detect when `producer_workers == &consumer_workers` AND the
consumer's read tile on any non-partition axis would require
slices the local producer does NOT own (i.e., the consumer's
read iv differs from the partition iv on a partitioned axis, or
the consumer's tile bounds exceed the producer's tile bounds).
Equivalent observation: when reader-iv == partition-iv on every
shared axis, the elision is correctness-safe (13-cnn case);
otherwise it is a silent miscompile (06/distributed2 case).

### AC#2: diagnose-first fail-loud guard

Per [[feedback-panic-not-diagnostic-recurring]], the FIRST
landing step is to fail-loud with a typed
`EmitError::ContractGap` ("data X in same-worker-set producer/
consumer where consumer reads outside its produce-tile; this
cross-worker transfer shape is not yet implemented; see
TASK-0324") right at the line-2501 short-circuit. This MUST
land BEFORE any codegen extension — silent-miscompile exposure
is the priority, and a typed error is strictly better than
wrong output even if it temporarily breaks more cells.

The guard MUST be precise enough that 13-cnn/batch_parallel's
correctness-coincides case does NOT spuriously fire. Use the
reader-iv == partition-iv check (or equivalent) to discriminate.

Note: per [[feedback-cross-pass-silent-sibling]], adding
ContractGap rejections has historically unblocked LEGITIMATE
shapes elsewhere (TASK-0268 / TASK-0175). The AC#2 guard must
be measured against the existing pass / skip cell matrix to
confirm no shipped cell newly breaks.

### AC#3: codegen extension

Emit cross-worker `tmp` transfers for this shape. Simplest
correct approach (N-to-N broadcast-of-gather):

- Each producer w_i pushes its hy row-band of tmp to every other
  consumer w_j (4 producers × 4 consumers = 16 pairs, minus 4
  self-pairs if locality is preserved; OR 16 with the self-pair
  as a no-op).
- Each consumer w_j waits on 3 (or 4) row-band pushes and
  assembles them into its full tmp Vec.
- Bit-identical against `reference.bin`.

### AC#4: smoke test promotion

The existing TASK-0298 schedule
(`distributed2.sched.nuc`) becomes the smoke test. Add an e2e
cell once codegen lands; remove the SILENT MISCOMPILE warning
from the schedule's comment header AND remove the four
[[skip]] entries from `nuc-nucleus/e2e-matrix.toml`
(~lines 1265-1304, all citing TASK-0324) AND add four
[[required]] entries in their place.

### AC#5: defensive negative + sibling guard tests

- Add a fixture that constructs the prod-set == cons-set + reader-
  iv-exceeds-producer-tile shape and asserts the cycle-N typed
  error fires (AC#2 hardening).
- Add a fixture that constructs the prod-set == cons-set + reader-
  iv == partition-iv shape (the 13-cnn case) and asserts the
  guard does NOT fire (the correctness-coincides escape valve).

## Honest scope

- **Severity**: HIGH (silent miscompile class is the worst
  failure mode; even a `panic!` would be better).
- **Exposure**: LOW today (no shipped cell triggers this
  defectively; the 13-cnn correctness-coincides case is sound
  by accident but masks the silent-elision path).
- **Priority**: MEDIUM. AC#0 + AC#2 should land quickly to close
  the silent-miscompile window. AC#3 (codegen) can be deferred
  until an M6+ schedule actually needs it.

## Dependencies

- Blocks: TASK-0298 (kept In Progress as smoke-test target).
- Trigger for AC#3: an M6 or later schedule that legitimately
  needs both-passes-distributed shape, OR the eventual M5+
  schedule that lifts TASK-0042 on 13-cnn/batch_parallel
  variants.

## Cross-reference

- TASK-0298 cycle-143 final notes (the reproducer + evidence).
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2501-2503`
  (the precise silent-elision site).
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:82-90`
  (the doc-lie about a fictional "compute worker = dst"
  fallback — AC#0 target).
- `nuc-nucleus/examples/13-cnn-inference/schedules/batch_parallel.sched.nuc:17-22`
  + `nuc-nucleus/examples/13-cnn-inference/prog.algo.nuc:58-60`
  (silent sibling, currently masked by TASK-0042 skip).
- `nuc-nucleus/e2e-matrix.toml` ~lines 464-499 (the TASK-0042
  skip masking the 13-cnn sibling).
- `nuc-nucleus/e2e-matrix.toml` ~lines 1265-1304 (the four
  distributed2 skips filed cycle 143).
- `nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc`
  (the reproducer; carries SILENT MISCOMPILE warning).
- MEMORY.md `feedback-panic-not-diagnostic-recurring` (the
  meta-pattern AC#2 follows).
- MEMORY.md `feedback-silent-sibling-defect` (the meta-rule
  whose 7th firing in this thread caught the 13-cnn sibling at
  review-gate time, not filing time).
- MEMORY.md `feedback-cross-pass-silent-sibling` (the precedent
  for ContractGap-unblocks-legitimate-shapes; informs AC#2
  rollout strategy).
- MEMORY.md `feedback-comment-doc-lie-recurring` (the meta-
  pattern AC#0 closes).

## Cycle-143 architect-review fold-back appendix

The original cycle-143 filing of this task contained three
imprecisions caught by the cycle-143 architect review (P2-1,
P2-2, P2-3) and corrected in-thread before any implementer
picked the task up:

- **P2-1 root-cause precision**: original "the per-pair-tile
  machinery treats each worker's access as same-worker without
  checking whether the consumer's read tile fits the producer's
  write tile" rewritten to the actual line-2501 set-equality
  short-circuit. The pass never reaches tile construction.
- **P2-2 sibling sweep gap**: original filing did not mention
  13-cnn-inference/batch_parallel; architect P2-2 found the
  identical code path firing there masked by the TASK-0042 skip.
  Added as a first-class section + AC#5 sibling-guard test.
- **P2-3 doc-lie magnitude**: original called the
  transfer_inject.rs:82-90 paragraph "off by direction";
  architect P2-3 correction: the "compute worker = dst" fallback
  is fabricated, not directionally wrong. Promoted to AC#0.
- **P3-1 byte-offset reconciliation**: cycle-143 commit body
  said "offset 128"; this description said "byte 129". Both
  correct (cmp 1-based vs offset 0-based, same byte); normalized
  to "cmp 1-based byte 129 (== 0-based offset 128)" throughout.

The fold-back preserves the original AC numbering intent but
adds AC#0 (doc-lie) at the front. Implementer onboarding should
read THIS rewritten description, not look for a separate "v1"
artifact.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 144 implementation plan (orchestrator self-implementing per memory feedback-spawned-agents-refuse-code-edits)

### Cycle scope decision

Land AC#0 (doc-lie fix) + AC#1 (detection logic, internal helper) + AC#2 (fail-loud guard) + AC#5 (both fixtures). DEFER AC#3 (cross-worker tmp codegen extension) and AC#4 (distributed2 smoke promotion) to a follow-up cycle.

Rationale: silent-miscompile exposure is the priority — a typed error is strictly better than wrong output even if it temporarily breaks more cells. AC#3 is a substantial new codegen path (N-to-N broadcast-of-gather); not in scope for one cycle.

### Plan

1. Signature change: `pub fn inject_transfers(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, TransferInjectError>`. Add a new typed enum mirroring the precedent (`apply_halo_inference` / `apply_reuse_inference`).
2. Detection logic (AC#1): at line 2501, when `producer_workers == &consumer_workers`, inspect the consumer's `edge.data_in_access` for this DataId. For each axis where that consumer access uses an IterVar `X`:
   - If `X` is the same iv on which the data was partitioned at the producer (i.e. `acfg.partition_worker_ranges.contains_key(&X)` AND that iv was the partition iv on the producer's corresponding axis), the elision is correctness-safe — continue.
   - Otherwise the consumer references a slice the local producer does NOT own → return Err.
3. Fail-loud guard (AC#2): return `Err(TransferInjectError::SilentElisionRisk { data: DataId, ... })` with a message naming TASK-0324 and the offending DataId.
4. Doc-lie fix (AC#0): rewrite lines 82-90 to describe the actual short-circuit (line 2501-2503 `continue; no transfer`) + cite line numbers + cross-ref TASK-0324. The fabricated 'compute worker = dst' fallback paragraph goes away.
5. Tests (AC#5):
   - Positive (fire) fixture: build a synthetic ACFG matching the 06/distributed2 shape (prod-set==cons-set + reader-iv != partition-iv) → expect Err(SilentElisionRisk).
   - Negative (no-fire) fixture: build a synthetic ACFG matching the 13-cnn/batch_parallel shape (prod-set==cons-set + reader-iv == partition-iv) → expect Ok.
6. Update all ~17 call sites to thread the Result. Production callers `.map_err` to bubble up; tests use `.expect` with an explanatory message.

### Verification gate

- 13-cnn/batch_parallel × {pthreads-sync, pthreads-async, mp-tcp-event} MUST stay green (the negative case must not fire).
- e2e baseline 112/92/0/20/0 (post-cycle-143) MUST hold.
- New positive + negative fixture tests pass.
- The 4 distributed2 [[skip]] entries stay skipped (codegen extension AC#3 still pending) — but the skip reason should still be valid (now a typed error rather than a silent miscompile).

## Cycle 144 landing — orchestrator self-implemented

### What landed (AC#0 + AC#1 + AC#2 + AC#5)

1. **AC#0 doc-lie fix** at `transfer_inject.rs:82-108`: rewrote the bullet that fabricated a 'compute worker = dst' fallback for the N-to-M case. New text honestly describes the line-2501 short-circuit, names the two known shapes (safe: 13-cnn-inference/batch_parallel; unsafe: 06-separable-filter/distributed2), and cross-references this task + the AC#2 validator.

2. **Signature change**: `pub fn inject_transfers(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, TransferInjectError>`. Mirrors the precedent set by `apply_halo_inference` / `apply_reuse_inference` (memory: [[feedback-panic-not-diagnostic-recurring]] — typed errors over panic).

3. **AC#1 + AC#2 validator**: new `check_no_silent_elision_risk` walker called as the FIRST step in `inject_transfers` (front-loaded; the existing recursive emission walk is unchanged for safe shapes). Discriminator is **axis-by-axis against the producer's `data_out_access`**:
   - For each axis `k`: if the producer's write index is a bare `Ident(X)` where X is a partition iv → require consumer's read index on axis k to be a bare `Ident(X)` with the same name (so worker w reads its own slice).
   - For axes where the producer's index is NOT a partition iv → no constraint (axis is whole-array at every worker).
   - For consumer reads with empty indices on partitioned data → reject (whole-array read while worker owns only a slice).
   - First failing axis returns `TransferInjectError::SameSetSilentElisionRisk { data, message }` with a TASK-0324 forward-link and a precise reason string.

4. **AC#5 fixtures** at `tests/transfer_inject.rs:2199+`:
   - `task0324_ac5_positive_fires_on_06_distributed2_shape`: producer writes `tmp[hy][hx]` on {w0..w3} with hy partitioned; consumer reads `tmp[vm][vx]` on {w0..w3} with vm ≠ hy → expects `Err(SameSetSilentElisionRisk { data: D_TMP, .. })` AND verifies the message forward-links TASK-0324 + names the partition-sliced axis.
   - `task0324_ac5_negative_does_not_fire_on_13_cnn_batch_parallel_shape`: producer/consumer chain (cb1/cb2/cls) all on {w0..w3} reading/writing `feat1[n]`/`feat2[n]`/`output[n]` with `loop n : partition=workers` → expects `Ok`.

5. **Call-site propagation**: 83 sites updated. Driver uses `.map_err(|e| format!("transfer-injection error: {e}"))?`. All 79 test sites updated via a Python regex pass to `.expect("inject_transfers")`; 4 edge-cases (chained `.root.collect_xfers()`, tail-expression returns) updated by hand.

6. **e2e-matrix.toml**: 4 distributed2 skip-reason texts updated to reflect AC#2-landed-state (typed compile error rather than silent miscompile). Schedule header at `distributed2.sched.nuc` rewritten with the same correction.

### What is NOT in this cycle (deferred to follow-up cycles)

- **AC#3**: cross-worker `tmp` codegen (N-to-N broadcast-of-gather). Each producer w_i pushes its slice to every other consumer w_j; each consumer w_j assembles from 3 (or 4) pushes. Substantial new codegen surface — its own cycle.
- **AC#4**: matrix promotion. Once AC#3 lands and the output is bit-identical against `reference.bin`, the 4 `[[skip]]` entries become `[[required]]` and the schedule header's warning is removed.

### Verification gate (cycle-144 self-run, pre-parallel-review)

- `just check`: clean.
- `just clippy`: clean.
- `just test`: all tests pass (0 failed across the workspace).
- `just test-release`: all tests pass (release profile guards against debug_assert!-divergence per [[feedback-qa-gate-misses-release-profile]]).
- `just e2e`: 112/92/0/20/0 — IDENTICAL to pre-cycle-143 baseline. No matrix regression.
- `just check-textual-replace-on-codegen`: OK.
- `just check-include-str-coverage`: OK.
- `just ci`: full hard gate green, including all 4 negative/determinism arms (zero-perturbation, determinism, cross-backend differential, required-coverage).

### Gotchas + subtleties (forward-carry for future cycles)

1. **Two attempts at the discriminator**. First version ('any consumer read iv must be the consumer's enclosing partition iv') over-rejected the accumulator-self-read shape `tmp[hy][hx] <-- hblur_acc(..., tmp[hy][hx])` (06/distributed shipped schedule). Four tests in `sidecar_halo.rs` caught this on first `just test`. The fix was to compare against the PRODUCER's `data_out_access` per-axis rather than against the consumer's enclosing scope alone — only axes where the producer writes with a partition iv need to be aligned at the consumer.

2. **`feedback-comment-doc-lie-recurring` self-defense**: the new module-doc bullet (AC#0) names the exact line range (2501-2503), the exact short-circuit (`continue; no transfer`), and both the safe + unsafe shape with example task IDs (13-cnn + 06/distributed2). The TASK-0319 grep-witness discipline applies — anyone editing this code can re-derive whether the bullet still matches reality.

3. **`feedback-silent-sibling-defect` audit during this cycle**: the architect's cycle-143 P2-2 caught 13-cnn-inference/batch_parallel as a silent sibling of 06/distributed2 (same producer-set == consumer-set + line-2501 path; correctness coincidence saved 13-cnn from miscompiling). This cycle's discriminator MUST NOT fire on 13-cnn — AC#5's negative test pins this explicitly. The validator is index-pattern-driven so 13-cnn's reader-iv-equals-partition-iv coincidence is structurally recognised, not a special case.

4. **`feedback-spawned-agents-refuse-code-edits` continues to apply**: orchestrator self-implemented this cycle. No implementer subagent was spawned.

5. **Synthetic test fixtures via `DataflowEdge::new` carry empty `data_in_access` indices**. The validator's discriminator clause for those is to CONTINUE (no producer access info → cannot distinguish safe from unsafe → fall back to pre-TASK-0324 elision). This is safe because synthetic-only fixtures don't exercise the silent-miscompile production code path. For real-fixture tests that DO want to exercise the validator, the test must construct DataflowEdge directly with proper data_in_access / data_out_access (see the 2 new AC#5 fixtures for the template).

### Forward-carries to future tasks (AC#3 follow-up)

- The AC#3 codegen cycle MUST land with the validator's rejection LIFTED simultaneously — otherwise the typed error keeps firing after AC#3 lands. The simplest path: at the validator, when AC#3 emits cross-worker pairs for this shape, the validator either short-circuits (because the emission walk now handles it) or its predicate is relaxed. The cleanest approach is to keep the validator and add a sibling fan-out path in `build_waits_for_op` that emits real cross-worker pairs; the validator then only fires when AC#3 doesn't apply (a residual edge case).

- The conservative-reject shapes (halo `data[n+1]`, const-indexed `data[5]`, arithmetic indices) are NOT exercised by any shipped schedule today. If a future schedule needs them and they ARE safe (e.g. halo+self-read with the halo machinery already extending the worker's tile), the discriminator must be enriched — for now they're rejected.

## Cycle 144 final state — fold-back complete; AC#3 + AC#4 remain

Parallel review gate run (qa-test-runner + mped-architect, both read-only). Both returned GO.

### Architect findings folded back in-thread (commit 70e92ad)

- **P1.2 (honesty)**: cycle-144 AC#0 fix deleted both true (N-to-M tile-rewrite convention at `rewrite_partition_tiles:1689-1698`) AND false (the same-set 'compute worker = dst' fabrication) content. Restored the N-to-M convention as a SEPARATE module-header bullet with explicit cross-reference to the new same-set bullet.
- **P2.2 (defensive)**: `TransferInjectError` now carries `#[non_exhaustive]` so future variants (AC#3 lift / TASK-0325 / TASK-0326) can land without breaking `match` exhaustiveness across the 84 .map_err / .expect call sites. AC#5 positive fixture's match updated for wildcard.
- **P2.4 + P2.5 (grep-witness)**: 'modulo accumulator self-writes' carve-out in `collect_producer_writes` now anchors to `LowerErrorKind::DoubleAssignment` in algo/ir.rs:256-260 (per TASK-0319 future-audit discipline).
- **P1.3 (conservatively-not-rejected)**: the `ident_iv_in_set` -> None branch in the per-axis check now spells out its three sub-cases with TASK-0326 cross-reference for the arithmetic-producer-write under-conservative path.

### Follow-up tracker entries filed

- **TASK-0325** (Medium, M6, silent-sibling): extend validator to the per-element `(src,dst)` same-worker skip in the fan-out loop at transfer_inject.rs:2994. Architect P1.1 — the structurally-identical sibling of the line-2501 set-equality short-circuit that this cycle's validator does NOT cover.
- **TASK-0326** (Low, validator-coverage): tighten discriminator for arithmetic-on-partition-iv producer writes. Architect P1.3 — the dormant under-conservative path.

### Cycle 144 status

**TASK-0324 status: stays In Progress.** AC#0/1/2/5 landed (silent-miscompile path → typed compile error). **AC#3 (cross-worker tmp codegen, N-to-N broadcast-of-gather) + AC#4 (matrix promotion to [[required]] + skip removal) remain.** Reproducer schedule kept at `distributed2.sched.nuc` as smoke-test target for the eventual AC#3 cycle.

### Final verification gate

- `just check`: clean.
- `just clippy`: clean.
- `just test`: all tests pass (dev profile).
- `just test-release`: all tests pass (post-TASK-0291 discipline).
- `just e2e`: 112/92/0/20/0 — IDENTICAL to pre-cycle-144 baseline. 3 back-to-back samples non-flake (qa-runner verified).
- `just check-textual-replace-on-codegen`: OK.
- `just check-include-str-coverage`: OK.
- `just ci` (full hard gate): green including all 4 negative/determinism arms.

### Forward-carried lessons (cycle 144 → future cycles)

1. **The validator-itself was a silent-sibling defect candidate**: cycle 144 wrote a new guard against the line-2501 set-equality short-circuit but did NOT grep for structurally-identical sibling guards (line 2994 per-element skip). Architect caught it. Filed as TASK-0325. Hygiene rule: when adding a new validator / guard / check against a defect class, enumerate every structural variant of the class in the codebase BEFORE writing the validator (memory: feedback-silent-sibling-defect cycle-144 update).

2. **`#[non_exhaustive]` should be added at first-variant-land**: cycle-144 added it AFTER 84 call sites already used the bare `Err` arm. The fix required only 1 site update (the AC#5 positive test) because the others used `.map_err`/`.expect` patterns rather than explicit match. Future enums in this codebase should carry `#[non_exhaustive]` from day one.

3. **AC#0 doc-lie fixes must preserve true content adjacent to false content**: the cycle-143 architect's P2-3 framing 'fabricated fallback' was correct for the same-set case but did NOT mean 'no compute-worker convention exists at all'. The cycle-144 implementer (orchestrator) over-corrected. Architect P1.2 caught it. Hygiene rule: when deleting a doc-lie, the same edit must verify that the surrounding true content remains intact OR is moved to where it still applies.

## Cycle-145 review-fold-back forward-carry (architect P3.1)

When AC#3 lifts the SameSetSilentElisionRisk variant (cross-worker tmp codegen lands; same-worker elision is no longer a silent miscompile), the variant NAME 'SameSetSilentElisionRisk' will be misleading — it currently also fires under partial worker-set overlap (post-cycle-145 generalisation). Two options at AC#3 land-time:

1. Rename to SameWorkerElisionRisk or SilentElisionRisk (variant rename, ~85 sites updated via the same #[non_exhaustive] discipline cycle-144 established).
2. Accept as a fixed-compatibility name and document the historical context.

Note for future AC#3 implementer: choose at AC#3 land-time; rename is the more honest option, but the name has a clean public-API status (only crate-internal callers exist today).

Cycle-145 reference: architect read-only review P3.1 cycle 145.
<!-- SECTION:NOTES:END -->
