---
id: TASK-0239
title: >-
  Extract shared multi-worker event-walker between pthreads-sync and
  pthreads-async (TASK-0228 Wave B-2 de-dup follow-up)
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 07:35'
updated_date: '2026-05-22 09:43'
labels:
  - tech-debt
  - M4
  - backend
dependencies:
  - TASK-0228
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 26 (TASK-0228 Wave B-2) duplicated pthreads-sync's render_worker_events + render_wait_assign + leading_axis_slice + collect_pre_init_sets into pthreads-async/src/multi_worker.rs (~400 LoC duplicate). The two implementations differ ONLY in 'slot' vs 'ring' variable prefix and the file-scope Slot<T> vs Ring<T> struct emission; everything else (Fire/Loop/Sync/Wait/check_frame, barrier identity, pre-init computation, slice-paste gather, partition_worker_ranges per-worker bounds) is byte-for-byte the same emit-string shape.\n\nThe right architectural move is to lift the shared walker into a parameterized helper (in pthreads-sync's pub surface, OR in a new backend-common crate). The parameter is the rendezvous-primitive shape: (struct-emit fn, instance-emit fn, callsite var-prefix). Then pthreads-async's Plan::emit becomes ~80 LoC of orchestration plus the substrate calls.\n\nThis was NOT done in cycle 26 because the precedent in this codebase is 'duplicate first, then extract once N>=3 sites exist' (cf TASK-0222 which did exactly that for the four check_frame emit-string templates after pthreads-sync + mp-tcp-bufsync both exhibited them). pthreads-async is now the second site for the worker-events walker; the extraction is justified, but it is its own cycle.\n\nA drift test (codegen-output-equal-modulo-substitution between the two backends on a real fixture) would be the right defense, OR the extraction itself which eliminates the duplication structurally.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-31 Plan (mped-architect-impl)

**Design: (B) direct-parameter shared walker (no trait).**

Rationale: the difference between backends is a single string ("slot" vs "ring"). The existing precedent in pthreads-sync/src/lib.rs (RenderCtxPub) is direct-pass, not trait-based. A trait would introduce dispatch ceremony without any second-axis of variation.

**SlotId = RingId = usize** — confirmed identical, so the map shape `BTreeMap<(DataId, SeqTag), usize>` is shareable verbatim.

## Files to modify

1. **NEW**: `nucleus/backends/pthreads-sync/src/multi_worker_walker.rs` (or add to lib.rs).
   - `pub struct WalkerCtx<'a>` bundling { names, sidecar, rendezvous_prefix: &str, rendezvous_ids: &BTreeMap<(DataId, SeqTag), usize>, pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile> }.
   - `pub fn render_worker_events_pub(&self, worker, events, out, indent, prefix) -> Result<(), EmitError>` — the walker, parameterised.
   - `pub fn render_wait_assign_pub(&self, name, data, seq, rhs) -> Result<String, EmitError>`.
   - `pub fn leading_axis_slice_pub(&self, data, tile) -> Result<Option<LeadingAxis>, EmitError>`.
   - `pub struct LeadingAxis { lo, hi, stride }`.
   - `pub fn collect_pre_init_sets_pub(events, waited, whole, indexed)`.
   - `pub fn collect_xfer_pairs_pub(events, out)`.
   - `pub fn collect_worker_rendezvous_pub(events, ids, out)` (replaces collect_worker_slots / collect_worker_rings).
   - `pub fn collect_barriers_by_tag_pub(events, f)`.

2. **MODIFY**: `nucleus/backends/pthreads-sync/src/lib.rs` — re-export the above.

3. **MODIFY**: `nucleus/backends/pthreads-sync/src/multi_worker.rs` — call the shared helpers, delete the now-duplicated local copies.

4. **MODIFY**: `nucleus/backends/pthreads-async/src/multi_worker.rs` — call the shared helpers via `pthreads_sync::` (importing from the re-export), delete ~400 LoC of duplicated walker code.

## Plan emit stays per-backend

The substrate decl (Slot vs Ring struct) and per-pair instance alloc (Slot::new() vs Ring::new(cap)) stay in their respective Plan::emit functions — they ARE the only real semantic difference between the two backends. Only the per-worker-events walker is shared.

## Byte-equivalence

The refactor MUST be pure code-reorganisation. The before/after diff for 02-split-add × pthreads-sync, × pthreads-async, and 13-pipeline-parallel × pthreads-async must be empty.

Captured before-refactor snapshots in /tmp/task-0239-byte-check/before/.

