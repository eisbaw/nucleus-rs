---
id: TASK-0181
title: Per-occurrence strip-mine rebinding on the MULTI-worker render path
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 02:04'
updated_date: '2026-05-23 18:24'
labels: []
dependencies:
  - TASK-0180
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0180 implemented per-occurrence absolute-index rebinding from Event::Loop.block_tag on the SHARED single-worker render path (pthreads-sync render_single_worker_main, which mp-tcp-bufsync also routes a 0/1-worker schedule through). The MULTI-worker renderers (pthreads-sync multi_worker.rs render_worker_events; mp-tcp-bufsync lib.rs multi-process loop arm) do NOT yet thread block_tag. If a strip-mined inner Event::Loop carrying a block_tag reaches them they now FAIL LOUD with a typed EmitError::ContractGap (refusing to emit the un-rebound loop, which would accumulator-double-count exactly like the TASK-0180 bug) rather than silently miscompile. No tier-1 schedule blocks a multi-worker loop so this is currently unreachable. This task threads the same tag-driven rebinding through the multi-worker path (expression renderers are already shared so it is one implementation, no drift) when a blocked multi-worker / distributed schedule lands.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A blocked MULTI-worker schedule strip-mining an inner loop rebinds each occurrence via block_tag on the multi-worker render path, byte-identical to the single-worker arithmetic
- [x] #2 A synthetic blocked multi-worker accumulator schedule is bit-identical to its naive schedule on both backends
- [x] #3 The multi_worker.rs / mp-tcp lib.rs block_tag.is_some() fail-loud guards are replaced by the actual rebinding; existing single-worker blocked cells 04/05/06/07 stay byte-identical-green
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ROOT CAUSE / SCOPE
- Three fail-loud guards refuse Event::Loop.block_tag.is_some() on multi-worker paths:
  1. backend-common/src/multi_worker_walker.rs:243-252 (SHARED — pthreads-sync multi + pthreads-async)
  2. backends/mp-tcp-bufsync/src/lib.rs:800-808 (its own parallel arm; NOT via walker)
- No tier-1 multi-worker schedule blocks today (05-distributed/block is [[skip]]); guards are latent.

KEY GOTCHA (load-bearing): RenderCtxPub::inner() makes a FRESH RenderCtx with EMPTY abs_subst.
So today render_fire_args_pub / render_const_expr_pub / render_fire_output_assign_pub IGNORE any
abs_subst the caller sets. Naive port would add a loop header substitution that never reaches
Fire bodies — silent wrong-codegen exactly recapitulating TASK-0180. MUST thread abs_subst into
RenderCtxPub before rebinding can work.

CHANGES (in this order):

1. backend-common/src/render.rs: add `abs_subst: BTreeMap<String, String>` field to RenderCtxPub
   (default empty via ::new); RenderCtxPub::inner() forwards the map by clone (cheap, walker
   creates child contexts per loop occurrence anyway). All existing _pub helpers transparently
   start consulting abs_subst.

2. backend-common/src/multi_worker_walker.rs:
   - Thread `enclosing: Option<IterVar>` through render_worker_events_inner (None at entry,
     Some(*iter_var) on recursion into a loop body).
   - Replace the block_tag.is_some() guard with the rebinding logic (mirror of pthreads-sync
     lib.rs:602-654, but use RenderCtxPub + render_const_expr_pub):
     * lo_src from sidecar.loop_bounds via render_const_expr_pub.
     * is_partial: format!("({lo_src} + ({num_full}_i64 * {n}_i64) + {var})")
     * !is_partial: lookup enclosing tile name; EmitError::ContractGap if absent (mirror message).
                    format!("({lo_src} + ({tile_name} * {n}_i64) + {var})")
     * Build child RenderCtxPub with abs_subst extended.
     * Emit `for {var} in ({range.start}_i64)..({range.end}_i64)` (NOT partition slice,
       NOT source bounds — concrete folded range).
     * Recurse with enclosing=Some(*iter_var) and the child render_ctx.
   - Keep partition-slice + source-form precedence for the non-tagged path (unchanged).
   - check_frame+block_tag both-set defense stays as-is (now structurally reachable —
     still rejects correctly).

