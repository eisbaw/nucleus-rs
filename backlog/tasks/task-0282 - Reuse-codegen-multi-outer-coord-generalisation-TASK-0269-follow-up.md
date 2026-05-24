---
id: TASK-0282
title: 'Reuse codegen: multi-outer-coord generalisation (TASK-0269 follow-up)'
status: Done
assignee:
  - '@mark'
created_date: '2026-05-24 15:24'
updated_date: '2026-05-24 18:21'
labels:
  - M5
  - codegen
  - reuse
  - forward-carried-from-TASK-0269
dependencies:
  - TASK-0269
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0269 (cycle 103, commit e21d75e) landed real circular-buffer codegen for pthreads-sync's single-worker path. The first-cut takes the FIRST matching DataRef per (data_id, axis) as the CANONICAL outer-axes pattern. Reads with the same outer axes get rewritten to the buffer; reads with DIFFERENT outer axes stay verbatim.

For 05-stencil/reuse this means: `img_in[y-1][x-1..=x+1]` (the first 3 args to blur3) get the buffer treatment. The 6 reads at `img_in[y][x-1..=x+1]` and `img_in[y+1][x-1..=x+1]` stay verbatim — even though they ALSO have an x-axis reuse pattern that would benefit.

## Scope

Generalise the rewrite: emit ONE buffer per UNIQUE (data_id, axis, outer-coord-tuple) group. For 05-stencil/reuse: 3 buffers (one per row y-1, y, y+1). All 9 reads get the buffer treatment.

## Acceptance Criteria

1. `render_reuse_buf_decls` discovers EVERY unique outer-axes pattern per (data_id, axis) — not just the first matching one. Returns a Vec<ReuseRewriteGroup> per (data_id) where each group is one buffer.
2. Buffer ident scheme: `__reuse_buf_<data>_a<axis>_g<group_idx>` (group_idx in source-order to keep determinism).
3. Every DataRef matching ANY group gets rewritten; only DataRefs with axis index that fails `try_reuse_axis_offset` OR outer axes not matching ANY group stay verbatim.
4. 05-stencil/reuse e2e test: 9 of 9 `img_in[...]` reads in the for-x body are buffer reads (verify by a grep that counts `img_in[` lines inside the for-x block and asserts \<= 3 — only the per-iter updates + 0 verbatim reads on the optimal path).
5. Bit-identical to reference.bin preserved.

## Honest scope

- This is a PERF rewrite; correctness already met by TASK-0269.
- The 3 prologues + 3 per-iter updates triple the loop-entry overhead; the per-call read count drops from 9 to 0. Net win for any non-trivial stencil; cost analysis is M6+ scope.
- The buffer name disambiguation (group_idx) is a determinism concern — must walk in BTreeMap-ordered source position.

## Dependencies

- TASK-0269 (closed, cycle 103): the single-(data, axis) shape that this generalises.
- TASK-0270 (open): when multi-worker walker lands real codegen, this generalisation should land on BOTH paths or document the asymmetry.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Tier-1 minimum-deep landing for TASK-0282 (multi-outer-coord reuse codegen generalisation).

Implementation plan:
1. nucleus/backend-common/src/render.rs:
   - Add group_idx: u64 field to ReuseRewriteGroup.
   - Change walk_arg_for_reuse signature from &mut BTreeMap<u64, ReuseRewriteGroup> to &mut Vec<ReuseRewriteGroup> (collect ALL unique (axis, outer_axes) tuples per data, not just first per axis).
   - For each new (axis, outer_axes) discovered, dedupe via .iter().any(|g| g.axis == axis && g.outer_axes == outer_axes); assign group_idx = .iter().filter(|g| g.axis == axis).count().
   - New buf_ident format: __reuse_buf_<data>_a<axis>_g<group_idx>.
   - Remove the early-out 'if found.len() == per_axis.len() { break; }' (must walk full body to discover ALL groups).

2. Update doc-comments in render.rs + multi_worker_walker.rs referencing the old __reuse_buf_<data>_a<axis> naming (now __reuse_buf_<data>_a<axis>_g<group_idx>).

3. Update tests using the old naming (always _g0 for previously-single-group cases):
   - nucleus/nucleus-compiler/tests/e2e_example_05.rs (1)
   - nucleus/backends/pthreads-sync/tests/reuse_marker.rs (4)
   - nucleus/backends/mp-tcp-bufsync/tests/reuse_codegen_emit.rs (3)
   - nucleus/backend-common/tests/multi_worker_reuse_marker.rs (2)