## Cycle-31 Implementation Complete (READY FOR REVIEW + COMMIT)

### Design (final)
Option (B) direct-parameter. The shared walker lives in NEW module `nucleus/backends/pthreads-sync/src/multi_worker_walker.rs`. `WalkerCtx` bundles { names, sidecar, rendezvous_prefix, rendezvous_ids, pair_tiles }. Both backends construct their own `WalkerCtx` and call `walker::render_worker_events` — pthreads-sync passes `rendezvous_prefix: "slot"`, pthreads-async passes `"ring"`.

### Files changed
1. **NEW** `nucleus/backends/pthreads-sync/src/multi_worker_walker.rs` (634 lines, ~280 LoC pure new code + comments; the rest is doc-port from the original sites)
   - `pub struct WalkerCtx<'a>`
   - `pub struct LeadingAxis` (fields module-private)
   - `pub type RendezvousId = usize;`
   - `pub fn render_worker_events` (entry; internally calls a private `render_worker_events_inner` to avoid re-building the RenderCtxPub on every recursion)
   - `pub fn render_wait_assign`
   - `pub fn collect_xfer_pairs`
   - `pub fn collect_worker_rendezvous` (replaces collect_worker_slots/collect_worker_rings — both walked identically)
   - `pub fn collect_barriers_by_tag`
   - `pub fn collect_pre_init_sets`
   - private `fn leading_axis_slice`
2. **MODIFY** `nucleus/backends/pthreads-sync/src/lib.rs` (+1 line) — `pub mod multi_worker_walker;`
3. **MODIFY** `nucleus/backends/pthreads-sync/src/multi_worker.rs` (-493 lines)
   - SlotId = RendezvousId alias
   - Plan::build calls walker::collect_xfer_pairs + walker::collect_barriers_by_tag
   - Plan::slots_used_by calls walker::collect_worker_rendezvous
   - Plan::render_worker_body constructs WalkerCtx { rendezvous_prefix: "slot" } and dispatches walker::render_worker_events
   - Plan::collect_pre_init calls walker::collect_pre_init_sets
   - DELETED: render_worker_events method, render_wait_assign method, leading_axis_slice method, LeadingAxis struct, and the file-level helpers collect_xfer_pairs / collect_worker_slots / collect_barriers_by_tag / collect_pre_init_sets
4. **MODIFY** `nucleus/backends/pthreads-async/src/multi_worker.rs` (-403 lines)
   - RingId = RendezvousId alias
   - Same WalkerCtx dispatch via walker::render_worker_events with rendezvous_prefix: "ring"
   - Plan::worker_rings uses walker::collect_worker_rendezvous
   - Plan::collect_pre_init uses walker::collect_pre_init_sets
   - Cycle-26 status docstring updated (was claiming TASK-0239 dedup was pending; now reflects that it landed)
   - DELETED: all duplicated walker code + helpers + LeadingAxis struct; test reference to local collect_barriers_by_tag rewired to walker:: form
   - Kept per-backend: Plan struct (carries ring_caps), Plan::emit (substrate decl + per-pair Ring::new(cap) alloc), collect_unique_count_check_frames (uses pthreads_sync::CountCheckLoop)

### LoC delta (real numbers)
- Before: pthreads-sync/multi_worker.rs 1147 + pthreads-async/multi_worker.rs 1269 = **2416 LoC** in the two affected files.
- After: pthreads-sync/multi_worker.rs 654 + pthreads-async/multi_worker.rs 870 + multi_worker_walker.rs 634 = **2158 LoC**.
- Net: **-258 LoC** (modest because the new module carries substantial inherited docstrings + the WalkerCtx scaffolding; the pure-code duplication eliminated is roughly the ~400 LoC the task estimated, but the doc-port partially offsets that line count).
- `git diff --stat` for the two modified files: **-967 deletions / +76 insertions**, plus 634 lines of NEW shared module — net ~-258 LoC at the workspace level.

### Byte-equivalence proof (3 reference points)
All emitted main.rs files byte-identical pre-/post-refactor:
- 02-split-add/split × pthreads-sync: IDENTICAL
- 02-split-add/split × pthreads-async: IDENTICAL
- 13-cnn-inference/pipeline_parallel × pthreads-async: IDENTICAL

Snapshots in `/tmp/task-0239-byte-check/{before,after,after2,after3}/` (last three are post-tightening confirmations; all match the BEFORE).

