---
id: TASK-0270
title: M5 Stage 2 — multi-worker walker real circular-buffer codegen (TASK-0265.02)
status: To Do
assignee: []
created_date: '2026-05-24 08:32'
updated_date: '2026-05-24 15:25'
labels:
  - M5
  - codegen
  - reuse
  - stage-2
  - forward-carried-from-TASK-0265
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier 3 of TASK-0265 — forward-carried from cycle 87.

Sibling of TASK-0269 (TASK-0265.01, single-worker). Whereas TASK-0269 targets render_event in pthreads-sync (used by ALL 4 backends for single-host schedules), this task wires the real circular-buffer emit at the SHARED multi-worker walker (backend-common/src/multi_worker_walker.rs::render_worker_events_inner Event::Loop arm).

## Affected backends
Multi-worker walker is consumed by:
- pthreads-sync (multi-worker)
- pthreads-async (multi-worker)
- mp-tcp-bufsync (multi-worker)
- mp-tcp-event (multi-worker)

All four pick up the buffer emit at once because the walker is the single source of truth.

## Scope
Identical rewrite shape to TASK-0269 but at the walker site:
1. At Event::Loop entry (BOTH the strip-mined block_tag branch AND the regular branch), read sidecar.reuse_widths.get(iter_var).
2. For each (DataId, axis, ReuseSlot), declare a Vec<T> circular buffer + initial-fill prologue + per-iteration rotate.
3. Rewrite DataRefs in body Fire args.

## Coordination with TASK-0263 halo Stage 2
Halo Stage 2 (TASK-0263 already landed for transfer_inject) and reuse Stage 2 (this task) BOTH consume the same Event::Loop emit site. They are orthogonal:
- Halo widens per-tile transfer ranges (pre-loop, NOT inside the body).
- Reuse rewrites read patterns INSIDE the loop body.
A loop carrying BOTH a halo entry AND a reuse entry needs both code paths active simultaneously. The walker already handles halo via transfer_inject's sidecar; reuse adds an inner-body rewrite.

## Coordination blockers
05-stencil/distributed × {pthreads-async, mp-tcp-event} cells are currently SKIP due to TASK-0267 (host-Push drop under partitioned consumers + async transfer) and TASK-0268 (sync_inject barrier deadlock on unequal-iter partitioned bodies). Those unblock the COMBINED partition+block+reuse exercise. This task can land WITHOUT them — a synthetic 2-worker reuse fixture serves as the test target. Once 0267/0268 land, 05-stencil/distributed's  becomes the integration test.

## AC
1. multi_worker_walker emits Vec<T> circular buffer + rewrite for any Event::Loop carrying reuse_widths.
2. A new multi-worker e2e fixture (synthetic 2-worker reuse) is bit-identical to its reference.
3. just e2e + just determinism-check stay GREEN on all 4 tier-1 backends.
4. cargo test --workspace stays GREEN; unit + integration tests cover the rewrite.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Reconstructed lost fragment from create-time bash interpolation: the integration target is 05-stencil/distributed loop x with block=64, vectorize=8, reuse; once TASK-0267 and TASK-0268 land.

## Forward-carried from TASK-0275 (cycle 96, halo (B) promotion)

Multi-worker walker reuse codegen lands AFTER TASK-0269 (pthreads-sync first). Two carry items:

1. The reuse driver is STRICT (TASK-0271), not (B) partition-policy-aware like halo (TASK-0275). The reason: reuse marker is universally consumed across every recognised slot; halo's transfer_inject consumer is conditional on partition=. Do not transplant the halo (B) shape here.

2. If you extend reuse_inference's walker to thread additional context (e.g. partition= for multi-worker scope checks), introduce a type alias for the paired-Vec return EARLY — clippy::type_complexity fires on the bare `Vec<(Error, Vec<String>)>` shape. See halo_inference.rs `HaloErrorWithScope` for the pattern.

**Forward-carried from TASK-0273 (cycle 98)**: when real circular-buffer codegen lands here on the multi_worker_walker (covering pthreads-async + mp-tcp-bufsync + mp-tcp-event), the `reuse_widths_pending` marker substring at render.rs:867 will rename (likely `reuse_buf_decl`) or be subsumed by a real `let __reuse_buf_<data>` declaration. The NEW test file `nucleus/backend-common/tests/multi_worker_reuse_marker.rs` (cycle 98) asserts the marker substring + 5 payload fields (iv=x, data=img_in, axis=1, length=3, min_offset=-1) on BOTH presence and absence arms. The test file already embeds a top-level module doc-comment warning the next implementer; update assertion shape in lockstep with the codegen change here — do NOT silently drop.

## Forward-carried from TASK-0269 (cycle 103, commit e21d75e)

