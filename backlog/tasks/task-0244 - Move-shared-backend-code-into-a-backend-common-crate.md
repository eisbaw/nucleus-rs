---
id: TASK-0244
title: Move shared backend code into a backend-common crate
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 09:42'
updated_date: '2026-05-22 11:40'
labels:
  - tech-debt
  - architecture
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle 31 (TASK-0239) lifted the shared multi-worker event-walker into pthreads-sync/src/multi_worker_walker.rs (634 LoC, pub from pthreads-sync). pthreads-async now imports via pthreads_sync::multi_worker_walker::*. This is a deliberate trade-off — pthreads-async already depends on pthreads-sync for TASK-0222 helpers and TASK-0238 NameTables — but it leaks pthreads-sync's module structure and creates a backwards-looking dependency arrow (async -> sync) that is not semantically real (they are siblings, neither is the parent).

Same architectural smell as TASK-0238 (NameTables that semantically belonged in compiler, not pthreads-sync), now resolved by moving it. The same move should apply here.

Proper home: a backend-common (or pthreads-common) crate carrying:
- multi_worker_walker (the shared event-walker)
- RenderCtxPub + render_*_pub helpers (already pub from pthreads-sync, same arrow problem)
- The shared check_frame template helpers (emit_count_static, emit_count_guard_local, emit_log_branch, emit_count_branch, collect_count_check_frames, sanitize_loop_var, emit_count_reporter_struct, CountCheckLoop)
- rust_type_of, render_array_init_for, rust_scalar_type_pub

Then pthreads-sync, pthreads-async, mp-tcp-bufsync all depend on backend-common (no inter-backend dependencies).

Deferred from TASK-0239 — getting de-dup landed first was the priority; the crate move is bounded but mechanical. A future M5+ backend (TASK-0042.02 mp-tcp-event) would be the natural forcing function, but doing it preemptively keeps architectural clarity.

Acceptance:
- New crate nucleus/backend-common/ exists with the listed exports.
- pthreads-sync, pthreads-async, mp-tcp-bufsync depend on backend-common (no inter-backend deps).
- e2e tally + cross-backend bit-identical invariants unchanged.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
cycle 37 (TASK-0244 — backend-common crate)

DESIGN: option B (full move). backend-common owns the entire shared
codegen surface; pthreads-sync, pthreads-async, mp-tcp-bufsync all
consume it. The ONLY surviving inter-backend arrow is pthreads-async +
mp-tcp-bufsync depending on pthreads-sync for render_single_worker_main
+ render_cargo_toml + render_run_sh — these are pthreads-sync's
straight-line single-worker emitter + project skeleton, which the two
other single-binary backends genuinely DELEGATE to (one binary, byte-
identical artefact). That arrow is semantic, not a shared-code leak.

NEW CRATE STRUCTURE: nucleus/backend-common/
  Cargo.toml                       — deps: compiler (only)
  src/lib.rs                       — pub uses (57 LoC)
  src/check_frame.rs               — CountCheckLoop, sanitize_loop_var,
                                     collect_count_check_frames,
                                     emit_count_reporter_struct,
                                     emit_count_static, emit_count_guard_local,
                                     emit_log_branch, emit_count_branch
                                     (217 LoC)
  src/render.rs                    — RenderCtx (with abs_subst, pub fields),
                                     RenderCtxPub (thin wrapper), data_name,
                                     render_fire_output_assign,
                                     render_fire_args, render_fire_arg,
                                     SliceForm + classify_data_slice,
                                     render_flat_index, render_int_expr,
                                     render_loop_bounds, render_const_expr,
                                     bin_op_str, rust_scalar_type,
                                     rust_scalar_zero, rust_type_of,
                                     render_array_init_for,
                                     rust_scalar_type_pub, the *_pub shims,
                                     EmitError (CANONICAL DEFINITION),
                                     write_file (740 LoC)
  src/multi_worker_walker.rs       — moved verbatim from pthreads-sync
                                     (640 LoC, comment header updated)

DIFF STATS:
  - LoC moved OUT of pthreads-sync/src/lib.rs:    832 lines  (1716 -> 884)
  - LoC moved OUT (multi_worker_walker.rs file):  639 lines  (whole-file delete)
  - LoC added to backend-common:                  1654 lines
  - mp-tcp-bufsync:  +11 LoC (import comments only; pthreads_sync::*
                     references rewritten to backend_common::*; runtime
                     dep on pthreads-sync KEPT because it consumes
                     render_single_worker_main, which stays in
                     pthreads-sync per the spec)
  - pthreads-async:  +8 LoC (import comments; pthreads-sync stays as
                     runtime dep for the same render_single_worker_main
                     reason)

