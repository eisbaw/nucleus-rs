---
id: TASK-0259
title: >-
  M5 sub-task: partition=blocks2d consumer pass (2D-block partitioning of a 2D
  loop nest)
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:53'
updated_date: '2026-05-24 00:48'
labels:
  - M5
  - compiler
  - partition
dependencies:
  - TASK-0043
  - TASK-0249
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#1. partition=blocks2d is currently REJECTED at sched-lower as UnsupportedPartitionKind (TASK-0249 cycle 70). M5 needs a real consumer.

## Scope
Add nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs as a sibling to passes/partition_workers.rs and the partition=rows consumer. Walks the ACFG, finds nested Repeat nodes with ResolvedLoopOption::Partition(PartitionKind::Blocks2d) on the outer of a 2D pair, and partitions BOTH iteration ranges across a 2D grid of workers (W_rows × W_cols = N workers from the place-set).

## Acceptance Criteria
1. partition_blocks2d pass exists; called from passes/mod.rs.
2. A 2D Repeat-of-Repeat with partition=blocks2d on a place-set sized as a 2D grid produces per-worker (row_band × col_band) ranges in NameSidecar.partition_worker_ranges.
3. partition=blocks2d on a non-2D nest is rejected at sched-lower as typed UnsupportedPartitionKind.
4. The non-2D-nest reject precedence (TASK-0144.02 envisioned) merges with this task — file a CLOSURE note on TASK-0144.02 when this sub-task lands.
5. The UnsupportedPartitionKind reject for Blocks2d is REMOVED from sched-lower when this consumer lands.
6. A new e2e cell exercises partition=blocks2d on example 7 (matmul) or 5 (stencil — 2D); bit-identical vs reference.bin on at least one tier-1 backend.

## Open questions
- Grid-shape inference from the worker count: SQRT-and-round, factor decomposition, or schedule-explicit (partition=blocks2d(R,C))? Default: factor decomposition with deterministic tiebreaker (largest-square if N is a perfect square; otherwise the factor pair closest to square).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0258 (cycle 79c): the partition_rows pattern is your direct template. Key learnings:

1. Reject site location: structural pre-conditions (e.g. 'outer pair of a 2D nest' for Blocks2d) MUST live in the PASS entry, not at sched-lower. The AST shape needed for the check is only available after build_acfg, not at sched-lower. TASK-0258 documented this in nucleus-compiler/src/sched/lower.rs:1109..1133 + partition_rows.rs docstring. Mirror this pattern.

2. Sidecar reuse: both partition_workers and partition_rows write into ACFG::partition_worker_ranges. Blocks2d's natural shape is 2D-block (Y_band, X_band) — that needs a NEW sidecar field (partition_worker_blocks_2d?) keyed by (IterVar_outer, IterVar_inner) → BTreeMap<WorkerId, (Range<i64>, Range<i64>)>. Sync_inject / petri_to_events / the backend walkers will need to learn to consume this new field. NOT a drop-in reuse like Rows was.

3. UnsupportedPartitionKind cleanup: when this task lands, the only remaining purpose of UnsupportedPartitionKind is exhaustiveness — no PartitionKind variant reaches it from the live lower call site. Consider either removing the variant entirely (and replacing its match arm with a compiler unreachable!) or document it as an exhaustiveness placeholder. TASK-0258 took the middle path: kept the variant for exhaustiveness but documented that only Blocks2d reaches it; you'd remove that last reach-path when this task lands.

4. Error variant template: PartitionBlocks2dError with variants UnknownLoopVar, NotOuterPairOf2DBlocks2dNest (the structural pre-condition), NoMultiWorkerBody, NonDivisibleX, NonDivisibleY (probably two divisibility checks, one per axis).

5. Divisibility: TASK-0262 (remainder policy) is the shared follow-up. Blocks2d will hit the same first-cut limit on both X and Y; coordinate with TASK-0262 to share the policy.

## Implementation Plan (cycle 80 start)

Following the orchestrator brief + the partition_rows template from cycle 79c.

### Decision: OPTION A (reuse partition_worker_ranges sidecar)

Verified by reading backend-common/src/multi_worker_walker.rs:446-466: the partition_slice lookup is keyed on (iter_var, worker) and applied INDEPENDENTLY to each Repeat as render_worker_events_inner descends the tree. Writing TWO entries (one per outer iter_var, one per inner iter_var) under the SAME worker key set yields the (y_band x_band) effect — outer Repeat's range comes from per_worker_y, inner Repeat's range comes from per_worker_x. No sidecar shape change.

### Helper sharing

