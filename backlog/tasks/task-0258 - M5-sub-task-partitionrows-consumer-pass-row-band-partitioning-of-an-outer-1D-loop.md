---
id: TASK-0258
title: >-
  M5 sub-task: partition=rows consumer pass (row-band partitioning of an outer
  1D loop)
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:53'
updated_date: '2026-05-24 00:01'
labels:
  - M5
  - compiler
  - partition
dependencies:
  - TASK-0043
  - TASK-0249
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#1. partition=rows is currently REJECTED at sched-lower as UnsupportedPartitionKind (TASK-0249 cycle 70). M5 needs a real consumer.

## Scope
Add nucleus/nucleus-compiler/src/passes/partition_rows.rs as a sibling to passes/partition_workers.rs. Walks the ACFG, finds Repeat nodes with ResolvedLoopOption::Partition(PartitionKind::Rows), and partitions the OUTER iteration range across the placement's worker set (round-robin band assignment by default).

## Acceptance Criteria
1. partition_rows pass exists; called from passes/mod.rs in the canonical pass order.
2. A 1D outer Repeat with partition=rows on a place-set of N workers gets per-worker row-band ranges in NameSidecar.partition_worker_ranges (same shape partition=workers uses today).
3. partition=rows on a NON-1D loop is rejected at sched-lower as a typed error (UnsupportedPartitionKind or a new variant — matches PRD §6.3.3 'bad combinations rejected at compile time').
4. UnsupportedPartitionKind for Rows is REMOVED from sched-lower (TASK-0249 reject becomes accept-and-route-to-consumer).
5. A new e2e cell exercises partition=rows on examples 5 or 6; bit-identical vs reference.bin on at least one tier-1 backend.

## Open questions
- Round-robin row-band vs strict equal-band assignment for non-divisible row counts. Default: same trailing-partial discipline that block_transform.rs uses (TASK-0218 / TASK-0181).
- Halo inference for stencil examples (5, 6) is TASK-0043 AC#2 — sibling task, not this one.

## Forward-carry from TASK-0249
The reject site at sched/lower.rs::lower_loop_option (the PartitionKind::Rows arm of UnsupportedPartitionKind) must be REMOVED when this consumer lands; otherwise the schedule never reaches the partition_rows pass. Same surgical edit pattern partition_workers used when it landed.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DESCRIPTION CORRECTION + clarification (orchestrator cycle 79b, pre-implementer): the original description said 'row-band partitioning of an outer 1D loop'. PRD §6.3.3 line 519 is explicit: 'partition=rows on a 1D iteration' is a BAD COMBINATION rejected at compile time. partition=rows is specifically for the OUTER of a 2D nest — it row-bands the outer (y) loop, leaving the inner (x) loop intact per worker. This is the original 05-stencil/distributed use case TASK-0249 surfaced (the inert  directive on a 2D y/x nest).

Refined scope for the implementer:
1. partition_rows pass applies ONLY when the partition=rows directive is on the OUTER loop of a 2D nest (Repeat-of-Repeat in the ACFG, on the same worker entity). Reject otherwise at sched-lower OR at the pass entry, with a typed UnsupportedPartitionKindFor1DLoop variant (NOT the existing UnsupportedPartitionKind blanket reject — that becomes too coarse).
2. Semantics: row-band the outer-loop range across the placement workers (same algorithm partition_workers uses for 1D, but applied to the outer of the 2D); inner loop body executes unchanged per worker.
3. Output: NameSidecar.partition_worker_ranges[outer_iv][worker_id] = row_band_range, exactly as partition_workers populates today. No NEW sidecar field needed — transfer_inject + the backend walker already consume partition_worker_ranges and apply per-worker slice handling (host-side gather via render_wait_assign).
4. The reject site at sched/lower.rs::lower_loop_option's PartitionKind::Rows arm: REMOVE the UnsupportedPartitionKind reject for Rows (keep for Blocks2d until TASK-0259 lands). Replace with an accept-and-route-to-consumer arm.
5. The NEW reject site (typed UnsupportedPartitionKindFor1DLoop or similar) fires when partition=rows is applied to a non-outer-of-2D context. Test both negative paths.

This is mostly a 'wire partition=rows through the existing partition_workers infra' task; the heavy lifting (per-worker range -> sidecar -> emit) already exists. Estimated scope: ~150-250 LoC including tests, mostly mechanical.

Halo inference (TASK-0260) is a SIBLING task — partition_rows alone does NOT solve the stencil halo problem; without halo widths, a row-band-partitioned stencil produces wrong output at the band boundaries. Plan ahead: when this task lands and an e2e cell is added, ensure either (a) the cell's algorithm has no halo (the cell verifies partition=rows mechanism only), or (b) the cell SKIPS until TASK-0260 lands. Pure partition=rows without halo will produce incorrect output on stencils — do NOT mark the cell [[required]] until halo inference is also wired.

Implementation Plan (cycle 79c — implementer):

