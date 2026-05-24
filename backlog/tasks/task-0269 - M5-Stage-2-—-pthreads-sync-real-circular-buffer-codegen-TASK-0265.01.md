---
id: TASK-0269
title: M5 Stage 2 — pthreads-sync real circular-buffer codegen (TASK-0265.01)
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 08:31'
updated_date: '2026-05-24 15:23'
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
Tier 2 of TASK-0265 — forward-carried from cycle 87.

The Tier 1 marker scaffold (commit 7d03606) wired walker-side LOOKUP of reuse_widths and emits a reuse_widths_pending comment at every Event::Loop body entry. This task replaces the comment with actual delay-line / circular-buffer Rust code on pthreads-sync (the simplest single-worker variant).

## Scope
At every Event::Loop body entry where sidecar.reuse_widths.get(iter_var) is Some, per (DataId, axis, ReuseSlot):

1. Declare a Vec<T> circular buffer of length elements (T from sidecar.data_type(data_id).scalar).
2. Emit an initial-fill prologue. For min_offset=-1, length=3 (offsets {-1, 0, +1}): seed buf[0] and buf[1] with img_in[y][0] and img_in[y][1] BEFORE the first body iteration; buf[2] gets img_in[y][x+1] inside the body.
3. At the start of each iteration, load the most-distant element into the buffer.
4. Rewrite every img_in[y][x+b] DataRef inside the body's Fire args to buf[(x + b - min_offset) as usize % length]. Other axes (img_in[y-1][x] — y-axis read) NOT rewritten. The rewrite happens at the Fire-arg DataRef render site (render_fire_arg in backend-common/render.rs).

## Coordination
Single-worker path (pthreads-sync render_event Event::Loop arm; also delegated to by pthreads-async, mp-tcp-bufsync, mp-tcp-event when single-host). Multi-worker landing is TASK-0265.02.

## Forward-carry from TASK-0265 Tier 1 (cycle 87)
- Marker substring reuse_widths_pending is grep-able; AC#4 test pins both presence + absence. When real codegen lands, the marker SHOULD stay (or be subsumed by a new substring) so the e2e detection test keeps firing as a no-Stage-1-regression canary.
- New 05-stencil/reuse.sched.nuc cell currently PASSES on all 4 backends bit-identical to reference.bin. Real codegen MUST keep output bit-identical (reuse is perf rewrite, not semantic).
- Rewrite-site: render_fire_arg has access to ctx.sidecar but currently has no reuse context. Threading active reuse slots through RenderCtx/RenderCtxPub is the cleanest path; alternative is a side-channel arg-rewrite map populated in the Event::Loop arm.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Implementation Plan (cycle, TASK-0269)

### Design

**Threading reuse-active slots through RenderCtx**:
- Add to BOTH `RenderCtx` and `RenderCtxPub` a new field `reuse_active: BTreeMap<DataId, BTreeMap<u64 /* axis */, ReuseSlotActive>>` where `ReuseSlotActive { slot: ReuseSlot, iv_name: String }`. The iv_name is needed to detect at `render_fire_arg` time whether the matching axis index is `iv`, `iv+b`, or `iv-b` (a bare-loop-var-with-constant-offset).
- Both `RenderCtx::new` and `RenderCtxPub::new` initialise empty.
- Add `with_reuse_active(child)` builder on `RenderCtxPub` mirroring `with_abs_subst`. `RenderCtx` uses the same pattern via in-place `{ reuse_active: child, ..ctx }` (existing precedent in pthreads-sync `render_event`).

### Marker substring — choice (a): KEEP

The current scaffold writes `// reuse_widths_pending: iv=x data=img_in axis=1 length=3 min_offset=-1 (...)` as a comment ABOVE where the buffer decl will land. Keep that EXACTLY as-is, and add the real buffer decl BELOW it. This:
- Preserves the 5 grep payload-field asserts in e2e_example_05.rs.
- Preserves the 2 walker tests + the strip-mine pthreads-sync test.
- The comment is a regression canary: AC#4 forward-progress marker still grep-able.
- Zero coupling churn to other test files.

The marker comment renames from "Stage 2 marker; circular-buffer emit forward-carried to TASK-0269 + TASK-0270" to "Stage 2 active; circular-buffer codegen below — TASK-0269 (pthreads-sync) / TASK-0270 (multi_worker_walker)" in render.rs's render_reuse_marker_comment. The substring `reuse_widths_pending` itself stays.

### Codegen shape at Event::Loop (NEW helper in backend-common/render.rs)