TASK-0269 landed real pthreads-sync single-worker circular-buffer codegen. The substrate is in backend-common/src/render.rs ready for the multi-worker landing:

### Already in place (for TASK-0270 to consume)

- `ReuseRewriteGroup` struct + `reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>` field on RenderCtx + RenderCtxPub (with_abs_subst preserves it; inner() copies it across the Pub→private bridge).
- `render_reuse_buf_decls(out, indent, iter_var, var, lo_expr_rs, body, ctx) -> Result<BTreeMap<DataId, Vec<ReuseRewriteGroup>>, EmitError>` — walks body to discover first matching DataRef per (data_id, axis); emits Vec<T> decl + unrolled prologue; returns the reuse_active map. Calls into render_flat_index which works through RenderCtx (private side).
- `render_reuse_per_iter_update(out, indent, groups, iv_expr_rs, ctx)` — per-iter most-distant slot fill.
- `try_rewrite_reuse_arg` inside render_fire_arg — consults ctx.reuse_active automatically for any ArgBinding::Data.

### What TASK-0270 needs to add

In multi_worker_walker.rs::Event::Loop arm (BOTH strip-mine path at line 404 and regular path at line 478):

1. Before writing the for-header, call render_reuse_buf_decls — but the helper takes RenderCtx (private) and the walker has RenderCtxPub. Two options:
   - (a) Add render_reuse_buf_decls_pub + render_reuse_per_iter_update_pub _pub shims that take RenderCtxPub and delegate to ctx.inner() (the existing _pub-shim precedent at the bottom of render.rs).
   - (b) Pull the private RenderCtx out of the walker's RenderCtxPub via a (yet-to-add) accessor.
   Option (a) is the cleaner sibling — matches render_fire_args_pub / render_flat_index_pub / render_const_expr_pub.

2. After populating the reuse_groups, build a child RenderCtxPub via with_abs_subst (the existing builder needs to also accept reuse_active OR a NEW with_reuse_active builder). Currently with_abs_subst sets reuse_active from self.reuse_active.clone(); a child with both new abs_subst AND new reuse_active needs a longer-form builder.

3. lo_expr_rs derivation: the walker's regular arm computes (lo, hi) via per-worker partition_worker_ranges fallback to sidecar.loop_bounds. The 'lo' string passed to render_reuse_buf_decls should match — the partition-projected lo (not the source-range lo) when partitioned. For 05-stencil/distributed × pthreads-async: w0/w1 get range 1..4 + 4..7 etc; the buffer prologue's source-array reads should use the per-worker lo (1, 4, 7, 10 respectively).

4. Determinism: render_reuse_buf_decls walks body in source order, BTreeMap iteration. Multi-worker emits per-worker bodies — each worker's body has only the projected events (this worker's). So each worker's discover_reuse_groups sees its OWN subset. Per-worker buffer + prologue is correct.

### Likely AC for TASK-0270 (forward-suggestion)

1. 05-stencil/distributed × pthreads-async (the shipped multi-worker reuse cell) becomes [[required]] M5 bit-identical to reference.bin. Cell is currently SKIP via TASK-0042 capability mismatch on pthreads-sync; the pthreads-async / mp-tcp-event variants would land first (TASK-0267 + TASK-0268 already closed cycle 101-102).
2. Marker substring `reuse_widths_pending` preserved.
3. New e2e assertions on the multi-worker emit verifying `__reuse_buf_img_in_a1` and `vec![0; 3usize]` and `rem_euclid(3_i64)` appear in per-worker code blocks.

### Honest limitations TASK-0269 carried forward (DO NOT regress)

- Narrow rewrite cut: only DataRefs matching the FIRST discovered outer-axes pattern get rewritten (the per-(data, axis) buffer is per-outer-coord-variation-1). The 6 of 9 reads with different outer axes stay verbatim. **TASK-0282 (multi-outer-coord generalisation) is filed as the follow-up that lifts this restriction.**
- Strip-mine arm uses textual replace `abs.replace(var, '0_i64')` to derive the prologue's lo expression. TASK-0270 should mirror or refactor.

### Cycle 103 e2e gate post-TASK-0269 (TASK-0270 should preserve)

- cargo test --workspace: 805 / 0 / 3
- just clippy: clean
- just e2e: 92 / 79 / 0 / 13 / 0
- just determinism-check: 92 / 79 / 0 / 13

### Tests pinning the contract (TASK-0270 must keep firing)

- nucleus-compiler/tests/e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive — 5 marker grep + 3 buffer-shape asserts + 1 symmetric absence on naive.
- pthreads-sync/tests/reuse_marker.rs (synthetic strip-mine marker pin).
- backend-common/tests/multi_worker_reuse_marker.rs (3 multi-worker marker pins — TASK-0270 should ADD a buffer-shape assert here once codegen lands).
<!-- SECTION:NOTES:END -->