CARGO.TOML CHANGES:
  + backend-common workspace member added
  + backend-common added as runtime dep of pthreads-sync,
    pthreads-async, mp-tcp-bufsync
  - mp-tcp-bufsync's pthreads-sync runtime dep WAS NOT dropped: it
    still calls render_single_worker_main from pthreads-sync.
    Spec said 'verify by cargo check' — verified: cargo check FAILS
    without the dep (the single-worker delegation arm). The
    dependency stays, but its REASON is now semantic delegation
    only (not shared codegen — that's all in backend-common now).

CROSS-BACKEND BYTE-IDENTICAL VERIFICATION:
  Snapshotted 6 cells (27 emitted files total: 4-5 files each cell)
  pre-refactor vs post-refactor, both via NUC_E2E_FORCE_SHARED_RUN_ID
  + a temporary NUC_E2E_KEEP_SCRATCH escape hatch (reverted post-
  verification). DIFF COUNT: 0. Every emitted file is byte-identical.
  Cells covered:
    01-elementwise-add/naive  × {sync, async}
    02-split-add/split        × {sync, async, mp-tcp-bufsync}
    13-cnn-inference/pipeline_parallel × pthreads-async

GATES:
  - nix develop -c just test:                       PASS (0 failures, 30+ suites)
  - nix develop -c just clippy:                     CLEAN (no warnings)
  - nix develop -c just e2e:                        66 total / 55 pass / 0 fail / 11 skipped (unchanged from baseline)
  - nix develop -c just determinism-check-negative: OK (55 cells perturbed)
  - nix develop -c just xbackend-check-negative:    OK (1 cell detected)

FOLLOW-UPS:
  None filed. The one remaining inter-backend arrow (pthreads-async +
  mp-tcp-bufsync → pthreads-sync for render_single_worker_main /
  render_cargo_toml / render_run_sh) is intentionally retained per the
  cycle-37 spec — it is a semantic delegation, not a shared-code
  leakage. If a future M5+ backend needs its own single-worker
  artefact shape (e.g. multi-process from line 1, no degenerate
  single-binary case) the right move is to lift the project-skeleton
  + straight-line emitter into backend-common too; until then the
  arrow stays where it best matches the code's actual ownership.

READY FOR REVIEW + COMMIT
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 37 (2026-05-22) — closed. Option B (full move) executed. New crate nucleus/backend-common/ (4 files, +1654 LoC) carries: multi_worker_walker (640 LoC, moved verbatim from cycle-31), check_frame helpers (217 LoC: CountCheckLoop, sanitize_loop_var, collect_count_check_frames, emit_count_reporter_struct, 4 emit_count_*/emit_log_branch helpers), render helpers (740 LoC: RenderCtx + RenderCtxPub + render_fire_args/output_assign/flat_index/int_expr/const_expr/loop_bounds + rust_type_of/scalar_type/array_init_for + canonical EmitError + write_file + all _pub shims), lib.rs (57 LoC: re-exports).

pthreads-sync/src/lib.rs: 1716 -> 884 LoC (-832); multi_worker_walker.rs deleted (-639). Total pthreads-sync net -1470 LoC.

All 3 backends now depend on backend-common for shared codegen. pthreads-async + mp-tcp-bufsync still depend on pthreads-sync for render_single_worker_main + render_cargo_toml + render_run_sh — semantic delegation (the single-binary backends share their straight-line emitter + Cargo.toml + run.sh; pthreads-sync owns them because it was the first single-binary backend). TASK-0246 filed (LOW) to optionally move render_cargo_toml + render_run_sh into backend-common::project_skeleton::single_binary in a future cycle — the architecture would then be: ONLY render_single_worker_main remains as the genuinely pthreads-sync-owned semantic delegation arrow.

backend-common is dependency-graph clean: depends ONLY on . No cycles. Architect-verified.

EmitError: now CANONICAL in backend-common::render. pthreads-sync re-exports it; the other two backends consume via backend-common's facade. Single source of truth.

Cross-backend byte-identical INVARIANT preserved across the refactor (the headline thesis check). Verified TWICE:
1. Implementer pre/post snapshot of 27 emitted files across 4 cells (01/02/13 × {sync, async, mp-tcp}): zero diff.
2. Architect independent rebuild+run of 13-cnn-inference/pipeline_parallel × pthreads-async: sha256 d893337208d7b46923581ecdea8e326e07e8c7e1204a13d867807d6795f7b861 matches reference.bin EXACTLY (same sha256 cycle 26 manually verified pre-refactor).

Gate (cycle 37): e2e 66/55/0/11 STABLE across 4 runs; just test 0 FAILED; just clippy clean; NUC_NONDET_PERTURBED_CELLS=55; NUC_XBACKEND_CORRUPTED_DETECTED=1 / APPLIED=16.

Review-gate (parallel read-only): both GO. Architect noted 2 LOWs: (a) EmitError import asymmetry between backends (cosmetic, single canonical source verified); (b) un-filed follow-up for render_cargo_toml + render_run_sh — FILED as TASK-0246 before commit.
<!-- SECTION:FINAL_SUMMARY:END -->