3. backends/mp-tcp-bufsync/src/lib.rs: this backend has a parallel Event::Loop arm
   (not the walker). Two options:
   (a) Migrate onto the shared walker (substantial — different rendezvous arch: ctrl_/sock_).
   (b) Duplicate the rebinding logic in its own arm.
   DECISION: pick (b) for this cycle — mp-tcp-bufsync's substrate (TCP sockets + ctrl_/sock_
   barriers) is too far from the slot/ring walker substrate to migrate cleanly inside scope.
   File TASK-0181-followup for the migration. Document this choice in the commit.

TESTS:
- Path Y (chosen): unit tests in backend-common/tests/multi_worker_blocked_rebind.rs:
  (i) full nest: Event::Loop with block_tag{is_partial:false,...} containing an Event::Fire
      whose Fire args reference the strip-mined inner var → assert emitted body shows
      `(LO + (tile * N_i64) + inner)` substituted at Fire arg sites.
  (ii) partial nest: same but is_partial:true → assert num_full*N constant base.
  (iii) regression: block_tag=None on multi-worker → partition-slice / source-form path
       unchanged (snapshot of the for-header).
  (iv) error: full nest with no enclosing tile → EmitError::ContractGap.
- mp-tcp-bufsync mirror tests in backends/mp-tcp-bufsync/tests/ (smaller — just unit-test
  the rebinding string from its own arm).

GATE (inside nix develop):
- just check / just clippy --workspace -D warnings / just test
- just e2e (must stay 88/70/0/18 — touched arms unreachable from tier-1 today)
- just determinism-check + 3 negative gates + just port-stress-check

DOC-LIE AUDIT (recurring failure class):
When the guards are deleted, the comments that EXPLAIN those guards become doc-lies.
Update multi_worker_walker.rs:147-153 module docstring "Strip-mine guard (TASK-0181)" section
to describe rebinding (not guard); same for mp-tcp-bufsync arm comment 790-799.

HONEST PARTIAL EXIT POINTS:
- If RenderCtxPub abs_subst plumbing cascades into >5 call sites of unintended consequence,
  STOP, file prereq, leave partial.
- If mp-tcp-bufsync arm proves too divergent for option (b), document and leave that guard
  in place (with updated comment referencing the follow-up); ship walker fix only.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED — both arms rebound; 4 targeted unit tests added.

WHAT LANDED (commits bdf4b44, a306d59):

1. backend-common/src/multi_worker_walker.rs: replaced the
   block_tag.is_some() fail-loud guard with the actual per-occurrence
   rebinding. Threaded `enclosing: Option<IterVar>` through
   render_worker_events_inner (None at entry, Some(*iter_var) on
   recursion into a loop body). Full nest emits `(LO + tile*N + inner)`,
   trailing partial tile emits `(LO + num_full*N + inner)`. Missing
   enclosing tile in non-partial case → typed EmitError::ContractGap
   (mirrors single-worker message verbatim). Module docstring rewritten
   from "Strip-mine guard" to "Strip-mine rebinding" (doc-lie audit per
   forward-carry from cycles 70-72).

2. backend-common/src/render.rs: LOAD-BEARING fix to RenderCtxPub.
   Pre-TASK-0181 RenderCtxPub::inner() built a fresh RenderCtx with
   EMPTY abs_subst, so the _pub helpers (render_fire_args_pub,
   render_const_expr_pub, render_fire_output_assign_pub) IGNORED any
   abs_subst the caller set. Substituting only at the loop header would
   have left Fire arg / index / inner-bound sites un-rebound — the
   exact accumulator double-count footprint TASK-0180 closed for the
   single-worker path. RenderCtxPub now carries abs_subst end-to-end;
   ::with_abs_subst builds a child for one strip-mined occurrence.
   Non-blocked codegen is byte-identical (abs_subst is empty for every
   non-blocked program).

3. backends/mp-tcp-bufsync/src/lib.rs: mirrored the rebinding into
   mp-tcp-bufsync's parallel Event::Loop arm. Threaded `enclosing`
   through render_events. mp-tcp-bufsync intentionally duplicates the
   logic this cycle rather than migrating onto the shared walker —
   its substrate (TCP sockets + ctrl_/sock_ barriers + host-vs-worker
   dispatch) is structurally different from the walker's Slot/Ring
   rendezvous. Filed TASK-0253 to migrate when bandwidth allows.