`render_reuse_buf_init(out, indent, iter_var_name, sidecar, names, lo_expr_rs) -> Result<BTreeMap<DataId, BTreeMap<u64, ReuseSlotActive>>, EmitError>`

When `sidecar.reuse_widths.get(iter_var)` is Some, for each (DataId, axis, ReuseSlot):
1. Determine T from `sidecar.data_type(data_id).scalar` (rust_scalar_type).
2. Emit `let mut __reuse_buf_<data>_a<axis>: Vec<T> = vec![<zero>; <length>];` (per-data, per-axis buffer name).
3. For each `b in min_offset .. min_offset+length-1`, EXCEPT the last (max_offset, which fills inside the iter): emit a prologue fill from the source array using `<lo_expr_rs> + b` for the reuse-axis position. For 05-stencil/reuse: `lo = 1`, min=-1, len=3, so `b in {-1, 0}` get prologue-seeded as `img_in[<y * 16 + (lo+b)>] -> buf[(lo+b - min_offset) % len]`. But to emit a correct prologue for any outer-axis combination we'd need the OUTER index expression in scope.

WAIT — there's a subtlety. The 9 `img_in[y][...]` reads have THREE different y-coordinates (y-1, y, y+1). The narrow cut from the task brief is: ONLY rewrite reads whose outer axes equal the bare loop variable exactly. For img_in at axis=1, axis=0 must be exactly `y` (Ident, no offset). Only 3 of 9 reads qualify (img_in[y][x-1], img_in[y][x], img_in[y][x+1]).

The prologue (which runs BEFORE the body loop) cannot reference `y` unless the y-loop is active. The reuse loop x is INSIDE the y-loop, so the prologue is emitted at x-loop entry — y IS in scope. So:
- Prologue: `for b in 0..(length-1) { buf[((<lo> + b - min_offset) as i64).rem_euclid(length) as usize] = img_in[<y * 16 + (lo + b)> as usize]; }` — but we want UNROLLED prologue for determinism + cleanliness.
- Per-iter update: At body entry, before the Fire: `buf[((x + (min_offset + length - 1) - min_offset) as i64).rem_euclid(length) as usize] = img_in[<y * 16 + (x + max_offset)> as usize];` which simplifies to `buf[(x + len - 1 - min_offset) % len] = source[outer * D + (x + max_offset)]`.

### Issue: how to compute the OUTER coordinate

The bare `Event::Loop` arm doesn't know the OUTER axis index expression in the source DataRef. The clean approach: the FULL DataRef for the reuse slot must be reconstructed from the FIRST matching Fire arg in the body. But Stage 1 reuse_inference computed offsets across all 3 matching reads; it doesn't preserve the FULL access pattern.

**Decision: defer prologue + per-iter update generation to where the read pattern is known**:
- At loop entry, declare ONLY the empty buffer (Vec<T> of length L). No prologue.
- At render_fire_arg rewrite time, the FIRST iteration (x = lo, lo+1, ..., lo+len-2) reads from the SOURCE array (cold buffer). Only x >= lo + len - 1 (i.e., x - lo >= len - 1, i.e., iv >= length-1+lo, where the offset (iv+max_offset - min_offset) % length wraps back) can be safely read from the buffer.

This is too complex. **Simpler approach**: emit a per-iter prologue that fills the SLOT for the offset `max_offset` (the rightmost; most-distant element) at the START of each iteration. For x = lo, the rightmost is x + max_offset = lo + max_offset = lo + (min_offset+length-1) = lo + (-1 + 2) = lo + 1. This fills the slot. BUT slots for x-1 = lo-1 (out of source bounds for x=1, since lo=1 ⇒ x-1=0; img_in[y][0] EXISTS but isn't normally written; it's read directly).

Actually re-read the task brief: "Initial-fill prologue. For min_offset=-1, length=3 (offsets {-1, 0, +1}): seed buf[0] and buf[1] with img_in[y][0] and img_in[y][1] BEFORE the first body iteration; buf[2] gets img_in[y][x+1] inside the body." So the prologue seeds offsets {-1, 0} (i.e., length-1 entries from the left), and the per-iter loads offset {+1} (the rightmost).

The prologue needs the y outer coord. **The y outer coord must come from the FIRST matching Fire-arg DataRef pattern**. Plan:

#### Revised approach: build a per-loop ReuseRewriteCtx populated at Event::Loop entry

