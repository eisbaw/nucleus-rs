---
id: TASK-0240
title: >-
  Multi-worker pthreads-async check_frame emit-string pinning tests (TASK-0228
  AC#5 test gap)
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 07:35'
updated_date: '2026-05-22 09:11'
labels:
  - M4
  - backend
  - test-coverage
dependencies:
  - TASK-0228
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0228 Wave B-2 (cycle 26) wired the multi-worker check_frame substrate (file-scope AtomicU64 statics via emit_count_static, host-thread Drop guards via emit_count_guard_local, per-worker Log/Count branches via emit_log_branch / emit_count_branch). The helpers are CALLED from Plan::emit, but no in-tree multi-worker pthreads-async fixture carries a check_loop directive, so the emit-string shape is UNTESTED for this backend.\n\nMirrors the structure that TASK-0236 set up for pthreads-sync + mp-tcp-bufsync's multi-worker check_frame: a synthetic 2-worker per_worker fixture with one Event::Loop carrying check_frame = Some(CheckFrame{loop_var, latency_max_ns, on_violation: ViolationKind::{Panic,Log,Count}}) for each of the three violation kinds, then string-pin the emitted main.rs against the expected (a) file-scope AtomicU64 static + reporter struct (Count only), (b) per-Count-loop Drop guard local in fn main, (c) per-iteration measurement + on-violation branch with the right idents.\n\nAcceptance:\n- nucleus/backends/pthreads-async/tests/check_frame_emit.rs created (mirror pthreads-sync's check_frame_codegen.rs + mp-tcp-bufsync's check_frame_emit.rs).\n- Three tests pin the Panic/Log/Count multi-worker emit shape.\n- The third-backend Final Summary on TASK-0222 (extract check_frame templates) can close once this lands: 'All three tier-1 backends consume the shared helpers AND test-pin their emit shape; drift is now structurally + test-detected for all three.'
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 29 plan (2026-05-22)

Mirror TASK-0236's pthreads-sync + mp-tcp-bufsync precedents for multi-worker check_frame emit-string pins, adapted to pthreads-async.

Approach: real (not fully synthetic) per_worker fixture — use a multi-worker schedule with partition=workers + transfer x : sync (the same shape mp-tcp-bufsync uses for its 3 multi-worker check_frame tests) and route it through test_common::lower_for_test with apply_partition_workers=true + inject_check_frames=true. This produces a real Event::Loop with check_frame populated on each compute worker; the host has no check_frame (partition=workers projects the loop onto compute workers only). Same fixture shape as pthreads-sync's CHECK_ALGO_SRC + 3 partition variants and mp-tcp-bufsync's MULTI_ALGO_SRC + 3 partition variants.

New file: nucleus/backends/pthreads-async/tests/check_frame_emit.rs

Tests:
1. multi_worker_panic_emit_pins_per_thread_panic_template — 2 Panic sites (one per compute worker; host has none).
2. multi_worker_log_emit_pins_per_thread_eprintln_template — 2 eprintln sites via emit_log_branch.
3. multi_worker_count_emit_pins_shared_static_guard_and_fetch_add — 1 file-scope static + 1 guard local in fn main + 2 fetch_add sites + 1 reporter struct (shared-memory model: one static across N threads, like pthreads-sync's multi_worker; UNLIKE mp-tcp which emits per-process).
4. (Optional, if room) multi_worker_count_dedups_static_across_workers — 1 static emitted even though both compute workers' Event::Loop carry the same check_frame.

Scratch-dir race avoidance (TASK-0241 forward-carry): each test uses a UNIQUE scratch dir keyed off the test function name (mirroring multi_worker_codegen.rs's pattern).

Mp-tcp-bufsync precedent is mostly applicable but the EXPECTED COUNTS differ for Count: mp-tcp emits per-process (2 statics across 2 separate worker bins), pthreads-async emits ONE static at file scope shared across N threads (same as pthreads-sync). The expected emit shape mirrors pthreads-sync's multi_worker.rs Log + Count tests.

Verification:
- Run new pthreads-async tests in isolation first.
- 5x stress to confirm no scratch-dir race.
- just test + just clippy + just e2e gate.

AC closure:
- TASK-0240: all 3 tests land.
- TASK-0227: AC#1 + AC#3 close (AC#2 already structurally met via shared-helper imports; AC#4 closes via gate).
- TASK-0228 AC#5: re-tickable (cycle 26 architect un-ticked over AC-gaming concerns; new pin tests make the structural claim test-backed).

## Cycle 29 progress (2026-05-22) — IMPLEMENTATION LANDED

File created: `nucleus/backends/pthreads-async/tests/check_frame_emit.rs` (369 lines).

3 tests landed (all green):
1. `multi_worker_panic_emit_pins_per_thread_panic_template` — pins 2 Instant::now() sites + 2 panic-message sites with threshold literal `5000000_u128` and the loop_var name `n` in the message; negative checks that Log eprintln + Count substrate are absent.
2. `multi_worker_log_emit_pins_per_thread_eprintln_template` — pins 2 eprintln template sites (`warning: check loop \`n\` violated latency_max=5000000 ns: ...`) via the shared `emit_log_branch` helper; negative checks Panic/Count absent.
3. `multi_worker_count_emit_pins_shared_static_guard_and_fetch_add` — pins (a) ONE file-scope `static NUC_CHECK_COUNT_n` (deduped by sanitized ident — both compute workers share it under partition=workers); (b) ONE `NucCheckCountReporter` struct + Drop impl; (c) ONE host-thread guard local in fn main with loop_var="n" + threshold_ns=5000000; (d) EXACTLY 2 fetch_add branch sites (per compute worker); negative checks Panic/Log absent.

Shared-memory model: tests assert ONE static + ONE guard + N fetch_add (vs mp-tcp-bufsync's per-process pattern of N statics + N guards + N fetch_add). Mirrors pthreads-sync's multi_worker_check_loop_count_emit_* expectations exactly — confirms the cross-backend shared-helper transparency claim from cycle 22 / TASK-0222.

Fixture: real lower-link-inject pipeline via `test_common::lower_for_test` with `apply_partition_workers=true` + `inject_check_frames=true`. Schedule has 2 compute workers + partition=workers + transfer:sync (same shape mp-tcp-bufsync uses in its 3 multi-worker check_frame tests). Each test uses a UNIQUE scratch dir keyed off the test function name (TASK-0241 forward-carry).

Gate (cycle 29):
- `nix develop -c just test`: 0 FAILED suites (all OK).
- `nix develop -c just clippy`: clean (one doc_lazy_continuation lint fixed during write).
- `nix develop -c just e2e`: 54 / 46 / 0 / 8 baseline preserved.
- 5x stress `cargo test -p pthreads-async`: 0/5 FAILED. Scratch-dir race avoided per-function unique dir naming.

Architect un-tick of TASK-0228 AC#5 (cycle 26 AC-gaming finding) is now structurally + test-backed reverseable. Cross-backend coverage matrix for check_frame multi-worker emit:
  pthreads-sync  : Panic + Log + Count pins (cycles 16, 23)
  mp-tcp-bufsync : Panic + Log + Count pins (cycle 23)
  pthreads-async : Panic + Log + Count pins (THIS CYCLE)
All three tier-1 backends now consume the shared helpers AND test-pin their emit shape — drift is structurally + test-detected for all three. TASK-0222 third-backend Final Summary unlocked.

No follow-ups filed: tests pin the EXPECTED emit-string shape produced by the shared helpers; no codegen defects surfaced. The Panic INLINE template is byte-identical across pthreads-sync and pthreads-async (both inline the format in render_worker_events); Log/Count route through shared helpers so byte-identical by construction.

Status: ready for review + commit. Not marking Done; orchestrator handles close + commit.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 29 (2026-05-22) — closed. Added nucleus/backends/pthreads-async/tests/check_frame_emit.rs (3 pin tests for Panic/Log/Count multi-worker emission). Shared-memory model: 1 static / 1 guard / N fetch_add — diverges intentionally from mp-tcp-bufsync's per-process model (N statics / N guards / N fetch_add) and matches pthreads-sync precedent (multi_worker.rs:894). Pin tightness: exact assert_eq counts (static_count==1, guard_count==1, fetch_add_count==2 for 2-worker fixture; not >= slack). Per-test unique scratch dirs (TASK-0241 forward-carry). TASK-0228 AC#5 re-ticked. Closes TASK-0222's third-backend coverage clause.
<!-- SECTION:FINAL_SUMMARY:END -->
