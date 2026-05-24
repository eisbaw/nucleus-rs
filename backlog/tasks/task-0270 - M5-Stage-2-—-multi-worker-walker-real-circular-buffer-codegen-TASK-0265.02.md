---
id: TASK-0270
title: M5 Stage 2 — multi-worker walker real circular-buffer codegen (TASK-0265.02)
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 08:32'
updated_date: '2026-05-24 16:40'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Multi-worker walker real circular-buffer codegen (TASK-0265 Stage 2 Tier 3, sibling of TASK-0269).

Builder choice: (a) - add RenderCtxPub::with_abs_subst_and_reuse_active for the strip-mine arm (single-call site builder; closest sibling pattern to existing with_abs_subst). The regular arm uses a fresh builder with_reuse_active.

Marker decision: keep 'reuse_widths_pending' marker substring INTACT (load-bearing across 4 grep tests; the multi-worker sites now do emit real codegen below it just like pthreads-sync after TASK-0269). Tweak the parenthetical from 'on pthreads-sync — TASK-0269' to '— TASK-0269 (single-worker) + TASK-0270 (multi-worker)' so the marker text is no longer a doc-lie on multi-worker call sites. The new __reuse_buf_<data>_a<axis> + rem_euclid(L_i64) substrings are the second-layer codegen canary added to multi_worker_reuse_marker.rs.

Render.rs changes:
1. Add render_reuse_buf_decls_pub(out, indent, iter_var, iter_var_name, lo_expr_rs, body, ctx: &RenderCtxPub) -> Result<BTreeMap<DataId, Vec<ReuseRewriteGroup>>, EmitError> (delegates to private fn via ctx.inner()).
2. Add render_reuse_per_iter_update_pub(out, indent, groups, iv_expr_rs, ctx: &RenderCtxPub) -> Result<(), EmitError>.
3. Add RenderCtxPub::with_abs_subst_and_reuse_active(abs_subst, reuse_active) -> RenderCtxPub<'a>.
4. Add RenderCtxPub::with_reuse_active(reuse_active) -> RenderCtxPub<'a>.
5. Update render_reuse_marker_comment parenthetical text to be backend-agnostic.