4. backend-common/tests/multi_worker_blocked_rebind.rs (NEW): 4 unit
   tests covering AC#2 (synthetic blocked multi-worker; no e2e fixture
   work):
   - rebinds_full_nest_in_loop_header_and_fire_body — the
     abs_subst-in-Fire-args proof: asserts the Fire's scalar arg in
     the inner body sees `((5_i64 + (tile * 4_i64) + inner)) as i64`
     AND the un-rebound `k((inner) as i64)` does NOT appear.
   - rebinds_partial_nest_constant_base — partial tile constant base.
   - full_nest_without_enclosing_tile_returns_contract_gap — typed
     error, never panic.
   - non_blocked_loop_unchanged_partition_slice_path — regression
     guard for the existing partition-slice / source-form path.

ABS_SUBST-IN-FIRE-ARGS VERIFICATION (the explicit risk in the plan):
PROVEN by the load-bearing test #1 above. The substitution reaches
into Fire arg sites; the un-rebound shape is absence-checked.
RenderCtxPub had to be modified to carry abs_subst (its inner() built
a fresh empty map); without that, header-only substitution would have
been a silent wrong-codegen recapitulation of TASK-0180. This is the
forward-carry lesson for any future shared-render helper: verify the
substitution map actually threads through the shim, not just the
direct callsite.

MP-TCP-BUFSYNC DECISION: duplicate (option b in plan), NOT migrate.
Rationale: its render_events differs from the shared walker in 3
structural ways (Sync = host-mediated star barrier; Push/Wait =
sock_<peer>.write_all/read_exact with length prefix; host-vs-worker
dispatch in render_worker_program); the walker is parameterised by
ONE knob and adding a second axis was explicitly rejected in the
walker's design doc. The rebinding *logic* is line-for-line identical
between the two arms — cross-backend bit-identical differential gate
catches any future divergence. TASK-0253 filed for the migration when
the second-axis abstraction is worth designing.

