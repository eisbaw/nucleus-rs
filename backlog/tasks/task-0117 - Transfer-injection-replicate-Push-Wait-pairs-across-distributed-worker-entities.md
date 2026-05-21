---
id: TASK-0117
title: >-
  Transfer-injection: replicate Push/Wait pairs across distributed worker
  entities
status: Done
assignee:
  - '@claude'
created_date: '2026-05-18 01:44'
updated_date: '2026-05-21 06:27'
labels:
  - M1
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
transfer_inject collapses 'place k on {w0,w1,w2,w3}' into a single canonical WorkerId for the src/dst on XferPlaceholder. A future partition pass (TASK-0016+ alignment) needs to fan out one Push/Wait pair per worker in the set, partitioned by loop.partition=... policy. Spec: when crossing from a singleton {host} into a distributed entity {w0..w3}, the schedule's transfer directive plus the loop.partition= should produce N pairs, each carrying its slice of the IterTile.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-1 IMPLEMENT (claude, 2026-05-21)

Landed: per-(src, dst) Push/Wait fan-out at `transfer_inject` with per-worker tile sink, backend per-(DataId, SeqTag) slot allocation + tile-aware host-side gather, and a sync-injection co-fix to suppress per-iteration body Syncs on partitioned Repeats. Example 13 batch_parallel × pthreads-sync now cargo-builds and is byte-identical to reference.bin.

### Architecture (file:line citations)

1. Fan-out in transfer_inject (`build_waits_for_op`): `nucleus/compiler/src/passes/transfer_inject.rs:1494..1531`
   - Emits one Wait per (src, dst) member of producer × consumer cartesian product, skipping same-worker pairs.
   - Each pair gets a fresh `SeqTag` from `state.fresh_seq()` (monotonic; same input ⇒ same seqs).
   - Initial tile = `enclosing_tile`; rewritten by the partition sink below.

2. Per-worker tile sink (post-hoist, post-splice): `transfer_inject.rs:1453..1547`
   - New `rewrite_partition_tiles` walks the final ACFG once, finds each Xfer, picks the "compute worker" (whichever of src/dst appears in `partition_worker_ranges`), and rewrites the tile to `[(iv, partition_ranges[iv][compute_worker])]` for each partitioned axis.
   - Doing it post-hoist preserves the TASK-0151 hoist-tile-rewrite invariant; an empty sidecar ⇒ no rewrite ⇒ pre-TASK-0117 behaviour for non-partitioned schedules (01..07).

3. Backend slot keying by (DataId, SeqTag): `nucleus/backends/pthreads-sync/src/multi_worker.rs:120..130, 175..190`
   - `slot_ids: BTreeMap<(DataId, SeqTag), SlotId>` replaces `BTreeMap<DataId, SlotId>`.
   - `pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>` records each pair's tile for the gather codegen.
   - For examples with one pair per data symbol the slot indices are byte-identical to pre-TASK-0117 (BTreeSet sorts by DataId first).

4. Tile-aware host-side gather: `multi_worker.rs:632..761`
   - New `render_wait_assign` + `leading_axis_slice` methods.
   - When the pair's tile names a strict sub-range of the data's leading axis: emit `{ let _tmp = slot.wait(); name[lo*stride..hi*stride].copy_from_slice(&_tmp[lo*stride..hi*stride]); }`.
   - Else (empty tile, scalar data, or full source-range coverage): the pre-TASK-0117 `name = slot.wait();` whole-array assign.
   - `stride = product of inner dims` from `sidecar.data_type(data).dims[1..]`.
   - Out-of-bounds tile bounds raise `EmitError::ContractGap` (fail loud).

5. Sync-injection co-fix: `nucleus/compiler/src/passes/sync_inject.rs:91..122, 167..189`
   - New `partitioned_iter_vars` set passed through `inject_in_node` / `inject_in_sequence`.
   - On a Repeat whose `iter_var` is partitioned, `wrap_repeat_body` is BYPASSED: no entry/exit Sync at body[0] / body[end].
   - The cross-worker data those Syncs were guarding is now delivered by the TASK-0117 fan-out Push/Wait pairs around the loop; the loop-boundary Syncs (Sequence rule between Repeat and its prior/next siblings) survive unchanged.
   - Non-partitioned Repeats keep their per-iteration body Syncs verbatim — pinned by `non_partitioned_repeat_keeps_body_entry_exit_syncs`.

### Why the sync_inject co-fix had to land in this task

The TASK-0117 brief framed the problem as "the FINAL pipeline piece to make example 13 batch_parallel × pthreads-sync cargo-build" with TASK-0212 supplying per-worker loop bounds. With only the fan-out, the generated binary cargo-built but DEADLOCKED at runtime: host iterates 0..16 ringing bar_1 each iteration, while each compute worker iterates its 4-element slice ringing bar_1 4 times each. The participant count (5) doesn't divide the total arrivals (16 + 4×4 = 32) so the Barrier::wait() cycle stalls.