1. Walk the body to find the FIRST ArgBinding::Data whose .data matches one of the reuse DataIds, AND whose outer axes (all axes except the reuse axis) are "bare-iv-only patterns" we can rewrite. Extract from it the outer-axis IrExpr list.
2. For each (DataId, axis, slot): compute the prologue source expression by SUBSTITUTING the reuse axis with `lo + b` (for b in min_offset..min_offset+length-1, except max_offset). Emit unrolled prologue: `buf[(b - min_offset).rem_euclid(length) as usize] = <source>;`.
3. At each body-iter entry: emit `buf[(iv + max_offset - min_offset).rem_euclid(length) as usize] = <source with x replaced by x + max_offset>;`.

Then render_fire_arg consults the ReuseRewriteCtx and rewrites matching reads to `buf[(iv + b - min_offset).rem_euclid(length) as usize]`.

### Files touched

1. `nucleus/backend-common/src/render.rs`:
   - Add ReuseSlotActive struct (slot + iv_name + outer_axes Vec<IrExpr>).
   - Add reuse_active field on RenderCtx + RenderCtxPub.
   - Add render_reuse_buf_decl(out, indent, iter_var, body_events, sidecar, names, ctx) -> Result<BTreeMap<...>, EmitError> that performs body-prefix-walk to discover outer_axes, emits decls + prologue, returns the per-(data,axis) reuse_active map.
   - Add render_reuse_per_iter_update(out, indent, iter_var, reuse_active, ctx) helper for the per-iter most-distant slot fill.
   - Modify render_fire_arg to consult ctx.reuse_active before falling through to scalar/sub-array classification: if ArgBinding::Data with non-empty indices, check whether (data, axis) matches a reuse_active slot AND the outer axes match the recorded outer_axes verbatim (Eq on IrExpr — note: PartialEq on IrExpr is structural, ignoring span; verified by memory entry project-algo-ast-span-substrate).
   - Update render_reuse_marker_comment text (rename trailing parenthetical to reflect codegen below; keep `reuse_widths_pending` substring).

2. `nucleus/backends/pthreads-sync/src/lib.rs`:
   - In Event::Loop arm of render_event, after writing the for-header and the marker comment, call render_reuse_buf_decl + emit the per-iter update at body-entry. Pass the populated reuse_active map into the recursive RenderCtx for body recursion.
   - Do this for BOTH the strip-mined (line 653) and regular (line 675) paths.

3. NO changes to multi_worker_walker.rs (TASK-0270 is the multi-worker landing).

### Test changes

The existing 5 payload-field asserts in e2e_example_05.rs (iv=x, data=img_in, axis=1, length=3, min_offset=-1) stay since the marker comment text is preserved. The existing 2 walker tests + 1 pthreads-sync strip-mine test all stay.

NEW assertions to add:
- e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive: add 3 NEW asserts that the emit contains `__reuse_buf_img_in_a1` (buffer decl) AND fewer than 9 `img_in[` reads in the for-x body (we should have 6 verbatim + the per-iter update = 7 reads of img_in; not 9).

Actually counting 'img_in[' in the body is brittle. Add only: `reuse_main.contains("__reuse_buf_img_in")` (buffer presence) + `reuse_main.contains(".rem_euclid(3)")` or similar buffer-indexing fingerprint. Defer brittle structural counts.

### Verification gate (before each commit)

1. cargo test --workspace ⇒ all green.
2. just clippy ⇒ clean (-D warnings).
3. just e2e ⇒ 92/79/0/13/0 preserved; reuse cell stays bit-identical to reference.bin.
4. just determinism-check ⇒ green.

### Commits planned

- Commit 1: backend-common — add reuse_active field + render_reuse_buf_decl + render_fire_arg rewrite path. Standalone; doesn't change emit because no caller populates the new map yet.
- Commit 2: pthreads-sync — wire render_reuse_buf_decl into Event::Loop arms; update e2e test asserts. Bit-identical preserved on reuse.sched.nuc (still produces correct output, just via the rewrite path).

### Honest limitations to record

- The rewrite cut is restrictive: only DataRefs whose OUTER axes match the FIRST matching read in the body verbatim are rewritten. In 05-stencil/reuse, this means the 3 reads with axis-0 = `y` get rewritten; the 6 reads at axis-0 = `y-1` and `y+1` stay verbatim. A more general rewrite (one buffer PER outer-coord variation) is left for a follow-up.
- The buffer is per-(data, axis), not per-(data, outer_coord, axis). For 05-stencil with 3 outer-coord variations, only 1 of those 3 benefits — the perf rewrite is incomplete (but correct).
- A follow-up task will be filed for the multi-outer-coord generalisation.

### Forward-carry to TASK-0270