multi_worker_walker.rs changes:
1. Extend render_block_tag_loop_header to return (RenderCtxPub<'a>, String) where the String is the structural strip_lo_expr built from same (lo_src, tile_name, n, is_partial, num_full) components - NO textual abs.replace().
2. In Event::Loop strip-mine arm (lines 395-424): after calling render_block_tag_loop_header, emit render_reuse_buf_decls_pub at the OUTER indent (before the for-header — wait, the helper already wrote the for-header). RESTRUCTURE: split header helper so the buffer decls land BEFORE the for line. Simpler restructure: helper now returns (child, strip_lo_expr) WITHOUT emitting the for-header; the caller emits buf_decls_pub at indent THEN the for-header at indent THEN the marker + per_iter_update at indent+1, then recurse, then closing.
3. In Event::Loop regular arm (lines 449-485): symmetric - call render_reuse_buf_decls_pub with per-worker lo BEFORE writing for-header.

Tests:
- backend-common/tests/multi_worker_reuse_marker.rs: extend test 3 (strip-mine arm) with a Fire body carrying a reuse-axis DataRef (mirroring pthreads-sync/tests/reuse_marker.rs::pthreads_sync_strip_mine_arm_emits_real_buffer_codegen). Assert __reuse_buf_img_in_a1, rem_euclid(3_i64), and 'x__tile' name-overlap regression. Add NEW test for regular arm codegen with same Fire body shape.

Verification gates:
- nix develop --command bash -c 'cd nucleus && cargo test --workspace' - expect 808+/0/3
- nix develop --command just clippy - clean
- nix develop --command just e2e - target: 92/79/0/13/0 preserved
- nix develop --command bash -c 'cd nucleus && cargo run --release --bin nucleus-e2e -- --check-determinism' - 92/79/0/13

Bonus possibility: 05-stencil/distributed × pthreads-async + mp-tcp-event currently PROMOTED bit-identical via marker-only. They MUST remain bit-identical after real codegen lands (reuse is a perf rewrite). If they regress that is a hard fail.
<!-- SECTION:PLAN:END -->

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

Forward-carried from TASK-0269 cycle-103 architect review (P2.1): when multi-worker walker landing replaces the marker-only emit with real circular-buffer codegen on the multi-worker path, the marker substring's parenthetical 'circular-buffer codegen below on pthreads-sync — TASK-0269' becomes a doc-lie on the multi-worker sites (it claims real codegen follows, but only the single-worker sites do). UPDATE THE MARKER TEXT IN LOCKSTEP with the multi-worker codegen landing — either branch the marker per-call-site (different text for the multi-worker arms), or rename the marker entirely to a backend-agnostic substring (e.g. 'reuse_buf_decl') and update all 4 grep tests + the test docstrings in lockstep. The grep tests carrying the substring are: nucleus/nucleus-compiler/tests/e2e_example_05.rs (5 asserts), nucleus/backends/pthreads-sync/tests/reuse_marker.rs (2 asserts + 1 new strip-mine codegen assertion as of cycle 103 hardening commit cd2310c), nucleus/backend-common/tests/multi_worker_reuse_marker.rs. This is now a HARD AC of TASK-0270, not optional.

## Cycle 104 landing (TASK-0270 Done — commit bab57cc)

### What landed (commit bab57cc)

Real circular-buffer codegen on the shared multi-worker walker (`backend-common::multi_worker_walker::render_worker_events_inner` Event::Loop arm). Both the strip-mine and regular arms now emit the per-(data, axis) `Vec<T>` decl + initial-fill prologue + per-iter rotate + DataRef rewrite. Consumers: pthreads-async + mp-tcp-event + pthreads-sync (multi-worker).

### Substrate added in backend-common::render

1. `RenderCtxPub::with_reuse_active(reuse_active)` — regular-arm builder preserving parent abs_subst.
2. `RenderCtxPub::with_abs_subst_and_reuse_active(abs_subst, reuse_active)` — strip-mine arm joint builder (one pass, no chain-clone overhead).
3. `render_reuse_buf_decls_pub(out, indent, iter_var, iter_var_name, lo_expr_rs, body, ctx)` — `_pub` shim layer matching the existing `render_fire_args_pub` / `render_const_expr_pub` precedent (delegates via `ctx.inner()`).
4. `render_reuse_per_iter_update_pub(out, indent, groups, iv_expr_rs, ctx)` — same shape.

### Walker changes

1. NEW pure helper `compute_block_tag_abs_exprs(iter_var, tag, enclosing, ctx) -> Result<(String, String), EmitError>` returning `(abs, strip_lo_expr)`. Both expressions built STRUCTURALLY from the same components — NO textual `abs.replace(var, "0_i64")` step. Mirrors the cycle-103 P1.1 architect fix on pthreads-sync.
2. `render_block_tag_loop_header` API unchanged — it now delegates to `compute_block_tag_abs_exprs` internally then writes the header (existing tests + mp-tcp-bufsync caller untouched).
3. Strip-mine arm: uses `compute_block_tag_abs_exprs`, emits buf decls + prologue at OUTER pad with `strip_lo_expr` as the prologue's lo argument, writes for-header, builds child via `with_abs_subst_and_reuse_active`, emits marker + per-iter update, recurses, closes.
4. Regular arm: emits buf decls + prologue at OUTER pad with the per-worker partition-projected lo (correct source-array index for the prologue fill when `partition_worker_ranges` recorded a slice), writes for-header, emits marker, emits per-iter update with bare var, builds child via `with_reuse_active`, recurses through both check_frame branches.

### Marker text update

`reuse_widths_pending` substring PRESERVED (load-bearing across 4 grep tests: e2e_example_05 5+ asserts, pthreads-sync/tests/reuse_marker.rs 2 asserts, backend-common/tests/multi_worker_reuse_marker.rs 3+ asserts). Parenthetical updated from "on pthreads-sync — TASK-0269" to "TASK-0269 single-worker + TASK-0270 multi-worker" so the marker is no longer a doc-lie on the multi-worker emission sites.

### Tests added

- `multi_worker_walker_regular_arm_emits_real_buffer_codegen` (multi_worker_reuse_marker.rs) — pins buffer decl + `vec![0; 3usize]` + `rem_euclid(3_i64)` on the non-strip-mine arm with a Fire body carrying a reuse-axis DataRef.
- `multi_worker_walker_strip_mine_arm_emits_real_buffer_codegen` — same for strip-mine arm + load-bearing P1.1 name-overlap regression (tile="x__tile" must appear intact; "0_i64__tile" must NOT appear).

### Per-AC status

- **AC#1** (multi_worker_walker emits Vec<T> circular buffer + rewrite for any Event::Loop carrying reuse_widths): **DONE**. Both regular + strip-mine arms emit. Verified by spot-check on 05-stencil/distributed × {pthreads-async, mp-tcp-event}.
- **AC#2** (multi-worker e2e fixture bit-identical to reference): **DONE**. 05-stencil/distributed × pthreads-async + mp-tcp-event remain PROMOTED [[required]] M5 bit-identical to reference.bin (reuse is a perf rewrite, not semantic).
- **AC#3** (just e2e + just determinism-check stay GREEN on all 4 tier-1 backends): **DONE**. Both at 92/79/0/13.
- **AC#4** (cargo test --workspace stays GREEN; unit + integration tests cover the rewrite): **DONE**. 808/0/3 (up 2 from 806 — two new codegen tests).

### Gate numbers (cycle 104 post-TASK-0270)

- cargo test --workspace: 808 / 0 / 3
- just clippy: clean
- just e2e: 92 / 79 / 0 / 13 / 0
- just determinism-check: 92 / 79 / 0 / 13

### Spot-check confirmation

05-stencil/distributed × pthreads-async (src/main.rs):
```
let mut __reuse_buf_img_in_a1: Vec<i32> = vec![0; 3usize];
__reuse_buf_img_in_a1[(((((1_i64 + (0_i64 * 64_i64) + 0_i64)) + (-1_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)] = img_in[...];
__reuse_buf_img_in_a1[(((((1_i64 + (0_i64 * 64_i64) + 0_i64)) + (0_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)] = img_in[...];
for x in (0_i64)..(64_i64) {
    // reuse_widths_pending: iv=x data=img_in axis=1 length=3 min_offset=-1 (Stage 2 active; circular-buffer codegen below — TASK-0269 single-worker + TASK-0270 multi-worker)
    __reuse_buf_img_in_a1[((((1_i64 + (0_i64 * 64_i64) + x)) + (1_i64) - (-1_i64)).rem_euclid(3_i64)) as usize] = img_in[...];
    img_out[...] = kernels::blur3(__reuse_buf_img_in_a1[...], __reuse_buf_img_in_a1[...], __reuse_buf_img_in_a1[...], img_in[...], img_in[...], img_in[...], img_in[...], img_in[...], img_in[...]);
```

Note the 3 axis-1 reads are rewritten to `__reuse_buf_img_in_a1[...]`; the 6 outer-axis reads (y-1, y+1 rows) stay verbatim. This is the TASK-0269 narrow-rewrite-cut applied here (also forward-carries to TASK-0282).

05-stencil/distributed × mp-tcp-event: each worker bin (w0..w3) carries 5 `__reuse_buf_img_in_a1` occurrences; host.rs (no blur3) carries 0.

### Honest limitations / gotchas forward-carried

1. **mp-tcp-bufsync's strip-mine arm**: still routes through `render_block_tag_loop_header` and does NOT emit reuse codegen on multi-worker. NOT a regression (it never did pre-TASK-0270); 05-stencil/distributed × mp-tcp-bufsync is SKIPPED on capability mismatch (async + buffer + notify=event). A follow-up task can wire reuse on mp-tcp-bufsync's per-event walker when a multi-worker reuse cell lands on a sync-capability schedule.
2. **Narrow-rewrite-cut from TASK-0269 applies here too**: only DataRefs matching the FIRST discovered outer-axes pattern per (data, axis) get rewritten. The 6 of 9 reads with different outer axes stay verbatim. TASK-0282 (multi-outer-coord generalisation) remains the filed follow-up.
3. **Order-sensitive emit**: buffer decl + prologue MUST land BEFORE the for-header (the buffer must persist across iterations). This is why `render_block_tag_loop_header` was split — the original helper wrote the header inline, so the walker had to grow its own header emit logic when buf decls had to land first. The pure `compute_block_tag_abs_exprs` helper keeps the structural-expression construction in one place across walker + mp-tcp-bufsync (via `render_block_tag_loop_header` continuing to use it).
4. **Substring overlap regression (P1.1)**: the multi-worker walker is now also immune to the `abs.replace(var, "0_i64")` substring-overlap defect. Both `abs` and `strip_lo_expr` are built structurally from `(lo_src, tile_name | num_full, n, var)` — no textual replace anywhere. Pinned by the new `multi_worker_walker_strip_mine_arm_emits_real_buffer_codegen` test.

### Forward-carry to TASK-0282 (multi-outer-coord generalisation)

The narrow-rewrite-cut applies on both the single-worker (TASK-0269) and multi-worker (TASK-0270) emission paths. TASK-0282's lift will need to touch BOTH `render_reuse_buf_decls` (in render.rs — used by both paths) AND any helper that derives groups per-coord-variation. The substrate (`ReuseRewriteGroup` + `reuse_active` BTreeMap + `try_rewrite_reuse_arg`) already supports multiple groups per DataId; the missing piece is the discovery walker (`discover_reuse_groups` + `walk_event_for_reuse`) collecting all coord-variations rather than just the first.

### Symmetric closure with TASK-0269

Both TASK-0269 (pthreads-sync single-worker, cycle 103) and TASK-0270 (multi-worker walker, cycle 104) are now Done. The reuse codegen path is closed on all 4 tier-1 backends:
- pthreads-sync single-worker: TASK-0269.
- pthreads-sync multi-worker: TASK-0270 (this task, via the shared walker).
- pthreads-async multi-worker: TASK-0270.
- mp-tcp-event multi-worker: TASK-0270.

mp-tcp-bufsync strip-mine arm remains marker-only on multi-worker reuse, but no shipped cell exercises it (capability mismatch on 05-stencil/distributed).

## CYCLE-104 REVIEW-HARDENING (orchestrator, 2026-05-24)

Parallel read-only review gate on commits bab57cc + 1bc841d:

**QA: GO** — 808/0/3 tests (no flake across 2 runs), clippy clean, 92/79/0/13/0 e2e (2 runs), determinism + both falsifiers BITE. Codegen-shape canaries verified across 4 cells (counts of `__reuse_buf_img_in_a1` match the expected 5 per worker × 4 workers for 05-stencil/distributed pthreads-async + mp-tcp-event; 5 per single-worker for 05-stencil/reuse pthreads-async + mp-tcp-event). No HashMap leak. cycle-103 P1.1 textual-replace defect verifiably absent.

**Architect: GO conditional on P1.1 fix**:

- **P1.1** — doc-lie in `render_reuse_marker_comment` body comment at render.rs:1052-1054. Claimed the marker now precedes circular-buffer codegen on "the shared multi-worker walker (used by pthreads-async + mp-tcp-bufsync + mp-tcp-event)" but mp-tcp-bufsync has its OWN per-event Plan walker at backends/mp-tcp-bufsync/src/lib.rs::Plan::render_events (line 772). It delegates to backend-common's render_block_tag_loop_header for the strip-mine header only and does NOT call render_worker_events_inner — so reuse codegen is absent on mp-tcp-bufsync. **Fixed in commit 601e81f** (doc rewritten with accurate consumer list + silent-sibling caveat).

- **P2.1** — same defect as architect P1.1 viewed from the silent-sibling angle, AND QA P3.1: mp-tcp-bufsync's Plan walker has no reuse codegen on EITHER arm. **Filed as TASK-0284** (LOW; dormant — no shipped cell exercises mp-tcp-bufsync's missing reuse path).