The per-iteration body-entry Sync is structurally meaningful only under symmetric iteration (02-split-add's host AND w0 both iterate 0..256). Under partition=workers it's structurally meaningless and operationally broken. Suppressing it on partitioned Repeats is the minimal fix.

I considered honest-stopping after the cargo-build symptom cleared, but the deadlock was a direct consequence of the fan-out + partition combination, not a pre-existing orthogonal gap. Filing a new task to fix sync_inject would have left the cell still red. The fix is 10 lines and matches the TASK-0212 architectural template (consume the partition directive at one site).

### AC status (against the implicit ACs from the brief + TASK-0211 expected-behavior block)

- AC-implicit-1 (transfer fan-out across distributed entities): GREEN. One Push/Wait pair per (src,dst) member of cartesian product; verified by `fanout_one_to_n_emits_n_pairs` (4 pairs for {host}→{w1..w4}), `fanout_n_to_one_emits_n_pairs` (4 pairs for {w1..w4}→{host}), and `fanout_one_to_one_unchanged` (1 pair for 1:1 — no regression).
- AC-implicit-2 (per-pair tile carries the compute worker's IterTile slice): GREEN. Verified by `fanout_per_worker_tile_for_input_direction`, `fanout_per_worker_tile_for_output_direction`, and `transfer_fanout_composes_with_partition_sidecar`.
- AC-implicit-3 (cargo-build for example 13 batch_parallel × pthreads-sync): GREEN. Regenerated /tmp/nuc-13-bp/src/main.rs, cargo build --release exit 0.
- AC-implicit-4 (byte-identical output vs reference.bin): GREEN. Runtime hash d893337208d7b469… matches reference.bin sha256 exactly.
- AC-implicit-5 (matrix promotion [[skip]]→[[required]]): GREEN. `nuc-nucleus/e2e-matrix.toml:429..442` promoted; e2e now 36/29/0/7/0 (was 36/28/0/8/0).
- AC-implicit-6 (no regression on 01..07): GREEN. e2e count, generated-crate byte identity unchanged; both backends unaffected. Cross-backend differential green.
- AC-implicit-7 (determinism preserved): GREEN. determinism-check ×2 byte-identical; det-negative bit (29 perturbed); xbackend-negative bit (14 corrupted, 1 detected).

### Honest limits / not-tested-this-cycle

- **N-to-M fan-out** (both sides multi-worker, e.g. all-to-all): the implementation falls back to the "compute worker = dst" convention. None of the in-tree schedules exercises this shape — partition contracts are 1:N or N:1 across a singleton boundary. A coordinate-mapping policy for genuine N×M would be a follow-up.
- **mp-tcp-bufsync** still uses the data-only slot keying; example 13 batch_parallel × mp-tcp remains skipped on TASK-0175 (host-excluding barrier) AND the mp-tcp port of the (DataId, SeqTag) keying. Untouched this cycle.
- **Non-leading-axis partition**: `leading_axis_slice` assumes the partitioned iter var's range maps to the data's leading dim. Example 13 (n is the leading axis of input/output) fits; a partitioned inner-axis schedule would need a generalised stride calculation. Filed as a documented gap.
- **block= × partition=workers**: still uncovered; not exercised by any schedule.

### Gate measurements (7-step)

1. `nix develop -c just test`                       — 489 passed; 0 failed; 2 ignored (was 479/0/2; +10 new tests).
2. `nix develop -c cargo clippy --workspace --all-targets -- -D warnings` — clean.
3. `nix develop -c just e2e`                        — 36 / 29 / 0 / 7 / 0 (was 36/28/0/8/0; +1 cell promoted [[skip]]→[[required]], byte-identical to reference.bin).
4. `nix develop -c just determinism-check` ×2       — byte-identical both runs.
5. `nix develop -c just determinism-check-negative` — 29 cells perturbed, sanity gate green.
6. `nix develop -c just xbackend-check-negative`    — 14 mp-tcp cells corrupted, 1 detected; gate green.
7. `nix develop -c just ci`                         — exit 0.

### Files changed

- `nucleus/compiler/src/passes/transfer_inject.rs` (fan-out + rewrite_partition_tiles sink + module doc rewrite).
- `nucleus/compiler/src/passes/sync_inject.rs` (partition-aware skip).
- `nucleus/backends/pthreads-sync/src/multi_worker.rs` (slot keying by (DataId, SeqTag), tile-aware Wait codegen, pair_tiles sidecar).
- `nucleus/compiler/tests/transfer_inject.rs` (+7 fan-out tests).
- `nucleus/compiler/tests/partition_workers.rs` (+3 composition tests).
- `nuc-nucleus/e2e-matrix.toml` (13/batch_parallel/pthreads-sync [[skip]]→[[required]]).

### Disposition

Marked Done. The cargo-build + byte-identical-output bar from the brief is met; all 7 gate steps green; new tests pin the fan-out shape, per-worker tile, sync-injection co-fix, and end-to-end composition. Sibling tasks updated separately.
<!-- SECTION:NOTES:END -->
