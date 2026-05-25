---
id: TASK-0264
title: >-
  M5 Stage 3: block-pair metadata recovery for halo-strip synthesis under
  partition=blocks2d
status: Done
assignee:
  - '@mark'
created_date: '2026-05-24 01:40'
updated_date: '2026-05-25 00:29'
labels:
  - M5
  - compiler
  - halo
  - partition
  - stage-3
dependencies:
  - TASK-0260
  - TASK-0259
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Stage 3 of the TASK-0260 halo loop. Stage 1 (TASK-0260, cycle 81) landed halo inference; Stage 2 (TASK-0263) will wire transfer_inject. This task addresses the TASK-0259 architect forward-carry: halo-strip Push/Wait synthesis under partition=blocks2d needs to identify the (row, col) neighbours of each worker.

## Problem
partition_blocks2d (TASK-0259) writes TWO entries into ACFG::partition_worker_ranges (one per iter_var, same WorkerId keyset) but does NOT carry block-pair metadata. A future halo-strip synthesis stage cannot tell from the sidecar alone whether two iter_var->Range maps come from one partition=blocks2d directive (paired axes; w_i owns (y_i, x_i) rectangle) OR from two independent partition=rows directives on unrelated loops.

## Acceptance criteria
1. Either re-derive pairing by walking linked.sched.loops for PartitionKind::Blocks2d directives, OR add ACFG.partition_pairs: BTreeMap<IterVar, IterVar> populated by partition_blocks2d. Pick consciously.
2. Worker -> (row, col) inverse: expose partition_blocks2d::decompose_grid as pub(crate), or add sidecar.grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)>. Pick consciously.
3. Halo-strip Push/Wait synthesis identifies the correct neighbours (N/S/E/W cells in 2D grid) for each worker under partition=blocks2d.
4. New e2e cell 05-stencil/distributed-2d x pthreads-async bit-identical to reference.bin.