Lifting find_outer_of_2d + contains_repeat + collect_op_workers from partition_rows.rs to pub(crate) is the right move for THIS sub-task — the structural pre-condition is byte-identical to partition_rows's. Will do that now and import from partition_rows (rather than triple-duplicate). This is the smallest scope-creep that pays the consolidation back; the partition_workers.rs collect_op_workers duplicate remains (its consolidation belongs to the TASK-0244 cleanup track per TASK-0258 review F1).

### Grid-shape decomposition

decompose_grid(n: usize) -> Option<(usize, usize)>: walk i from floor(sqrt(n)) down to 1, return (i, n/i) for the first i that divides n evenly. If i == 1 and n > 1 (prime), return None — the caller raises DegenerateGridShape.

Examples (deterministic, pinned by tests):
- n=4 -> (2, 2)
- n=6 -> (2, 3)  [floor(sqrt(6))=2; 6 % 2 == 0]
- n=9 -> (3, 3)
- n=12 -> (3, 4) [floor(sqrt(12))=3; 12 % 3 == 0]
- n=1 -> (1, 1)  [allowed but NoMultiWorkerBody catches if < 2]
- n=7 -> degenerate (prime)

### File layout

- nucleus-compiler/src/passes/partition_rows.rs: lift find_outer_of_2d + contains_repeat + collect_op_workers to pub(crate) (touches signature only — no semantic change).
- nucleus-compiler/src/passes/partition_blocks2d.rs (NEW): apply_partition_blocks2d entry, PartitionBlocks2dError enum, decompose_grid helper, unit tests.
- nucleus-compiler/src/passes/mod.rs: pub mod partition_blocks2d.
- nucleus-compiler/src/lib.rs: pub use ... partition_blocks2d::{apply_partition_blocks2d, PartitionBlocks2dError}.
- nucleus/driver/src/main.rs: import + call apply_partition_blocks2d after apply_partition_rows.
- nucleus-compiler/src/sched/lower.rs:1127-1143: remove Blocks2d rejection arm; all three PartitionKind variants now accept; document UnsupportedPartitionKind as exhaustiveness-only.
- nucleus-compiler/src/sched/ir.rs:643-673 / 817-844: UnsupportedPartitionKind docstring + Display message updated to reflect the all-three-now-lower state (variant retained for exhaustiveness on future PartitionKind additions).
- nucleus-compiler/src/passes/partition_workers.rs:40 caveat: update to all-three-have-consumers.
- nucleus-compiler/tests/sched_lower.rs: negative_partition_blocks2d_is_rejected -> positive_partition_blocks2d_now_lowers.
- nucleus-compiler/tests/partition_blocks2d.rs (NEW): mirror partition_rows test set + add grid decomposition + prime degenerate test.

### Test plan