4. New AC#4 assertion in e2e_example_05.rs: count occurrences of img_in[ inside the for-x body, assert <= 3 (only the per-iter updates remain after generalisation).

Gate: just e2e baseline 92/79/0/13/0 must hold (perf rewrite — bit-identical preserved). cargo test --workspace + clippy + determinism-check stay GREEN.

Honest scope:
- Tier 1 only (this cycle). Multi-worker walker path uses the same shared helpers — coverage carries forward.
- Buffer count grows from 1 to N (where N = unique outer-coord patterns); 05-stencil/reuse: 1 -> 3.
- TASK-0270 multi-worker walker codegen consumes the same ReuseRewriteGroup shape — no asymmetry expected.
- Limitation: no perf measurement (PRD §11 M6+ scope) — only correctness + bit-identity preserved.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 110 LANDED in commit 6984c64.

Per-AC final:
- AC#1 (Vec<ReuseRewriteGroup>, every unique outer-axes group): MET. New `group_idx: u64` field, dedupe via IrExpr PartialEq, source-discovery-order index assignment.
- AC#2 (group_idx in source-order naming `__reuse_buf_<data>_a<axis>_g<idx>`): MET. Determinism via BTreeMap iteration on per_data + per_axis plus Vec-append-order on found.
- AC#3 (every matching DataRef rewritten): MET via try_rewrite_reuse_arg outer-axes match loop iterating the longer Vec.
- AC#4 (9 of 9 img_in reads rewritten in 05-stencil/reuse for-x body, <= 3 verbatim): MET. New presence asserts on _g1/_g2 + brace-extraction count check in e2e_example_05.rs.
- AC#5 (bit-identical to reference.bin preserved): MET. e2e 92/79/0/13/0 + determinism 92/79/0/13.