1. NEW pass file: nucleus/nucleus-compiler/src/passes/partition_rows.rs
   - Mirrors partition_workers.rs shape: `apply_partition_rows(&LinkedIR, ACFG) -> Result<ACFG, PartitionRowsError>`.
   - Errors: PartitionRowsError with 4 variants:
       UnknownLoopVar  (linker-invariant)
       NotOuterOf2DNest  (PRD §6.3.3 'partition=rows on 1D iteration': category error)
       NoMultiWorkerBody (mirrors PartitionError)
       NonDivisible      (mirrors PartitionError)
   - Algorithm: walk ACFG, find Repeat with ResolvedLoopOption::Partition(PartitionKind::Rows). Verify body contains an inner Repeat structurally (Repeat-of-Repeat via find_outer_with_inner_repeat helper, peeking through Sequence). Verify that inner-Repeat body's worker union >= 2. Apply row-band slicing (same divisible/round-robin algorithm partition_workers uses on the outer iter_var). Write into ACFG.partition_worker_ranges[outer_iv][worker_id].

2. Wire into nucleus/driver/src/main.rs: import `apply_partition_rows`; call IMMEDIATELY after apply_partition_workers (line 332). Both consume + return ACFG; pure sequential composition.

3. nucleus/nucleus-compiler/src/passes/mod.rs: add `pub mod partition_rows;` next to `partition_workers`.

4. nucleus/nucleus-compiler/src/lib.rs: add `pub use passes::partition_rows::{apply_partition_rows, PartitionRowsError};` next to the partition_workers export.

5. sched-lower change in nucleus/nucleus-compiler/src/sched/lower.rs:1120-1133:
   - Remove the `PartitionKind::Rows` arm from the alternation; only Blocks2d rejects now.
   - Update doc-comment to reflect that Rows now lowers and routes to the partition_rows consumer.
   - Display message in src/sched/ir.rs:816..835 updated: when kind == Rows, this is unreachable (won't fire) but encoded for exhaustiveness — keep the message accurate by keeping the keyword mapping; remove 'rows' from the actionable suggestion (replace with 'omit the directive').

6. Test files:
   (a) nucleus/nucleus-compiler/tests/partition_rows.rs — new file. Mirror partition_workers.rs tests shape:
       - positive: synthetic 2D Repeat-of-Repeat, partition=rows on outer over 4-worker body. Per-worker ranges 0..4, 4..8, 8..12, 12..16 for source range 0..16.
       - negative_1d_iter_rejected: synthetic 1D Repeat (no inner Repeat in body) → NotOuterOf2DNest.
       - negative_single_worker_body: synthetic 2D Repeat-of-Repeat with single-worker body → NoMultiWorkerBody.
       - negative_non_divisible: synthetic 2D Repeat-of-Repeat, range 0..17 across 4 workers → NonDivisible.
       - positive_deterministic_two_runs: byte-identical between two runs (BTreeMap discipline).
   (b) nucleus/nucleus-compiler/tests/sched_lower.rs: 
       - rename existing `negative_partition_rows_is_rejected` → `positive_partition_rows_now_lowers` and flip the assertion: lowers ok, includes ResolvedLoopOption::Partition(PartitionKind::Rows).
       - keep `negative_partition_blocks2d_is_rejected` unchanged (Blocks2d still rejects).
       - keep `positive_partition_workers_still_lowers` unchanged.

7. nucleus/nucleus-compiler/tests/sched_parser.rs + tests/sched_lower.rs: `parses_05_stencil_distributed` and `lowers_05_stencil_distributed` expect count_loops()/loops.len() == 1 today. After restoring the y-directive: count is 2 and the y-loop options include ResolvedLoopOption::Partition(PartitionKind::Rows). Update comment to cite TASK-0258 (consumer landed) instead of TASK-0249 (silent-drop closed).

8. nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc:
   - Re-introduce `loop y : partition=rows;` after the algo's outer y loop.
   - Rewrite header NOTE block: TASK-0249 removed the directive because no consumer existed; TASK-0258 restored it now that partition_rows lands. Cell remains [[skip]] (TASK-0117 / TASK-0042.05 / halo are sibling gates). 
   - Footer note: halo inference (TASK-0260) is the remaining barrier to a bit-identical stencil cell.

9. Update partition_workers.rs:40 caveat comment: 'partition=rows now consumed by passes/partition_rows.rs (TASK-0258)'. Keep `Blocks2d rejects at sched-lower as UnsupportedPartitionKind`.

10. Verification gate (run via nix develop -c just <recipe>):
    a. just test
    b. just clippy (cargo clippy --workspace --all-targets -- -D warnings)
    c. cd nucleus && cargo fmt --check -p nucleus-compiler
    d. just e2e (88/?/0/? — must preserve 0 required-fail and 0 failures)
    e. just determinism-check
    f. just determinism-check-negative
    g. just xbackend-check-negative

11. Commits in 2-3 logical units:
    a. passes/partition_rows.rs + mod.rs + lib.rs export + driver wire-up
    b. sched/lower.rs + sched/ir.rs accept-Rows update + tests update
    c. 05-stencil/distributed.sched.nuc restoration + parser/lower tests update

12. Out of scope (will file follow-ups on commit):
    - Halo inference (TASK-0260 already filed)
    - Stencil e2e cell exercising partition=rows + halo bit-identical to reference.bin (blocked-on TASK-0260)
<!-- SECTION:NOTES:END -->