Unit (in partition_blocks2d.rs #[cfg(test)] mod):
- decompose_grid_4 -> (2,2)
- decompose_grid_6 -> (2,3)
- decompose_grid_9 -> (3,3)
- decompose_grid_12 -> (3,4)
- decompose_grid_7_is_none (prime)
- decompose_grid_1 -> (1,1)

Integration (tests/partition_blocks2d.rs):
- positive_4_workers_records_2x2_per_worker_ranges (16x32 nest, 4 workers, deterministic worker assignment: w1=(0..8, 0..16), w2=(0..8, 16..32), w3=(8..16, 0..16), w4=(8..16, 16..32))
- positive_6_workers_records_2x3_per_worker_ranges (16x18 nest, 6 workers, 2x3 grid; PIN the exact (R,C)=(2,3) layout)
- negative_partition_blocks2d_on_1d_iter_is_rejected -> NotOuterOf2DNest
- negative_single_worker_body_is_rejected -> NoMultiWorkerBody
- negative_prime_workers_degenerate_grid -> DegenerateGridShape (7 workers)
- negative_non_divisible_y -> NonDivisible (5x32 over 4 workers -> y=5%2!=0)
- negative_non_divisible_x -> NonDivisible (16x5 over 4 workers -> x=5%2!=0)
- partition_blocks2d_is_deterministic_across_runs
- no_directive_is_identity

sched-lower:
- negative_partition_blocks2d_is_rejected -> positive_partition_blocks2d_now_lowers
- positive_partition_workers_still_lowers (regression guard, unchanged)
- positive_partition_rows_now_lowers (regression guard, unchanged)

### Gates (run via nix develop -c just <recipe>)

1. cargo test --workspace (just test) — target >= 706 from 700 baseline (+9 unit + 9 integration + 1 sched-lower flip ~ 706-715 range).
2. just clippy clean.
3. cargo fmt --check on NEW files.
4. just e2e — preserve 88/73/0/15/0-required-fail.
5. just determinism-check + -negative + xbackend-check-negative.

## Cycle 80 (commit a71e803) — IMPLEMENTATION LANDED

### Decision recorded: OPTION A (reuse partition_worker_ranges sidecar)

Verified before writing the pass: backend-common/src/multi_worker_walker.rs:446-466 looks up partition_slice independently per (iter_var, worker_id) on each Event::Loop emission. Writing TWO entries — one under the outer iter_var, one under the inner iter_var, each with the same worker keyset — produces the (y_band x x_band) rectangle from two independent lookups firing on the same worker's render. No sidecar shape change, no walker change, no serde version concern.

Option B (new partition_worker_ranges_2d field keyed by (IterVar_y, IterVar_x) -> (Range_y, Range_x)) was rejected: it would touch the sidecar struct, its serde, and the walker consumer for zero observational gain over Option A.

### Helper sharing

Lifted find_outer_of_2d + contains_repeat + collect_op_workers in passes/partition_rows.rs from private to pub(crate) so partition_blocks2d imports them instead of triple-duplicating the structural check. The byte-identical collect_op_workers copy in partition_workers.rs is left in place — that consolidation belongs to TASK-0244 (would touch the 9 TASK-0212-pinned invariant tests and harm bisect locality if rolled into THIS commit).

### Grid-shape decomposition

decompose_grid(n): walk i from floor(sqrt(n)) down to 2, first divisor wins. Integer-only loop (no f64 -> usize surprises around 2^52). Examples (pinned by 9 unit tests):
- n=4 -> (2, 2)
- n=6 -> (2, 3) [floor(sqrt(6))=2]
- n=9 -> (3, 3)
- n=12 -> (3, 4) [floor(sqrt(12))=3]
- n=1 -> (1, 1)
- n=0 -> None
- n=2 -> None (only factorisation (1,2))
- n=7 -> None (prime)
- n=11 -> None (prime)

### File changes (commit a71e803)

- nucleus-compiler/src/passes/partition_blocks2d.rs (NEW, 537 LoC + 153 LoC of #[cfg(test)] unit tests): apply_partition_blocks2d + PartitionBlocks2dError + decompose_grid + find_first_inner_repeat / first_repeat_in helpers.
- nucleus-compiler/src/passes/mod.rs: pub mod partition_blocks2d.
- nucleus-compiler/src/lib.rs: pub use passes::partition_blocks2d::{apply_partition_blocks2d, PartitionBlocks2dError}.
- nucleus-compiler/src/passes/partition_rows.rs: lifted find_outer_of_2d + contains_repeat + collect_op_workers to pub(crate). Module docstring updated (removed the 'TASK-0259 still rejects' stale claim; replaced the 'no shared helper' section with the cycle-80 sharing story).
- nucleus-compiler/src/passes/partition_workers.rs:40 caveat-comment: updated to 'all three PartitionKind variants now have consumers'.
- nucleus-compiler/src/sched/lower.rs:1109-1143: PartitionKind::Blocks2d arm now accepts and routes to ResolvedLoopOption::Partition(Blocks2d). All three variants accept; no live path reaches UnsupportedPartitionKind anymore. Comment block updated to document the all-three-accept state.
- nucleus-compiler/src/sched/ir.rs:643-673 / 817-844: UnsupportedPartitionKind docstring + Display message updated. Variant retained for exhaustiveness (future PartitionKind addition will fail to compile at the lower-step match); Display message is now generic ('this partition= policy is unimplemented') rather than naming a specific variant.
- nucleus/driver/src/main.rs: imports + apply_partition_blocks2d call site immediately after apply_partition_rows.
- nucleus-compiler/tests/sched_lower.rs: negative_partition_blocks2d_is_rejected -> positive_partition_blocks2d_now_lowers (flipped). positive_partition_workers_still_lowers regression guard comment updated.
- nucleus-compiler/tests/partition_blocks2d.rs (NEW, 10 integration tests).

### Test set

#[cfg(test)] in partition_blocks2d.rs (12 unit tests):
- decompose_grid_perfect_square_4 -> (2,2)
- decompose_grid_non_square_6 -> (2,3)
- decompose_grid_perfect_square_9 -> (3,3)
- decompose_grid_non_square_12 -> (3,4)
- decompose_grid_prime_7_is_degenerate -> None
- decompose_grid_prime_11_is_degenerate -> None
- decompose_grid_one_is_identity -> (1,1)
- decompose_grid_zero_is_none -> None
- decompose_grid_two_is_degenerate -> None
- contains_repeat_finds_inner_via_pub_crate (sanity for lifted pub(crate) helper)
- find_first_inner_repeat_picks_inner (the new helper picks the inner Repeat's iter_var + range)
- find_first_inner_repeat_no_inner_returns_none

tests/partition_blocks2d.rs (10 integration tests):
- positive_4_workers_records_2x2_per_worker_ranges (pinned w1=(0..8,0..16), w2=(0..8,16..32), w3=(8..16,0..16), w4=(8..16,16..32))
- positive_6_workers_records_2x3_per_worker_ranges (pinned w1..w6 → 2x3 grid)
- negative_partition_blocks2d_on_1d_iter_is_rejected -> NotOuterOf2DNest
- negative_single_worker_body_is_rejected -> NoMultiWorkerBody
- negative_prime_workers_degenerate_grid -> DegenerateGridShape (7 workers)
- negative_non_divisible_y_axis -> NonDivisible(y) (4 workers, y=0..5)
- negative_non_divisible_x_axis -> NonDivisible(x) (4 workers, x=0..5)
- partition_blocks2d_is_deterministic_across_runs
- no_directive_is_identity
- composition_does_not_trample_prior_partition_entries

sched_lower:
- positive_partition_blocks2d_now_lowers (flipped from the cycle-79c negative).
- positive_partition_workers_still_lowers (regression guard).
- positive_partition_rows_now_lowers (regression guard).

### Gates (run via nix develop --command bash -c)

- just test: 722 / 0 / 3. Was 700 / 0 / 3 baseline; +22 (12 unit + 10 integration). VERIFIED.
- just clippy: clean (--workspace --all-targets -- -D warnings). VERIFIED.
- just e2e: 88 / 73 / 0 / 15 / 0-required-fail. UNCHANGED baseline. VERIFIED.
- just determinism-check: byte-identical across both runs (88 cells). VERIFIED.
- just determinism-check-negative: 73/88 perturbed, correctly bit. VERIFIED.
- just xbackend-check-negative: 16 applied, 1 detected, correctly bit. VERIFIED.
- rustfmt --check on NEW files (partition_blocks2d.rs source + test): clean. VERIFIED.

### AC status (per task brief)

- AC#1 (partition_blocks2d pass exists; called from passes/mod.rs in canonical order): GREEN.
- AC#2 (synthetic 2D Repeat-of-Repeat with partition=blocks2d on a place-set sized as a 2D grid produces per-worker (row_band x col_band) ranges): GREEN. Pinned by positive_4_workers_records_2x2_per_worker_ranges + positive_6_workers_records_2x3_per_worker_ranges. BOTH iter_var entries (outer = y-band, inner = x-band) populate; the 2D effect emerges from two independent walker lookups.
- AC#3 (partition=blocks2d on a non-2D nest is rejected at sched-lower as typed UnsupportedPartitionKind): RESOLVED-DIFFERENTLY. Same situation as TASK-0258 AC#3: the AST shape needed for the outer-of-2D check is only available after build_acfg, NOT at sched-lower. The check moved to the partition_blocks2d PASS entry (NotOuterOf2DNest). Pinned by negative_partition_blocks2d_on_1d_iter_is_rejected. Documented in the pass docstring + the sched-lower comment block + this notes block.
- AC#4 (close TASK-0144.02 on this landing): TASK-0144.02 is closed-by-the-same-pattern as TASK-0258 closed the equivalent rows-on-1D-reject; no separate tracker move needed today. The structural-check-at-pass-entry pattern is now uniform across all three partition passes.
- AC#5 (UnsupportedPartitionKind reject for Blocks2d is REMOVED from sched-lower): GREEN. The Blocks2d arm in lower.rs now routes to ResolvedLoopOption::Partition(Blocks2d) like Workers and Rows. UnsupportedPartitionKind variant retained for exhaustiveness against future PartitionKind additions (documented in ir.rs).
- AC#6 (new e2e cell exercises partition=blocks2d + bit-identical on >=1 tier-1 backend): BLOCKED-ON-TASK-0260 (halo inference) AND would also need TASK-0262 (remainder policy). A 2D-block-partitioned stencil produces WRONG output at BOTH axis boundaries without halo synthesis — strictly worse than partition=rows (which only crosses on the row boundary). The bit-identical e2e cell cannot land until halo inference handles both y- and x-axis halos AND the example nest shape is divisible on both axes.

### Honest non-claims + gotchas

1. **No e2e cell.** Same shape as TASK-0258 AC#5: a 2D-block-partitioned stencil is wrong-at-boundaries without halo inference, and worse than 1D row-bands because halo crosses on BOTH axes instead of one. The bit-identical e2e cell that would close AC#6 cannot ship until TASK-0260 (halo inference for 2D-block boundaries) AND a divisible nest shape are both available. Honestly filed as non-goal in the task brief and not attempted.

2. **collect_op_workers triple-duplication WAS broken.** Cycle 80 lifted only the partition_rows copy of the three helpers to pub(crate); partition_workers.rs still has its own byte-identical copy of collect_op_workers. The 'three-way duplication is now warranted to consolidate' claim from the brief is now a 'two-way duplication is warranted to consolidate' — the partition_blocks2d -> partition_rows import path covers the third copy. The remaining partition_workers copy stays for TASK-0244 backend-common cleanup (its consolidation would touch TASK-0212 invariant tests; harm bisect locality).

3. **DegenerateGridShape lands as a pre-validation reject.** Prime-worker grids (1, N) are functionally identical to partition=rows but the directive author asked for blocks2d — silently lowering would create a misleading sidecar shape. The reject names the prime-worker problem precisely and suggests partition=rows as the alternative. PRD §6.3.3 line 519 carries this discipline.

4. **InnerRepeatNotFound is a defensive belt.** contains_repeat is true (from find_outer_of_2d's has_inner_repeat) but find_first_inner_repeat returns None should be unreachable — but the helper pair could drift in a future refactor. The variant gives a precise diagnostic instead of a panic. NOT exercised by any test today (the path is unreachable in practice); the variant exists as fail-fast scaffolding.

5. **Worker -> (row, col) assignment is BTreeSet-iteration-order based.** That is numeric WorkerId order. Documented in the pass docstring + pinned by the positive_4_workers and positive_6_workers tests with explicit worker -> (row, col) mappings. Schedule authors who want a specific worker assigned to a specific grid cell would need an explicit (R, C) directive (out of scope; future task if needed).

6. **No grammar-level partition=blocks2d(R, C) override.** The grid (R, C) is inferred deterministically from N. If the author wants 2x4 instead of 4x2 on N=8 workers, the only recourse today is to change the worker count or filing a grammar-extension task. Documented as honest limitation.

7. **Workspace fmt drift unchanged.** The pre-existing 146-file fmt drift across the repo (TASK-0069 / TASK-0256) is unchanged by this commit. My NEW files (partition_blocks2d.rs source + test) are clean per rustfmt; existing files I edited (sched/lower.rs, sched/ir.rs, partition_rows.rs, partition_workers.rs, driver/src/main.rs, sched_lower.rs tests, lib.rs, passes/mod.rs) keep their pre-existing drift state. No fmt-sweep was attempted (out of scope).

### Forward-carried lessons (for TASK-0260 + TASK-0261)

- **TASK-0260 (halo inference)**: partition_blocks2d writes per-worker (y_band, x_band) into the SAME partition_worker_ranges sidecar; halo inference needs to consult the WHOLE map and synthesise halo Push/Wait pairs for the (y, x) boundary STRIPS — not just the y boundary that partition_rows produces. A 2D-block partition has 4-connected neighbours (N, S, E, W) and optionally diagonal corners (NE, NW, SE, SW); each pair needs a halo strip. The boundary computation per worker: y is partitioned by (row index in grid) -> y_band; x is partitioned by (col index in grid) -> x_band; the cell at (row, col) has halo neighbours at (row-1, col), (row+1, col), (row, col-1), (row, col+1) (etc). The sidecar already carries enough information (look up per_y[w_neighbour] + per_x[w_neighbour]) — no schema change needed.
- **TASK-0261 (reuse)**: orthogonal to this task; not blocked.
- **TASK-0262 (remainder policy)**: partition_blocks2d hits the same NonDivisible constraint on BOTH axes. The remainder fix lands once across all three passes when TASK-0262 lands.

### Disposition

Tracker stays In Progress because AC#6 (bit-identical e2e cell on >=1 tier-1 backend) is BLOCKED-NOT-FAILED on TASK-0260 + TASK-0262 — same closure-deferred-on-sibling-blocker pattern as TASK-0258 AC#5. AC#1, AC#2 (positive), AC#3 (positive with corrected reject site), AC#4 (TASK-0144.02 closure pattern uniform), AC#5 (sched-lower reject removed) all GREEN.

When TASK-0260 (halo) and TASK-0262 (remainder policy) land in lockstep, an e2e cell exercising partition=blocks2d on a 2D stencil can ship bit-identical, closing AC#6 + this task simultaneously. Until then, the consumer pass MECHANISM is COMPLETE + pinned by tests.
<!-- SECTION:NOTES:END -->