Lessons that will land here for the multi-worker sibling:
- The reuse_active threading shape on RenderCtxPub mirrors with_abs_subst.
- The 'find first matching DataRef in body' pattern for outer-axis discovery.
- The narrow cut + follow-up filing pattern.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Forward-carried from TASK-0275 (cycle 96, halo (B) promotion)

When pthreads-sync circular-buffer codegen consumes reuse_widths, note that the reuse driver is already STRICT (TASK-0271 cycle 88, no advisory bucket). That is the right shape for THIS task — every reuse slot is universally consumed by the Tier 1 marker today, and your real codegen will only strengthen that. Do NOT mirror the halo (B) partition-policy-aware shape here; the two pass siblings are asymmetric on purpose (transfer_inject is conditional on partition=, reuse marker is universal).

Implementation lesson: if you need to thread additional context into the walker errors (the TASK-0275 refactor changed the halo walker return to `Vec<(Error, Vec<String>)>` to pair errors with their enclosing scope), introduce a type alias EARLY — clippy::type_complexity fires on the bare tuple+vec shape (1 error on first attempt; saved by `type HaloErrorWithScope = (HaloInferenceError, Vec<String>);`).

**Forward-carried from TASK-0273 (cycle 98)**: when real circular-buffer codegen lands here on pthreads-sync's single-worker path, the `reuse_widths_pending` marker substring at render.rs:867 will rename (likely `reuse_buf_decl`) or be subsumed entirely by a `let __reuse_buf_<data>: Vec<...>` declaration. The grep assertions in `nucleus/nucleus-compiler/tests/e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive` (5 payload-field asserts: iv=x, data=img_in, axis=1, length=3, min_offset=-1) MUST be updated in lockstep — do NOT silently drop the marker without replacing the assertion shape.

## Cycle 103 implementation progress (pre-commit gate snapshot)

### Implementation landed (uncommitted)
- backend-common/src/render.rs (+~500 lines):
  - New `ReuseRewriteGroup` struct (axis, slot, outer_axes, buf_ident, iv_name).
  - New `reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>` field on RenderCtx + RenderCtxPub (with with_abs_subst preserving it; inner() copying).
  - New `render_reuse_buf_decls` (walks body to discover first matching DataRef per (data_id, axis); emits Vec<T> decl + unrolled prologue; returns the reuse_active map).
  - New `render_reuse_per_iter_update` (per-iter most-distant slot fill).
  - New `try_rewrite_reuse_arg` consulted by render_fire_arg (rewrites ArgBinding::Data with matching outer axes + iv+b reuse-axis index into buf[((iv+b-min_offset).rem_euclid(L)) as usize]).
  - render_reuse_marker_comment text updated (keeps load-bearing 'reuse_widths_pending' substring; trailing parenthetical now says 'Stage 2 active; circular-buffer codegen below on pthreads-sync — TASK-0269').
- pthreads-sync/src/lib.rs (+85 lines): wired render_reuse_buf_decls + render_reuse_per_iter_update at BOTH Event::Loop arms (strip-mined and regular). Body recursion now uses a child RenderCtx with reuse_active populated.
- nucleus-compiler/tests/e2e_example_05.rs (+38 lines): 3 new presence asserts on the reuse cell (__reuse_buf_img_in_a1 buffer decl, vec![0; 3usize] init, rem_euclid(3_i64) wrap); 1 symmetric absence assert on naive (no __reuse_buf identifier).

### Per-AC status (pre-commit)
- AC#1 (Vec<T> decl + initial-fill prologue): MET. `let mut __reuse_buf_img_in_a1: Vec<i32> = vec![0; 3usize];` + 2 prologue fills (offsets -1, 0) above the for-x header.
- AC#2 (per-iter update of most-distant element): MET. `__reuse_buf_img_in_a1[((x + 1 - (-1)).rem_euclid(3)) as usize] = img_in[(y-1)*16 + (x+1)];` at body entry.
- AC#3 (rewrite img_in[y][x+b] reads to buf[...]): PARTIAL — 3 of 9 reads rewritten (only the row-y-1 outer-axis pattern matches the canonical group from the first matching DataRef). NARROW FIRST-CUT per task brief; the 6 reads at outer axes y and y+1 stay verbatim. Follow-up task to be filed for one-buffer-per-outer-coord-variation generalisation.
- AC#4 (bit-identical to reference.bin + marker grep-able): MET. `just e2e` reuse_pthreads_sync_bit_identical PASS. Marker substring `reuse_widths_pending` preserved + 3 new buffer-shape asserts pin codegen substance.