### Gate numbers
- `just test`: ALL crates pass; test result: FAILED count = 0.
- `just clippy`: clean (cargo clippy --workspace --all-targets -- -D warnings).
- `just e2e`: total: 54   pass: 47   fail: 0   skipped: 7   required-fail: 0  (the locked baseline).
- `just determinism-check-negative`: "OK: determinism check correctly bit on injected nondeterminism" (NUC_NONDET_PERTURBED_CELLS=47).
- `just xbackend-check-negative`: "OK: cross-backend differential correctly bit on injected mp-tcp corruption" (CORRUPTED_APPLIED=14, DETECTED=1).
- 3x stress e2e: 54/47/0/7 on all three runs (stable).

### Cross-checks for the headline thesis
The pthreads-sync / pthreads-async cross-backend bit-identical differential is the falsifiability claim that this refactor MUST preserve. Confirmed both directly (the byte-identical emit on 02-split-add and 13-pipeline-parallel) and transitively (the e2e differential gate, which compares stdout across backends per cell, was green on all 47 passing cells across three stress runs).

### Honest limits
1. The walker module is in pthreads-sync, not a separate backend-common crate. Justification: pthreads-async already depends on pthreads-sync (for the shared codegen helpers — TASK-0222, TASK-0238); adding a third crate would be more disruptive than the substantive content warrants. If a future M5+ backend wants the walker WITHOUT pulling in pthreads-sync's Slot<T> single-worker code, this becomes a real motivator for the extraction. NOT done this cycle.
2. mp-tcp-bufsync has its own walker (multi-process, different rendezvous shape — wire codec, not shared-memory). Intentionally untouched; the refactor is for the two shared-memory backends only.
3. No new follow-up tasks needed. The walker's pre-existing TASK-0117 honest-limit (assumes leading-axis partition for the slice-paste shape) carries forward verbatim — not a new debt introduced this cycle.

### Status
READY FOR REVIEW + COMMIT. Nothing left to do for TASK-0239.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 31 (2026-05-22) — closed. Lifted ~400 LoC of duplicated multi-worker event-walker out of both pthreads-sync and pthreads-async into a new shared module nucleus/backends/pthreads-sync/src/multi_worker_walker.rs (634 LoC pub surface). Both backends now route through the single source of truth, parameterised by rendezvous_prefix: &str ('slot' vs 'ring').

Design (option B, direct-parameter — no trait): the differentiation between backends is a single string. RendezvousId aliases usize and serves both backends' SlotId / RingId. The walker exports 9 pub items, each verified by the architect to have at least one external consumer. WalkerCtx struct carries (names, sidecar, per_worker, ring/slot_ids, pair_tiles, rendezvous_prefix, [optional partition rebinding for cycle-26 strip-mine guard]).

LoC delta: pre 2416, post 2158, net -258. The savings are modest because the new module preserves the inherited docstrings; the pure-code duplication eliminated is the ~400 LoC the task estimated.

Byte-equivalence verified: 02-split-add/split × {pthreads-sync, pthreads-async} AND 13-cnn-inference/pipeline_parallel × pthreads-async emit byte-identical to their pre-refactor snapshots. The refactor was pure code reorganisation with zero emission change — confirming the cycle-26 manual copy was correctly mechanical.

Gate (cycle 31):
- just test: 0 FAILED suites.
- just clippy: clean.
- just e2e: 54 / 47 / 0 / 7 stable across 4 runs.
- just determinism-check-negative: OK, PERTURBED=47.
- just xbackend-check-negative: OK, DETECTED=1 (APPLIED=14).

Preserved verbatim (architect-verified):
- TASK-0181 strip-mine fail-loud guard.
- Cycle-26 defensive double-guard (check_frame.is_some() && block_tag.is_some()).
- TASK-0212 partition_worker_ranges per-worker bounds override.
- TASK-0117 leading_axis_slice + render_wait_assign slice-paste gather.

Follow-up filed:
- TASK-0244 (LOW): move multi_worker_walker + the rest of the cross-backend pub surface into a dedicated backend-common crate. Same architectural smell TASK-0238 fixed for NameTables — the current arrow async -> sync isn't semantically real (they are siblings). Bounded, mechanical, deferred until a future backend forces it.

Review-gate (parallel read-only): both qa-test-runner + mped-architect GO. No HIGH / no MEDIUM-blocking findings; architect flagged the backend-common smell as MEDIUM with the explicit 'do not block, file follow-up' verdict.
<!-- SECTION:FINAL_SUMMARY:END -->