Files touched (8 modified, +263/-94 lines):
- nucleus/backend-common/src/render.rs (core: discover_reuse_groups, walk_arg_for_reuse, ReuseRewriteGroup struct; doc-comment updates)
- nucleus/backend-common/src/multi_worker_walker.rs (doc-comment updates)
- nucleus/backends/pthreads-sync/src/lib.rs (doc-comment update)
- nucleus/backend-common/tests/multi_worker_reuse_marker.rs (rename _a1 -> _a1_g0)
- nucleus/backends/pthreads-sync/tests/reuse_marker.rs (rename _a0 -> _a0_g0, _a1 -> _a1_g0)
- nucleus/backends/mp-tcp-bufsync/tests/reuse_codegen_emit.rs (rename _x_a0 -> _x_a0_g0)
- nucleus/nucleus-compiler/tests/e2e_example_05.rs (presence asserts on _g0/_g1/_g2 + AC#4 verbatim count <= 3)

CYCLE-110 REVIEW-HARDENING (orchestrator, 2026-05-24):

Parallel read-only review-gate ran on commit 6984c64. **qa-test-runner GO** (818 passed, 0 failed; e2e 92/79/0/13/0 across 2 samples; determinism 92/79/0/13; clippy -D warnings clean; emit inspection confirms 3 buffer decls + 6 prologue fills + 3 per-iter updates + 9 buffer reads in blur3 call on single-worker, 12 group decls = 4 workers × 3 groups on multi-worker; no new fmt drift on touched files). **mped-architect GO with 2 P1 doc-lie fixes**:

- **P1.1 (doc-lie)**: render.rs:465-477 `render_fire_arg` comment still said "restrictive cut keeps the first-cut landing narrow (only 1 of 3 outer-coord variations) ... follow-up". TASK-0282 IS the follow-up; this commit overhauls the function. Rewrote to describe TASK-0282 full-coverage rewrite (3 buffers covering y-1, y, y+1).
- **P1.2 (stale doc-lie)**: render.rs:1074-1083 still claimed reuse codegen "silently absent on mp-tcp-bufsync". TASK-0284 (cycle 107, commit 215bb7d) fixed that. Rewrote the block to describe the post-cycle-107 reality: all four tier-1 backends now consume the reuse codegen.

Both P1s fixed in-thread. Also updated the trailing parenthetical text in the emitted marker comment from "TASK-0269 single-worker + TASK-0270 multi-worker" to "multi-outer-coord rewrite landed cycles 103/104/107/110 on all 4 tier-1 backends" — change is comment-only in the EMITTED source, so determinism + bit-identicality preserved (re-verified post-hardening: e2e 92/79/0/13/0, determinism 92/79/0/13).

**P2 findings filed as follow-up tasks**:
- **TASK-0286** (P2.1): Outer-axes structural-equality dedupe risk. `y - 1 + 0` vs `y - 1` would over-emit redundant buffers (conservative-safe but the AC#4 `<= 3` grep would NOT catch a silent over-emission). Cheap defence: `canonicalise_outer_axes` helper at insertion site. Filed LOW priority — no shipped fixture triggers today.
- **TASK-0287** (P2.2): AC#4 body-extraction is fragile to indent changes (`find(\\n        }\\)` literal 8-space close). A future tile-wrap or partition-wrap would silently shrink the extracted body. Brace-balance scan from `for_x_open_brace + 1` to depth-0 close is the robust replacement. Filed LOW priority — pre-emptive hardening.

**P3 observations** (architect): determinism sound (BTreeMap walk-order + Vec-append-order, no HashMap escape); multi-worker parity confirmed (both  strip-mine arm and  regular arm call render_reuse_buf_decls_pub + thread via with_reuse_active; pthreads-sync  calls the private equivalents identically; mp-tcp-bufsync Plan::render_events at  consumes the _pub shim per TASK-0284); test-count delta +12 over TASK-0269 baseline 806 → 818 is legitimate (multi_worker_reuse_marker, reuse_buf_math, reuse_codegen_emit, reuse_marker.rs, e2e_example_05.rs accumulated test churn cycles 103-110); honest-scope wording is honest (perf disclaimed as M6+ scope, structural claims verifiable).

**Forward-carry memory updated** (architect P5): `project-cross-backend-differential.md` appended a cycle-110 section describing the multi-outer-coord generalisation + the post-cycle-110 buffer-ident substring rename (anyone grepping for the old `_a<axis>` substring after cycle 110 will miss). TASK-0286 + TASK-0287 referenced as open follow-ups.

### Forward-carried lessons for next implementer

1. **Outer-axes canonicalisation precondition**: the dedupe at `walk_arg_for_reuse:1370-1375` assumes upstream passes have canonicalised outer-axes ASTs. If a future affine pass starts emitting `Add(_, IntLit(0))` or similar in outer-axis positions, the dedupe will over-emit. TASK-0286 is the cheap defence; consider lifting earlier if the upstream changes.

2. **Multi-worker amplification of group count**: the multiplicity per  is . 05-stencil/distributed × pthreads-async = 4 × 3 = 12 buffer decls. A 5-point cross stencil would be 4 × 5 = 20; a 7-point box on 8 workers = 56. Probably not a problem at M5 scale but a memory entry candidate (`project-reuse-multi-group-cardinality`) if a future cell pushes >5 groups per data.

3. **Strip-mine + multi-outer-coord combination**: 05-stencil/distributed exercises both today (block=64, vectorize=8, reuse on x + 3 outer-y patterns from blur3). The combinational growth is multiplicative; the strip-mine arm in multi_worker_walker.rs:485 and pthreads-sync/src/lib.rs:653 both call render_reuse_buf_decls (private/pub variants) with the SAME body recursion, so the multi-group emit lands uniformly. Verified by 12 buffer decls in the e2e-scratch emit of `example_05_distributed_pthreads_async_no_inner_bar_check`.

4. **Future M6 partition-aware reuse**: if a schedule needs per-worker buffers (not the current shared-across-workers shape), the natural extension is `(data_id, axis, outer_axes_tuple, partition_id)` — i.e. `group_idx` plus a worker-key. The `ReuseRewriteGroup` shape is the load-bearing surface; `buf_ident` would become `_g<group_idx>_w<worker_id>` or similar. Today the multi-worker walker threads ONE `reuse_active` map across all workers (worker-agnostic). M6 partition-aware extension needs either per-worker `RenderCtx::reuse_active` or a `(worker_id, data_id) -> Vec<group>` reshape.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 110 LANDED multi-outer-coord reuse codegen generalisation in commit 6984c64. Single change to discover_reuse_groups + walk_arg_for_reuse + ReuseRewriteGroup: collect every unique (data_id, axis, outer_axes) instead of only the first per axis. 05-stencil/reuse now emits 3 buffers (one per row y-1, y, y+1) and rewrites all 9 img_in reads in blur3; multi-worker amplifies to 4 workers × 3 groups = 12 buffer decls on 05-stencil/distributed × pthreads-async. Uniform _g<group_idx> naming (single-group cases carry _g0). Architect P1 doc-lies fixed in-thread (cycle-110 review-hardening). P2 follow-ups TASK-0286 + TASK-0287 filed. Memory updated. Gate: 818 tests pass, e2e 92/79/0/13/0 across 2 samples, determinism 92/79/0/13, clippy clean. All 5 ACs MET.
<!-- SECTION:FINAL_SUMMARY:END -->