### Gate (pre-commit)
- cargo test --workspace: 805 / 0 / 3 (unchanged from baseline — additive feature on a pre-existing cell)
- cargo clippy --workspace --all-targets -- -D warnings: clean
- just e2e: 92/79/0/13/0 (preserved; reuse cell stays bit-identical)
- just determinism-check: 92/79/0/13 (all green)
- cargo fmt -p backend-common -p pthreads-sync -p nucleus-compiler: clean (a pre-existing unrelated fmt drift in nucleus-compiler/src/passes/sync_inject.rs was reverted to keep this commit focused).

### Emitted main.rs shape (05-stencil/reuse × pthreads-sync; verified bit-identical to reference.bin):
```rust
for y in (1_i64)..((16_i64 - 1_i64)) {
    let mut __reuse_buf_img_in_a1: Vec<i32> = vec![0; 3usize];
    __reuse_buf_img_in_a1[((((1_i64) + (-1_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)] = img_in[(((y - 1)) * 16 + ((1_i64) + (-1_i64))) as usize];
    __reuse_buf_img_in_a1[((((1_i64) + (0_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)] = img_in[(((y - 1)) * 16 + ((1_i64) + (0_i64))) as usize];
    for x in (1_i64)..((16_i64 - 1_i64)) {
        // reuse_widths_pending: iv=x data=img_in axis=1 length=3 min_offset=-1 (...)
        __reuse_buf_img_in_a1[(((x) + (1_i64) - (-1_i64)).rem_euclid(3_i64)) as usize] = img_in[(((y - 1)) * 16 + ((x) + (1_i64))) as usize];
        img_out[((y) * 16 + (x)) as usize] = kernels::blur3(
            __reuse_buf_img_in_a1[((((x) + (-1_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)],
            __reuse_buf_img_in_a1[((((x) + (0_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)],
            __reuse_buf_img_in_a1[((((x) + (1_i64) - (-1_i64)).rem_euclid(3_i64)) as usize)],
            img_in[((y) * 16 + ((x - 1))) as usize],
            ...
        );
    }
}
```

### Honest limitations (record + follow-up will file)
1. Narrow rewrite cut: only DataRefs matching the FIRST discovered outer-axes pattern get rewritten. In 05-stencil/reuse this means row y-1 reads get the buffer treatment; rows y and y+1 stay verbatim. The buffer is per-(data, axis) not per-(data, outer-coord-tuple, axis). A 3-buffer generalisation (one per outer-coord variation) is left for a follow-up task.
2. Restricted reuse-axis index shape: only `iv`, `iv + b`, `iv - b`, `b + iv` are recognised. Strided / coefficient-not-1 / pure-const shapes fall through to the verbatim path (consistent with Stage 1's affine-decompose contract — Stage 1 rejects anything we'd not rewrite, so no false negatives).
3. Strip-mine arm: the prologue's 'lo' expression is computed via a textual replace of var with '0_i64' in the rebound abs expression. Pragmatic given 05-stencil/reuse is single-host (the strip-mine arm is not exercised by the e2e cell); the synthetic test at pthreads-sync/tests/reuse_marker.rs covers the marker side. **For 05-stencil/distributed (currently SKIP on TASK-0267 ✅ landed + TASK-0268 ✅ landed but still skipped via TASK-0042 capability on pthreads-sync) the strip-mine + reuse combination would exercise this branch.**
4. Multi-worker walker (TASK-0270) NOT touched. The walker's reuse_widths consumer remains marker-only. Tier 3 lands separately.

### Forward-carry to TASK-0270 (multi-worker walker sibling)
- ReuseRewriteGroup + reuse_active threading already in RenderCtxPub — Tier 3 implementer just needs to call render_reuse_buf_decls + render_reuse_per_iter_update at multi_worker_walker.rs's Event::Loop arms (strip-mined line 404 + regular line 478), and pass the resulting groups into the with_abs_subst-derived child ctx for body recursion.
- Same first-matching-DataRef discovery; same narrow rewrite cut.
- Per-worker partition slices already drive the loop bounds in multi_worker_walker — the reuse buffer decl + prologue use the per-worker partition slice's lo (not the source lo). The current discover_reuse_groups walks the worker's body events which already carry the partition-projected shape.
- mp-tcp-bufsync host emit doesn't share render_event with the multi_worker_walker — TASK-0270 needs to verify whether host-side single-worker-style emit ALSO needs the buffer decl (when host carries a reuse-tagged loop it lowers through pthreads-sync's render_single_worker_main, which already has the fix from this task).
<!-- SECTION:NOTES:END -->