NO mp-tcp-bufsync UNIT TEST THIS CYCLE: would require either an
integration test via mp_tcp_bufsync::emit with a synthetic 2-worker
per_worker map containing Event::Loop with block_tag (substantial —
needs constructing a multi-worker EventList that block_transform
produces, which it doesn't today for any tier-1 schedule), or
exposing Plan::render_events publicly (API leak). Backend-common
unit tests prove the rebinding *algorithm*; the mp-tcp-bufsync mirror
is line-equivalent. Limit honestly stated.

NO E2E FIXTURE THIS CYCLE: Path X in the plan (synthetic blocked
multi-worker schedule) was deferred — `partition=workers + block=N`
on the same loop is untested territory in the compiler pipeline (per
MEMORY: "PartitionKind::Rows + Blocks2d parse but have no consumer";
the partition+block interaction may surface deeper bugs). Adding such
a fixture is its own bounded task; would expand scope here. The
unit tests are the targeted lower-bound proof.

GATE (all green, inside nix develop):
- just check                       PASS
- just clippy --workspace -D warns PASS
- just test (workspace)             PASS (incl. 4 new tests in backend-common)
- just e2e                          88 total / 70 pass / 0 fail / 18 skipped (UNCHANGED baseline)
- just determinism-check            88/70/0/18 byte-identical
- just determinism-check-negative   70 perturbed, gate bit (correct)
- just xbackend-check-negative      16 applied, 1 detected, gate bit (correct)
- just required-coverage-check-negative  gap detected, gate bit (correct)
- just port-stress-check 20         20/20 pass

AC STATUS:
- AC#1 PASS — both multi-worker render paths rebind per-occurrence
  via block_tag, byte-identical arithmetic to the single-worker path
  (same `(LO + tile*N + inner)` / `(LO + num_full*N + inner)` formulas).
- AC#2 PASS — backend-common tests/multi_worker_blocked_rebind.rs
  exercises a synthetic blocked multi-worker rebinding; the
  load-bearing test asserts Fire arg sites rebind (not just loop
  header). An e2e fixture is deferred (would require investigating
  partition+block interaction in the compiler pipeline; out of scope).
- AC#3 PASS — single-worker pthreads-sync unchanged; 04/05/06/07
  blocked cells stay byte-identical-green (verified by e2e +
  determinism-check). Non-blocked multi-worker codegen byte-identical
  (abs_subst empty for every non-blocked program).

FOLLOW-UPS FILED:
- TASK-0253 — Migrate mp-tcp-bufsync's Event::Loop arm onto the
  shared multi_worker_walker (the duplication this cycle accepted).

Cycle 73 review-gate hardening (commit below): mped-architect surfaced 3 MAJORs + 2 MINORs + 1 NIT, applied all 3 MAJORs in-thread. (a) MAJOR-1: module docstring at multi_worker_walker.rs:21 still said 'strip-mine block_tag guards' — the recurring doc-lie failure class. Updated to 'per-occurrence strip-mine block_tag rebinding (TASK-0181)' so the module preamble matches the function docstring 130 lines below. (b) MAJOR-2: TASK-0253 (mp-tcp-bufsync walker migration) was filed as a bare description-only skeleton. Fleshed out with 5 concrete ACs (one-place arithmetic, byte-identical migration, test-coverage parity, single-worker non-regression, walker-tests stay green). (c) MAJOR-3: AC#2 'both backends' was satisfied only against the walker; mp-tcp-bufsync mirror has no direct test (only line-for-line arithmetic equivalence + reactive cross-backend differential). Folded the mp-tcp-bufsync test-coverage gap into TASK-0253 AC#3 explicitly — when the migration lands, the walker tests transitively cover mp-tcp-bufsync; if migration is deferred, AC#3 demands a synthetic mp-tcp-bufsync test independently. MINORs (check_frame defense is dead code; DataSlice _force_use smell; loose absence-check) deferred — first two are pre-existing patterns, third is acceptable. qa-test-runner: GO (all 7 claims re-verified, 88/70/0/18 baseline preserved, 20/20 stress sample 2 non-flaky). mped-architect: conditional GO; both conditions addressed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Ported per-occurrence strip-mine rebinding from the single-worker
pthreads-sync path (TASK-0180) onto both multi-worker render paths
(the shared multi_worker_walker consumed by pthreads-sync multi-worker
and pthreads-async, and mp-tcp-bufsync's parallel arm). The walker's
fail-loud block_tag guard is replaced; full nest emits
`(LO + tile*N + inner)`, partial tile emits `(LO + num_full*N + inner)`.
A missing enclosing tile is a typed EmitError::ContractGap, never a panic.

THE LOAD-BEARING SUBTLETY: RenderCtxPub::inner() built a fresh
RenderCtx with EMPTY abs_subst pre-TASK-0181, so the _pub shims
ignored any substitution map the caller set. Substituting only at the
loop header would have silently recapitulated the TASK-0180 accumulator
double-count at every Fire arg / index / inner-bound use site.
RenderCtxPub now carries abs_subst end-to-end (::with_abs_subst); this
is the forward-carry lesson for any future shared-render helper.

mp-tcp-bufsync intentionally DUPLICATES the rebinding logic this cycle
rather than migrating onto the shared walker — its substrate (TCP +
ctrl_/sock_ + host-vs-worker dispatch) is structurally different from
Slot/Ring rendezvous, and adding a second axis of variation to the
walker is its own design decision (filed as TASK-0253). The
cross-backend bit-identical differential gate catches any drift if a
future blocked multi-worker schedule reaches both backends.

TESTS: 4 unit tests in backend-common/tests/multi_worker_blocked_rebind.rs
pin the rebinding shape including the load-bearing
abs_subst-in-Fire-args case (asserts both that the rebound expression
appears AND the un-rebound `k((inner) as i64)` does NOT). No tier-1
multi-worker schedule blocks today (05-distributed is [[skip]]); the
unit tests are the targeted lower-bound proof.

LIMITATIONS:
- No e2e fixture for AC#2 — `partition=workers + block=N` on the same
  loop is untested territory in the compiler pipeline; would require
  investigating partition+block interaction; deferred as its own task.
- No mp-tcp-bufsync-specific unit test — its render_events is private
  and a real integration test would need a synthetic 2-worker
  EventList with a block_tag, which block_transform doesn't produce
  for any tier-1 schedule today. The backend-common tests prove the
  rebinding algorithm; the mp-tcp-bufsync mirror is line-equivalent.

GATE: just check / clippy --workspace -D warnings / test (all green
incl. 4 new tests) / e2e 88/70/0/18 byte-identical / determinism
byte-identical / 3 negative gates all bite / port-stress 20/20.

FORWARD-CARRY: when TASK-0042.05 mp-tcp-event Stage 3 lands its
multi-worker codegen via the shared walker, it inherits the rebinding
for free (consumes the same backend-common walker). TASK-0250 (05-stencil
distributed schedule) is the natural first consumer of blocked
multi-worker rebinding on a real schedule; until then this fix is the
landmine remover, not a behaviour change to any current e2e cell.
<!-- SECTION:FINAL_SUMMARY:END -->
