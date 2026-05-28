---
id: TASK-0349
title: >-
  Codegen: whole-array broadcast init creates dead vec! init that triggers
  unused_assignments warning
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-27 18:03'
updated_date: '2026-05-27 23:59'
labels:
  - codegen
  - cosmetic
  - multi-worker
  - quality
dependencies: []
priority: low
---

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 220 plan (orchestrator-direct, per memory feedback-spawned-agents-refuse-code-edits).

SCOPE (AC#1-AC#5 = full closure across all 7 tier-1 backends).

ROOT-CAUSE ANALYSIS (empirically verified on /tmp/task0349-probe/spmv-dist):

For 17-spmv/distributed × pthreads-sync, each worker thread emits:
  let mut x: Vec<i32> = vec![0; 8];   // pre-init (dead)
  ...
  x = w0_slot_8.wait();                // whole-array recv (overwrites)

vs the slice-paste case (live pre-init):
  let mut col_idx: Vec<i32> = vec![0; 24];                                    // pre-init (PARTIALLY live)
  { let _tmp = w0_slot_0.wait(); col_idx[0usize..6usize].copy_from_slice(&_tmp[0usize..6usize]); }  // overwrites SLICE

The dead-init pattern fires whenever a data symbol's ONLY Wait(s) resolve to whole-array recv. The discriminator is `wait_slice(...) == None` in nucleus/backend-common/src/multi_worker_walker/wait.rs:237.

FIX SHAPE (Option (a) from task description, the cleanest):
1. New walker helper `collect_let_at_wait_data(events, pair_tiles, sidecar)`: returns BTreeSet<DataId> of data where (a) all Waits in `events` are whole-array (wait_slice returns None) AND (b) the data is NOT in `accumulate_waits` AND (c) the data is NOT also indexed-Fire-written. For these data, the pre-init `let mut` is provably dead.
2. Walker's render_wait_assign: for whole-array recv on data in `let_at_wait`, emit `let {name}: {rty} = {rhs};` instead of `{name} = {rhs};`. The typed let-binding declares-and-assigns in one statement.
3. WalkerCtx: add `let_at_wait_data: &BTreeSet<DataId>` field. Pass from each backend.
4. Per-backend `collect_pre_init`: subtract `let_at_wait_data` from the pre-init set.

TOUCHED FILES (estimate):
- nucleus/backend-common/src/multi_worker_walker/collect.rs (+ ~30 LoC new helper)
- nucleus/backend-common/src/multi_worker_walker/wait.rs (+ ~5 LoC signature + body change)
- nucleus/backend-common/src/multi_worker_walker/ctx.rs (+ ~1 LoC WalkerCtx field)
- nucleus/backend-common/src/multi_worker_walker/event_walker.rs (signature thread-through if used; ~3 LoC)
- 7 backend pre-init sites:
  - pthreads-sync/src/multi_worker.rs
  - pthreads-async/src/multi_worker.rs
  - openmp-rs/src/multi_worker.rs
  - mp-tcp-bufsync/src/plan/worker_program.rs
  - mp-tcp-poll/src/plan/worker_program.rs
  - mp-tcp-event/src/multi_worker/worker_program.rs
  - mp-uds-event/src/multi_worker/worker_program.rs
  Each: ~5 LoC (compute let_at_wait, subtract from pre-init set, pass to WalkerCtx or render_wait_assign).

VERIFICATION:
- Re-emit /tmp/task0349-probe/spmv-dist + cargo build: 0 unused_assignments warnings (baseline: 4).
- Re-emit for 16-jacobi/distributed, 07-matmul/distributed, 06-separable-filter/distributed, 08-histogram/distributed (other multi-worker distributed schedules with whole-array broadcast): 0 warnings each.
- All 7 backends: 0 warnings on all multi-worker distributed cells.
- nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e": all green; e2e 280/246/0/34/0 unchanged (AC#5 bit-identity).

HONEST RISKS:
- The cycle-128 meta-rule predicts the next cycle following a defect-class-sweep cycle (cycle 219) is the highest-risk cycle for that exact class. TASK-0349 is code-change not closure-narrative, so the structural-match is lower; but per-backend mirror code (7 backends touched identically) is the OTHER recurring sibling-defect axis (memory cycles 137 + 138-140 + 146 + 149 + 195b). MITIGATION: at each backend's pre-init edit, grep for the cohort tuple (`pthreads-sync.*pthreads-async`, etc.) to verify all 7 paired and use a centralized `subtract_let_at_wait` helper if duplication exceeds 3 sites.
- The `let_at_wait` classifier MUST handle multi-Wait per data (a data Waited at multiple seq tags in a Repeat body). Conservative: data is in let_at_wait iff EVERY Wait of it is whole-array. Mixed-mode (some whole, some slice) data stays pre-init+`x = ...wait()`.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Filed cycle 212 (TASK-0341.03.02 codegen-warning surface) ===

## Problem

The multi-worker codegen emits, per worker, for each broadcast (whole-array, NOT slice-paste) data symbol:

  let mut x: Vec<i32> = vec![0; 8];
  ...
  x = w0_slot_8.wait();   // recv `x` from host

The initial `vec![0; 8]` is fully overwritten by the assignment, so Rust emits `warning: value assigned to `x` is never read; help: maybe it is overwritten before being read?`. Observed 4 times in the cycle-212 17-spmv/distributed × pthreads-sync emit (one per worker).

## Why it works correctly (despite the warning)

The output is bit-identical against reference.bin on every tier-1 backend — the runtime semantics are correct. The wasted allocation is `vec![0; N]` (a single Vec<i32> of length N=8 in 17-spmv) per worker, per broadcast data: bounded and small. Not a correctness defect.

## Root cause sketch

Compare slice-paste data (e.g. `val` in 17-spmv/distributed):

  let mut val: Vec<i32> = vec![0; 24];
  ...
  { let _tmp = w0_slot_4.wait(); val[0..6].copy_from_slice(&_tmp[0..6]); }  // recv slice

The slice-paste path keeps the init bytes for the OUT-OF-RANGE slots (val[6..24] stays 0 on w0). Those out-of-range slots are then READ by the cargo compiler so the init is live.

The broadcast path overwrites the WHOLE vec, so the init is dead. Two options at the codegen layer:
- (a) Emit `let x: Vec<i32> = w0_slot_8.wait();` directly (no preallocation; the assignment is also the declaration).
- (b) Emit `let mut x: Vec<i32> = Vec::with_capacity(8); x = w0_slot_8.wait();` — saves the zero-init but still has a dead alloc.

Option (a) is cleanest. It requires distinguishing whole-array recv from slice-paste recv at codegen time — the data is already in the codegen layer (the leading-axis filter at TASK-0301 emits empty bounds for whole-array, populated bounds for slice-paste).

## Why this is NOT urgent

- Cosmetic only — output is bit-identical, no runtime cost beyond a transient vec! that the optimizer likely elides at -O3.
- 4 warnings per multi-worker emit is below noise threshold; it does not fail clippy (the warnings are on the EMITTED code, not the workspace code).
- Memory project-negative-seam-and-backend-layout — the codegen output is what users see when they emit + cargo build a nucleus project, so this is user-facing cosmetic warning noise eventually worth removing.

## Acceptance criteria (when picked up)

1. The codegen distinguishes whole-array recv from slice-paste recv at emit time.
2. The emitted code uses `let x = slot.wait();` for whole-array recv (no preallocation).
3. The slice-paste path is unchanged.
4. Cargo build of emitted projects (across all 7 tier-1 backends) emits NO `unused_assignments` warnings on the broadcast-recv pattern.
5. e2e remains bit-identical (no behavioral change).

## Companion / linkage

- Surfaced by TASK-0341.03.02 cycle 212 (17-spmv/distributed); the pattern exists in EVERY multi-worker distributed schedule with a whole-array-broadcast data (07-matmul `b`, 16-jacobi `seed` if cleanly broadcast, 05-stencil's `img_in` under leading_axis_slice, etc.). The cosmetic-only nature means this is LOW priority but the audience for "emitted code looks professional" is real.
- Related: project-negative-seam-and-backend-layout (each backend's emitted main.rs surface).

=== Cycle 220 closure (orchestrator-direct) ===

AC#1+#2+#3+#5 LANDED via the let-at-wait optimization. AC#4 closed across all 7 tier-1 backends with the honest scope correction below.

LANDED CODE CHANGES:
- nucleus/backend-common/src/multi_worker_walker/wait.rs: new pub `is_whole_array_recv` (wraps wait_slice's classification); `render_wait_assign` gains a `let_at_wait: &BTreeSet<DataId>` parameter; emits `let {name} = {rhs};` (declare-and-assign) for whole-array recv on data in let_at_wait, falls back to `{name} = {rhs};` otherwise.
- nucleus/backend-common/src/multi_worker_walker/collect.rs: new pub `collect_let_at_wait_data` — descends into Event::Loop bodies, returns BTreeSet<DataId> where every Wait is whole-array AND data is not accumulate-fan-in AND not indexed-Fire-written.
- nucleus/backend-common/src/multi_worker_walker/ctx.rs: WalkerCtx gains `let_at_wait_data: &BTreeSet<DataId>` field + `empty_let_at_wait_set()` static helper.
- nucleus/backend-common/src/multi_worker_walker/event_walker.rs: threads `ctx.let_at_wait_data` to render_wait_assign.
- 5 walker-using backends (pthreads-sync, pthreads-async, openmp-rs, mp-tcp-event, mp-uds-event): each `collect_pre_init` now returns tuple (Vec, BTreeSet); each emit site destructures and passes &let_at_wait to WalkerCtx; `#[allow(clippy::type_complexity)]` on the tuple-return signature.
- 2 wrap-using backends (mp-tcp-bufsync, mp-tcp-poll): pass `WalkerCtx::empty_let_at_wait_set()` to direct render_wait_assign calls (their `{ let __buf = ...; {assign} }` wrap pattern would block-scope a `let {name} = ...;` incorrectly).
- 4 test files: WalkerCtx construction sites updated with empty_let_at_wait_set().

VERIFICATION:
- just check + just clippy + just test (1019/0/3 dev) + just test-release (1018/0/3 release, 1-test debug_assert delta per TASK-0291) + just check-textual-replace-on-codegen + check-include-str-coverage + check-narrative-doc-lie + check-mega-files: ALL CLEAN.
- just e2e: 280/246/0/34/0 unchanged (AC#5 bit-identical preserved).
- AC#4 empirical audit: 17-spmv/distributed × all 7 tier-1 backends emit 0 `unused_assignments` warnings on `cargo build` (verified post-fix). Spot-checks on 06-separable-filter/distributed × 3 backends, 07-matmul/distributed × 3, 07-matmul/distributed-2d × 3, 08-histogram/distributed × 3: all 0 warnings.

HONEST SCOPE CORRECTION (architect cycle-220 P1.2 fold-back):
The cycle-220 plan implied the unused_assignments warning was universal across all 7 tier-1 backends. Architect P1.2 empirically falsified this:
- 3 backends (pthreads-sync, pthreads-async, openmp-rs) lack `unused_assignments` in their per-worker `#[allow]` attribute → warning surfaced pre-cycle-220 → the cycle-220 fix is genuinely needed.
- 4 backends (mp-tcp-event, mp-uds-event, mp-tcp-bufsync, mp-tcp-poll) include `unused_assignments` in their `#[allow]` attribute → warning was suppressed pre-cycle-220 regardless of the cycle-220 fix.
The cycle-220 fix lands on 5 of the 7 (3 fix-needed + 2 mp-tcp-event/mp-uds-event redundantly cleaner-emit); 2 (mp-tcp-bufsync, mp-tcp-poll) skip the explicit fix. AC#4 closes for all 7 because the warning is in fact silenced everywhere (3 by the cycle-220 fix, 4 by the pre-existing `#[allow]`).

=== Cycle 220b architect P1.1+P1.2+P2.1+P3.4 pre-commit fold-back ===

Architect read-only review (P1.1 BLOCKING): the cycle-220 inline comments in bufsync/poll events.rs incorrectly claimed "the wrap-block reassignment naturally avoids the unused_assignments warning". This is `feedback-implementer-disclosure-mechanism-wrong` (20th firing of feedback-silent-sibling-defect class spanning into mechanism-wrong sub-class).

The REAL mechanism: bufsync (worker_program.rs:121-122) and poll (worker_program.rs:89-90) emit `#[allow(unused_mut, dead_code, unused_variables, unused_assignments, clippy::needless_late_init)]` on `fn main()`. The wrap shape itself does NOT protect from the warning — rustc fires it on nested-block reassignment when the outer `let mut` allow is absent (architect-verified by stripping the attribute and re-compiling).

Fold-back actions cycle 220b (in-thread pre-commit):
- P1.1: bufsync events.rs + poll events.rs inline comments rewritten to cite the per-main `#[allow(unused_assignments)]` mechanism + cross-reference the pre-existing rationale at bufsync worker_program.rs:114-118. The wrap-naturally-avoids claim is explicitly retracted.
- P1.2: this addendum + commit message honestly disclose the 3-of-7-vs-4-of-7 fix-needed split.
- P2.1: 11 of 14 test WalkerCtx construction sites had 12-space let_at_wait_data indent vs the surrounding 8-space accumulate_waits (sed-batch artifact from the cycle-220 test fixup). Re-aligned all 11 via per-file sed (blocked_rebind ×4, reuse_marker ×6, wait_assign_slice line 197 ×1). Architect counted 10; my recount gave 11 (architect's count was approximate; both within the same defect class).
- P3.4: is_whole_array_recv docstring's "Err on shape-error invariant violations" widened to "Err on shape-error or sidecar-lookup invariant violations" (wait_slice's Err arms span both classes).

Follow-ups filed cycle 220b:
- TASK-0354 (architect P2.2): unit-level tests for collect_let_at_wait_data + is_whole_array_recv (positive + negative cases pinning the classifier semantics).
- TASK-0355 (architect P3.1): unify is_whole_array_tile (collect.rs:260) + is_whole_array_recv (wait.rs:369) — both classify the same condition but with slightly different edge-case semantics.
- TASK-0356 (architect P3.2): scope-mismatch defensive test for let-at-wait inside Event::Loop body (no in-tree schedule triggers this today; theoretical risk of non-compiling emit when a Fire-input consumes Wait-data at outer scope).

NOT FOLDED:
- Architect P2.2 (no unit tests): filed as TASK-0354.
- Architect P3.1 (classifier unification): filed as TASK-0355.
- Architect P3.2 (scope-mismatch defensive): filed as TASK-0356.
- Architect P3.3 (plan file-path drift mention): superseded by this closure addendum which records landed reality.

FORWARD-CARRY LESSONS (for next phase3 cycle):
- The cycle-128 meta-rule fired AGAIN (20th firing of feedback-silent-sibling-defect class, narrowing to feedback-implementer-disclosure-mechanism-wrong sub-class): cycle 219 was a doc-lie sweep cycle; cycle 220's pre-commit closure narrative contained a mechanism-wrong doc-lie about why bufsync/poll don't show the warning. The architect's empirical mechanism-isolation discipline (strip the `#[allow]` and re-compile) caught it pre-commit. The same discipline should apply to EVERY "this mechanism is why X works" claim in implementer/orchestrator notes: probe the alternative-mechanism hypothesis empirically.
- Sed-batch on test files for new field addition produced systematic indent drift (11 of 14 sites). Per memory feedback-sed-batch-tracker-md-substitution: even on test code, sed-batch carries silent-sibling-on-style risk. Per-string atomic Edit calls would have avoided this; if sed is used, post-edit `diff` or grep-witness on indentation must catch the drift before commit.

AC TICK MAP:
- AC#1: ticked cycle 220 (whole-array recv distinguished from slice-paste at codegen time via collect_let_at_wait_data + wait_slice classification).
- AC#2: ticked cycle 220 (emitted code uses `let {name} = {rhs};` for whole-array recv in 5 walker-using backends).
- AC#3: ticked cycle 220 (slice-paste path unchanged — verified by 06-separable-filter/distributed e2e + spot-checks).
- AC#4: ticked cycle 220b (0 unused_assignments warnings across all 7 tier-1 backends on distributed cells; 3 backends fixed by cycle-220, 4 backends pre-suppressed by `#[allow(unused_assignments)]`).
- AC#5: ticked cycle 220 (e2e 280/246/0/34/0 unchanged).

Status: Done.
<!-- SECTION:NOTES:END -->