- **P2.2** — stale section header at render.rs:1069 said "TASK-0269 Stage 2 Tier 2 (pthreads-sync single-worker path)" even though cycle 104 added the _pub shims under it. **Fixed in commit 601e81f**.

- **P2.3** — redundant names.iter_var.get lookup in render_block_tag_loop_header (now also looked up by compute_block_tag_abs_exprs). Cosmetic cruft, no correctness impact. **Deferred**.

- **P2.4** — builder explosion smell (two new with_* methods at once; 2^N if a third orthogonal field lands). **Deferred** as future tech-debt note.

- **P3** — cosmetic in-source TASK-id references + BTreeMap.clone() perf at body recursion. **Deferred**.

- **QA P2.1** — marker-discriminator stylistic follow-up (architect P2.1 from cycle 103 was 'rename or branch the marker'; cycle 104 chose 'extend the parenthetical' instead). QA noted the new arm-specific tests provide a stronger discriminator than the marker text would. **Accepted as the test-based discriminator approach**.

## Gate post-hardening (this cycle, verified by both reviewers + orchestrator)

- cargo test --workspace: **808 / 0 / 3** (+2 vs cycle 103 baseline 806).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: **92 / 79 / 0 / 13 / 0** required-fail (preserved; reuse is perf rewrite not semantic).
- just determinism-check: **92 / 79 / 0 / 13** (GREEN).
- just determinism-check-negative: bites correctly.
- just xbackend-check-negative: bites correctly.

