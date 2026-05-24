---
id: TASK-0273
title: Extend reuse_widths_pending marker assertion to multi_worker_walker path
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 08:46'
updated_date: '2026-05-24 12:04'
labels:
  - M5
  - test-gap
  - reuse
dependencies:
  - TASK-0265
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0265 cycle-87 review (architect P2 finding).

TASK-0265 Tier 1 landed `render_reuse_marker_comment` at TWO sites:
- `nucleus/backends/pthreads-sync/src/lib.rs::render_event` (single-worker emit path)
- `nucleus/backend-common/src/multi_worker_walker.rs` (multi-worker emit path)

The grep test `nucleus/nucleus-compiler/tests/e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive` only exercises the FIRST site. It runs `--backend pthreads-sync` against the single-host `reuse.sched.nuc` schedule, which routes through `render_event`. The multi_worker_walker.rs call site is NOT covered.

A regression that drops the marker emit from `multi_worker_walker.rs` (but not `render_event`) would silently pass.

## Why the gap exists today

The only shipped multi-worker reuse schedule is `nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc` which carries `loop x : block=64, vectorize=8, reuse;`. That cell is currently [[skip]]ped across all 4 backends due to TASK-0267 (host-Push synthesis drop) + TASK-0268 (sync_inject barrier deadlock). Until those land, there is no e2e cell exercising multi_worker_walker.rs with a reuse-tagged loop.

## Two paths to close

**Option A (blocked-on-siblings)**: wait for TASK-0267 + TASK-0268 to land, then add `reuse_distributed_multi_worker_marker_present` test that runs `--backend pthreads-sync` (or any backend; all 4 share the walker) against `distributed.sched.nuc` and greps for the marker on each per-worker emit.

**Option B (synthetic fixture)**: add a hand-built fixture in `nucleus/nucleus-compiler/tests/` that builds an ACFG with `reuse_widths` populated AND `partition_worker_ranges` populated, then calls the walker directly via `render_worker_events` and asserts the marker substring appears. Decouples this coverage from TASK-0267/0268 closure. Costs: hand-building the fixture + threading the `RenderCtxPub` etc.

## Acceptance

1. A test in `nucleus/nucleus-compiler/tests/` OR `nucleus/backend-common/tests/` that exercises `multi_worker_walker.rs`'s call to `render_reuse_marker_comment` with a non-empty `reuse_widths` sidecar entry AND asserts the marker substring appears in the per-worker emit.
2. Symmetric ABSENCE: same fixture with empty `reuse_widths` asserts ZERO occurrences (defensive — catches an over-eager emit).
3. Test runs in < 30s (no full cargo build cycle if possible — favour direct walker invocation).

## Dependencies

- Forward-carried from: TASK-0265 (cycle-87 architect P2 review item).
- Option A depends on: TASK-0267 + TASK-0268 (the runtime bugs blocking 05-stencil/distributed).
- Option B: standalone.
- Related: TASK-0269 (when real circular-buffer codegen lands on the walker, the marker substring may rename to `reuse_buf_decl` or similar — update test in lockstep).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Plan (cycle 98)

Option B (standalone synthetic). Clone fixture shape from `nucleus/backend-common/tests/multi_worker_blocked_rebind.rs`.

New file: `nucleus/backend-common/tests/multi_worker_reuse_marker.rs`.

Two tests:
1. `multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated`:
   - WalkerCtx fixture with sidecar.reuse_widths[iv][data][axis=0] = ReuseSlot{length=3, min_offset=-1}.
   - Event::Loop(iv, 0..16, body=[]) — non-strip-mine arm (block_tag: None), hits the line-478 call site.
   - Assert: contains 'reuse_widths_pending' substring; also iv name, data name, length, min_offset payload (catches drop-marker AND drop-payload regressions).
2. `multi_worker_walker_skips_reuse_marker_when_reuse_widths_empty`:
   - Same fixture but reuse_widths empty.
   - Assert !contains('reuse_widths_pending').