## References
- TASK-0259 partition_blocks2d implementation: nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs
- TASK-0259 architect forward-carry notes: backlog task view TASK-0259
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Staged landing. TASK-0264 has 4 ACs (sidecar plumbing AC#1+2; halo-strip Push/Wait synthesis AC#3; new e2e cell AC#4). Cycle 113 lands AC#1+2 only — the data plumbing prep that the AC#3 consumer pass will need. AC#3+4 are filed as a follow-up TASK-0289 with the design picks pre-recorded so the next session can resume cold.

DESIGN PICKS (architect P2 from TASK-0259 cycle 80):
- AC#1: Option B — add ACFG.partition_pairs: BTreeMap<IterVar, IterVar> (outer_iv -> inner_iv). Rationale: keeps consumer-site lookup O(log n) at the partition_blocks2d call site already knows the pairing; re-deriving by walking linked.sched.loops would duplicate the logic upstream-of-codegen at every downstream pass that needs the pairing.
- AC#2: Option B — add ACFG.grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)> (rows, cols). Rationale: same reasoning. The partition_blocks2d call site already computed grid_rows + grid_cols via decompose_grid; preserving them in the sidecar is the natural shape. Exposing decompose_grid as pub(crate) would force any downstream consumer to re-invoke it (and to know the worker count for that outer iv — another piece of information already known at the partition site).

IMPLEMENTATION:
1. nucleus/nucleus-compiler/src/acfg.rs: add two new fields to ACFG. Initialise empty in build_acfg(). Doc-comment cross-references TASK-0264 + the consumer's intent.
2. nucleus/nucleus-compiler/src/sidecar.rs: add the same two fields to NameSidecar with #[cfg_attr(feature = "serde", serde(default))]. Mirror from ACFG in build_sidecar().
3. nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs: in the to_record commit loop, insert (plan.outer_iter_var, plan.inner_iter_var) into partition_pairs + (plan.outer_iter_var, (plan.grid_rows as u32, plan.grid_cols as u32)) into grid_shape_for_outer_iv. Destructure the ACFG with the new fields. The other partition passes (partition_workers, partition_rows) forward both verbatim — they do not populate.
4. Every other pass that destructures ACFG (sync_inject, transfer_inject, block_transform, halo_inference, reuse_inference, partition_workers, partition_rows): add the new fields to the destructure pattern and reconstruct. Verbatim forward.
5. Tests:
   - nucleus/nucleus-compiler/tests/partition_blocks2d.rs: add 2 assertions to positive_4_workers test + positive_6_workers test pinning partition_pairs[outer_iv]=inner_iv + grid_shape_for_outer_iv[outer_iv]=(rows, cols).
   - nucleus/nucleus-compiler/tests/sidecar_partition_blocks2d.rs (new file): serde JSON round-trip golden test pinning the new fields' wire shape. Includes a missing-fields-default test (older payload deserialises as empty).
6. Driver: build_acfg() already initialises empty; build_sidecar() needs to mirror the two new fields. Both are no-ops on every shipped schedule that does NOT carry partition=blocks2d, so the e2e baseline 92/79/0/13/0 is preserved.

GATE:
- cargo test --workspace: ~829+ (+~4 new tests).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 92/79/0/13/0 preserved (no shipped fixture exercises partition=blocks2d in a [[required]] cell).
- just determinism-check: 92/79/0/13.

HONEST SCOPE:
- AC#1+2 only. The sidecar plumbing is data-flow-only — no consumer reads the new fields yet, so the e2e matrix bytes are unchanged.
- AC#3 (halo-strip Push/Wait synthesis) + AC#4 (new e2e cell on a 2D-divisible stencil) deferred to TASK-0289 — a substantive piece of work needing transfer_inject extension + a new schedule fixture + a divisible 2D grid example.
- TASK-0264 stays In Progress until TASK-0289 lands; mark Done only after AC#3+4 also close.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-113 LANDED in commit 365dc99 (substantive) + cycle-113 review-hardening commit (in progress: positive end-to-end sidecar mirror test + tracker close).

Per-AC progress:
- AC#1 (pairing recovery): MET via ACFG.partition_pairs + NameSidecar.partition_pairs (Option B: sidecar, not re-derivation).
- AC#2 (worker -> (row, col) inverse): MET via ACFG.grid_shape_for_outer_iv + NameSidecar.grid_shape_for_outer_iv (Option B: sidecar, not decompose_grid exposure).
- AC#3 (halo-strip Push/Wait synthesis): DEFERRED to TASK-0289 with design picks pre-recorded.
- AC#4 (new bit-identical 2D e2e cell): DEFERRED to TASK-0289.

Status stays In Progress until TASK-0289 lands AC#3 + AC#4.

Cycle-113 review: qa-test-runner GO (833 tests pass, 0 failed; e2e 92/79/0/13/0 across 2 samples; determinism 92/79/0/13; clippy clean; zero NEW fmt drift; no consumer reads new fields so emit bytes byte-identical to cycle 112). mped-architect GO (staged landing is honest not AC-gaming; pairing recovery generalises to N disjoint blocks2d directives; worker -> (row, col) inversion soundness verified — row-major from body_workers.iter().enumerate() at write site matches consumer formula in TASK-0289).

Architect P2.1 hardening applied in-thread (commit pending): new positive end-to-end test positive_4_workers_sidecar_mirrors_pair_and_grid_shape in tests/partition_blocks2d.rs — drives apply_partition_blocks2d -> build_sidecar pipeline and asserts NameSidecar mirrors ACFG exactly. Closes the test gap the architect flagged (the cycle-113 first landing had writer-side asserts but no end-to-end wire-shape assertion).

Architect P2.2 (field-name future-proofing) deferred — partition_pairs + grid_shape_for_outer_iv are 2D-specific names today; if blocks3d ever lands, rename under E0063 enforcement. LOW priority, speculative.

Forward-carried lessons (read these before resuming AC#3/AC#4):
- The sidecar writer is single-source (only partition_blocks2d.rs writes partition_pairs + grid_shape_for_outer_iv). The 7 sibling passes destructure-and-forward verbatim. Extending the writer is contained.
- The mirror is via build_sidecar's single .clone() (sidecar.rs after the existing partition_worker_ranges / halo_widths / reuse_widths mirrors).
- AC#3 consumer formula: i = body_workers.iter().position(|w| *w == worker).unwrap(); (row, col) = (i / cols, i % cols) where (rows, cols) = sidecar.grid_shape_for_outer_iv.get(outer_iv).unwrap(). body_workers iteration is BTreeSet numeric order — that matches the partition_blocks2d row-major assignment at partition_blocks2d.rs:409-411.
- Memory entry project-cross-backend-differential: NOT updated this cycle (no e2e movement; plumbing only). Update when AC#3+4 land via TASK-0289.

Files modified (cycle 113): 19 files, +515/-4 lines (substantive) + 1 file +47 (hardening).
- src/acfg.rs + src/sidecar.rs (field additions)
- src/passes/partition_blocks2d.rs (writer)
- src/passes/{sync_inject, transfer_inject, block_transform, halo_inference, reuse_inference, partition_workers, partition_rows}.rs (verbatim forwarders)
- tests/{partition_blocks2d, partition_workers, partition_rows, transfer_inject, transfer_inject_hoist, sync_inject, acfg_to_petri, petri_to_events}.rs (hand-built ACFG instances extended)
- tests/sidecar_partition_blocks2d.rs (new file, 3 tests)

## Cycle 115 close — AC#3 + AC#4 now MET (cascade close from TASK-0289 → TASK-0290 → TASK-0294)

Full M5 Stage-3 halo-strip chain landed:
- Cycle 113 (commit 365dc99 + fec78e9): AC#1 (partition_pairs sidecar) + AC#2 (grid_shape_for_outer_iv sidecar) — sidecar plumbing.
- Cycle 114a (TASK-0289, commit f8d58ea + 11c23e7): AC#3 (halo-strip Push/Wait synthesis fires).
- Cycle 114b (TASK-0290, commits b334627/d2a4ec6/436f146/7a564e7): placement fix + first e2e cell wired in (initially [[skip]] pending wait_slice 2D path).
- Cycle 115 (TASK-0294): the wait_slice 2D row-loop slice-paste in backend-common closes the bit-identical assertion; AC#4 (new e2e cell 05-stencil/distributed-2d × pthreads-async bit-identical to reference.bin) MET via promotion to [[required]] in nuc-nucleus/e2e-matrix.toml.

e2e baseline: 96/80/0/16/0 (was 92/79/0/13/0 pre-cycle-114b).

Task closed cleanly. The Stage-3 keystone is shut.
<!-- SECTION:NOTES:END -->