## Stale-binary trap (cycle-104-specific gotcha for forward-carry)

The release binaries (nucleus driver + nucleus-e2e) became stale during the session — the implementer's first run showed the OLD pre-cycle-104 emit in target/e2e-matrix/. The cargo workspace dependency graph did not invalidate nucleus driver against backend-common's source change, OR cargo's mtime-based invalidation missed the inline edits. After a forced `cargo build --release --workspace` (32s) the nucleus driver was fresh and the emit correctly showed the new reuse codegen. **Forward-carry to TASK-0284 + future M5/M6 implementer cycles**: always force-rebuild the release binaries before relying on target/e2e-matrix/ content. `stat -c \"%Y\" nucleus/target/release/nucleus` vs the source mtime is the quick check.

## Review-gate decision

**GO** for Done. All 4 ACs GREEN per the implementer's report + verified by both reviewers:

- AC#1 (multi_worker_walker emits Vec<T> + rewrite for any Event::Loop carrying reuse_widths): MET on both arms; the new pure helper compute_block_tag_abs_exprs handles the strip-mine path without textual replace.
- AC#2 (multi-worker e2e fixture bit-identical to reference): MET — 05-stencil/distributed × pthreads-async + mp-tcp-event remain [[required]] bit-identical with REAL reuse codegen now.
- AC#3 (just e2e + determinism stay GREEN on all 4 tier-1 backends): MET.
- AC#4 (cargo test --workspace GREEN; unit + integration tests cover the rewrite): MET — 2 new tests pin the codegen shape on both walker arms with the x__tile name-overlap regression assertion.

The mp-tcp-bufsync silent-sibling (architect P2.1 / QA P3.1) is honest scope: this task wired the SHARED walker, mp-tcp-bufsync's private walker is a separate site filed as TASK-0284.

## Final commits for TASK-0270

- bab57cc — multi-worker walker reuse codegen + new builders + new tests.
- 1bc841d — tracker: cycle-104 implementation summary.
- 601e81f — cycle-104 review-hardening 1/2 (architect P1.1 + P2.2): doc-honesty fixes + TASK-0284 filed.
- (this commit) — cycle-104 review-hardening 2/2: tracker close-out.
<!-- SECTION:NOTES:END -->