Path-verified facts:
- ReuseSlot at `nucleus_compiler::passes::reuse_inference::ReuseSlot` (re-exported from `nucleus_compiler` per lib.rs line 72: `apply_reuse_inference, apply_reuse_inference_advisory, ReuseInferenceError, ReuseSlot`).
- backend-common's only dep is nucleus-compiler — direct import works.
- Map shape: `BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64 /* axis */, ReuseSlot>>>` (axis is u64, not usize).
- WalkerCtx fields: names, sidecar, rendezvous_prefix, rendezvous_ids, pair_tiles.
- The line-478 call site fires the marker AFTER writing 'for {var} in (lo)..(hi) {', so an empty body is fine.

No production code changes.

## Final summary (cycle 98 — TASK-0273 Done)

**Commit**: 1e62b4b — `backend-common/tests: TASK-0273 pin reuse marker via multi_worker_walker (Option B)`.

**Approach**: Option B (standalone synthetic fixture) chosen over Option A (waiting on TASK-0267 + TASK-0268). Decouples coverage from those still-open runtime-bug tasks; future e2e cell on 05-stencil/distributed will provide complementary integration coverage when those land.

**Deliverable**: `nucleus/backend-common/tests/multi_worker_reuse_marker.rs` (new, 252 lines). Two tests:
1. `multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated` — presence + 5 payload-field assertions (iv, data, axis, length, min_offset). The 5-field payload coverage exceeds the single-worker e2e test's pin shape and catches a subtler regression class (payload-stripping refactor).
2. `multi_worker_walker_skips_reuse_marker_when_reuse_widths_empty` — symmetric absence (`assert_eq!(count, 0)`).

**Per-AC**:
- AC#1 (test exists, asserts marker substring + multi_worker_walker call site): YES — both tests route through `backend_common::multi_worker_walker::render_worker_events`, hitting the non-strip-mine `Event::Loop` body-entry call site at multi_worker_walker.rs:478.
- AC#2 (symmetric absence): YES — test 2 asserts `out.matches(...).count() == 0`.
- AC#3 (<30s, no full cargo build): YES — direct walker invocation; both tests complete in <0.01s per cargo test output.

**Gate** (cycle-98 baseline preserved):
- `cargo test -p backend-common --test multi_worker_reuse_marker`: 2 passed; 0 failed; finished in 0.00s.
- `just test`: 0 failures across workspace (20 crates).
- `just e2e`: total: 92   pass: 77   fail: 0   skipped: 15   required-fail: 0.
- `just determinism-check`: total: 92   pass: 77   fail: 0   skipped: 15.
- `just clippy`: clean (`-D warnings`).
- `just fmt-check`: clean (initial draft tripped one let-binding line-break; `cargo fmt --all -- backend-common/tests/multi_worker_reuse_marker.rs` normalised, re-checked clean).

**Lessons forward-carried**:
- `ReuseSlot` import path: `nucleus_compiler::passes::reuse_inference::ReuseSlot` (NOT `nucleus_compiler::sidecar::ReuseSlot` — sidecar only mentions it by qualified path in its field type). The `pub use` re-export at lib.rs:72 also exposes it as `nucleus_compiler::ReuseSlot`; either works.
- `sidecar.reuse_widths` axis key type is `u64`, not `usize` as the task description suggested. Verified at sidecar.rs:298.
- The non-strip-mine `Event::Loop` arm (block_tag=None) is the simpler call site to exercise — empty body still emits the marker because `render_reuse_marker_comment` fires at body-entry BEFORE recursing into `body`. No need to construct a populated `Fire` body for the marker contract test (would only add ceremony).
- TASK-0269 + TASK-0270 forward-link: when real circular-buffer codegen lands, the `reuse_widths_pending` substring will rename/subsume. The new test file embeds a module-level forward-carry warning to the next implementer; same warning appended to TASK-0269 + TASK-0270 notes.

**No follow-up tasks filed**: production code untouched; no defects discovered.

**Honest limits**:
- This is an OPTION-B coverage shim, NOT a full integration test. Once TASK-0267 + TASK-0268 land and 05-stencil/distributed unskips, the resulting end-to-end build-and-grep would provide a stronger guarantee (real ACFG + real partition + real walker). Option B's value is that it pins the contract WITHOUT waiting on those — but it cannot catch a regression in the upstream ACFG `reuse_widths` population (only in the walker's consumption of it). The single-worker e2e test still provides that upstream coverage on the `reuse.sched.nuc` single-host path.
<!-- SECTION:NOTES:END -->
