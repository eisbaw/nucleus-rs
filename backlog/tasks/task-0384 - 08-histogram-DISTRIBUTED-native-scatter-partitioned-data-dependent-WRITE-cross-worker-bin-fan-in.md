---
id: TASK-0384
title: >-
  08-histogram DISTRIBUTED native scatter (partitioned data-dependent WRITE +
  cross-worker bin fan-in)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-31 05:03'
updated_date: '2026-05-31 16:42'
labels:
  - compiler
  - scatter
  - histogram
  - distributed
  - broaden
dependencies:
  - TASK-0376
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BROADEN follow-up to TASK-0376 (which landed the SINGLE-WORKER native scatter histogram[input[i]] <-- inc(histogram[input[i]]), 7 tier-1 backends bit-identical, e2e 329->336). The distributed step partitions `input` across workers, each worker scatters into a LOCAL partial histogram, then a cross-worker combine sums the partials element-wise into the final BINS-wide histogram.

This is the WRITE analog of the deferred 17-spmv DISTRIBUTED gather (whole-array broadcast). Two distinct hard problems vs the single-worker slice:
1. A data-dependent WRITE under a `partition=` schedule: the target bin `input[i]` is NOT statically known, so the transfer/halo inference cannot place writes to a partitioned `histogram` per-worker. Either (a) replicate the full `histogram` per worker (each worker scatters its input slice into a private full-width partial) + element-wise-sum combine on the host, OR (b) reject fail-loud under a partitioned schedule (today's behaviour — verify which guard fires: halo_inference data-dependent-stride rule, transfer_inject, or the scatter render path).
2. The cross-worker partial-histogram combine is the SAME overlapping-write accumulator fan-in TASK-0343 solved for 08-histogram/distributed (the masked variant): collect_accumulate_waits + render_wait_assign(accumulate) element-wise wrapping_add into a pre-initialised dest. Check whether the scatter variant's per-worker-partial combine reuses that helper or needs a new shape.

Scope: a distributed.scatter.sched.nuc + the partition-aware data-dependent-WRITE lowering/codegen; bit-identity vs reference.bin across the tier-1 backends where it compiles; honest [[skip]] for backends that can't. Companion to TASK-0044.04 / TASK-0341.03.02 (distributed gather).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 halo is_scatter_rmw fatal-under-partition gate is relaxed to ADMIT a scatter RMW ONLY for the input-index partition + full-histogram-replicate shape (each worker scatters its input slice into a private full-width partial histogram); it MUST stay FATAL for a bin-partition or any shape where replicate-per-worker is unsound. The discriminator is recorded in a docstring + unit-tested on BOTH arms (admit input-index-partition, reject bin-partition)
- [x] #2 histogram replicates WHOLE-ARRAY (not partitioned) to each worker; input i-bands; the cross-worker partial-histogram combine reuses TASK-0343 collect_accumulate_waits + render_wait_assign(accumulate) element-wise wrapping_add into a pre-initialised dest (or a new shape with recorded rationale if that helper does not fit)
- [x] #3 New schedule schedules/distributed.scatter.sched.nuc for prog.scatter.algo.nuc (partition=workers on input index i; histogram whole-array replicate; input i-band) + e2e-matrix.toml cells; byte-identical to reference.bin AND bit-identical cross-backend on every admitting backend; honest [[skip]] with task ref for any non-admitting backend
- [x] #4 FIFO send/recv order verified byte-identical on the STRICT-FIFO backends (mp-tcp-bufsync, mp-tcp-poll) — do NOT trust only the per-seq-demux backends passing, they mask Push/Wait order bugs (project-mp-tcp-event-vs-bufsync-safety-profile / TASK-0389). If an order mismatch surfaces, fix or file against TASK-0389
- [x] #5 Gate green: build/clippy/test/test-release 0-fail (modulo known TASK-0291 release -1); e2e existing baseline 357/300/0/57/0 preserved byte-identical, new cells added on top; all numbers re-run not transcribed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
PLAN (TASK-0384, building on TASK-0373 forward-carry):

1. Onboard: read prog.scatter.algo.nuc, schedules, kernels.scatter.rs, halo_inference.rs gate + stamping, transfer_inject opaque-dim path. DONE — confirmed: (a) data_out_access.indices = lhs.indices = [input[i]] so record_access_per_dim marks histogram dim 0 OPAQUE -> whole-array replicate (item #2 auto-handled); (b) the ONLY rejecting guard is error_is_fatal_under_partition: is_scatter_rmw && scope.any(partitioned).

2. Empirically reproduce TODAY's rejection: build release nucleus, write a throwaway distributed.scatter.sched.nuc (partition=workers(i), histogram+input sync), confirm it rejects with the halo DataDependentStride error. (de-risk before editing.)

3. Soundness discriminator (item #1, the crux): relax error_is_fatal_under_partition for a scatter RMW to ADMIT iff the scatter target ref_name is NOT affinely indexed by any partitioned iv anywhere in the algo (input-index partition + full-histogram-replicate). Keep FATAL if a partitioned iv affinely indexes the scatter target (bin-partition/band -> replicate-per-worker unsound). Encode as a helper scatter_target_replicates_whole_array(linked, ref_name) with a clear docstring. Unit-test BOTH arms (admit input-index-partition; reject synthetic bin-partition).

4. Add schedules/distributed.scatter.sched.nuc (partition=workers on i; transfer input:sync, histogram:sync) + e2e-matrix.toml cells for the admitting backends. Combine reuses TASK-0343 collect_accumulate_waits + render_wait_assign(accumulate) (verify, no new shape).

5. Gate: nix develop just build/clippy/test/test-release/e2e. Confirm bufsync AND poll byte-identical (NOT just demux backends). Preserve baseline 357/300/0/57/0, new cells add on top. Run check-* fences + full ci if time.

6. Honest notes; file follow-ups for any non-admitting backend with task ref + code comment.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0373 (distributed gather, landed b121365+14cc5b8). EMPIRICALLY VERIFIED which guard fires for the distributed scatter today (item #1): halo_inference DataDependentStride. The scatter RHS histogram[input[i]] is a data-dependent READ that halo_inference DOES walk (via visit_arg/process_call); the LHS write index input[i] is NOT walked by halo at all (halo only inspects kernel-call ARGS, never LHS). So the rejection point is the RHS RMW read, not the write.

TASK-0373 relaxed DataDependentStride to advisory ONLY for a PURE GATHER (affine LHS) and added an is_scatter_rmw flag (halo_inference.rs) that KEEPS a scatter RMW (data-dependent LHS) FATAL under partition. The flag is computed at collect_from_stmts: lhs.indices.any(expr_contains_dataref_or_call), threaded CallSite->IndexSite->DataDependentStride{is_scatter_rmw}. error_is_fatal_under_partition: is_scatter_rmw && any-scope-iv-partitioned. So today distributed scatter is rejected with: halo-inference error (under partitioned iv): kernel call inc reads histogram with a data-dependent index at axis 0.

CRITICAL GOTCHA for whoever lands TASK-0384: option (a) replicate-full-histogram-per-worker + element-wise-sum combine ALREADY WORKS END-TO-END if you simply flip is_scatter_rmw handling to advisory. I verified this: with a throwaway distributed_scatter.sched.nuc (partition=workers(i), input+histogram sync) and the unconditional-advisory halo relaxation, the emit was byte-identical to reference.bin on pthreads-sync — because each worker scatters its input-slice into a PRIVATE full-width histogram (vec![0;16]) and the TASK-0343 collect_accumulate_waits + render_wait_assign(accumulate) combine sums the partials element-wise at the host. So item #2 (cross-worker combine) is ALREADY HANDLED by the TASK-0343 accumulator helper — no new combine shape needed for option (a). The work for TASK-0384 is therefore mostly: (1) decide option (a) vs (b) [a works today, just gated off], (2) flip the is_scatter_rmw fatal gate to admit option (a) under partition, (3) confirm the input partition i-bands + histogram replicates-then-accumulates, (4) add distributed.scatter.sched.nuc + 7 cells. WATCH the FIFO send/recv order trap (TASK-0373 gotcha 2): for the scatter, input is the only host->worker broadcast and there is no gather-index array, so the order trap likely does NOT bite — but VERIFY bufsync/poll emit byte-identical (do not trust the 5 event backends passing; they mask order bugs per project-mp-tcp-event-vs-bufsync-safety-profile).

SOUNDNESS NOTE: option (a) replicate-per-worker is only correct because every input[i] is pre-clipped to a valid bin (prog.scatter.algo.nuc pre-condition) AND the partition is over input-index i, not over histogram bins. If a future schedule partitions over BINS, replicate-per-worker is wrong (each worker would only own a bin band). Keep is_scatter_rmw FATAL for any partition that is not the input-index-partition + full-histogram-replicate shape.

IMPLEMENTATION (cycle-223, building on TASK-0373). Landed exactly the option-(a) shape the forward-carry predicted.

WHAT CHANGED:
- nucleus/nucleus-compiler/src/passes/halo_inference.rs: relaxed error_is_fatal_under_partition DataDependentStride arm. Was `is_scatter_rmw && scope.any(partitioned)` (always fatal under partition). Now `is_scatter_rmw && scope.any(partitioned) && !scatter_target_replicates_whole_array(linked, ref_name)`. New helper scatter_target_replicates_whole_array walks linked.algo.stmts (collect_loop_vars + algo_target_has_affine_partitioned_index/expr_bands_target/indexed_ref_bands_target): returns false (KEEP FATAL) iff ANY index into the scatter target ref_name — on a write LHS OR a read DataRef, any depth — affinely references a partitioned iv (a BIN partition / band). A data-dependent index dim is skipped (opaque -> whole-array, the sound case). For prog.scatter.algo.nuc the only iv `i` indexes the SOURCE input[i], never histogram affinely, so histogram replicates whole-array -> ADMIT.
- Unit tests (BOTH arms, AC#1): task0384_input_index_partitioned_scatter_rmw_admits (canonical h[input[i]]<--inc(h[input[i]]) under partition=workers(i) -> ADVISORY) and task0384_bin_partitioned_scatter_rmw_stays_fatal (h[input[i]]<--inc(h[input[i]], h[i]) with i affinely indexing the target -> FATAL). Added build_linked_gather_arity test helper. The TASK-0373 test that asserted the canonical shape STAYS FATAL was renamed+inverted to the admit test (it was a deliberate-to-be-relaxed pin); doc-lie sweep updated 6 stale `stays FATAL / WRITE unhandled (TASK-0384)` comments across the module docs, field doc, two entry-point docstrings, collect_from_stmts + classify_index comments, and the test-overview block.
- nuc-nucleus/examples/08-histogram/schedules/distributed.scatter.sched.nuc (NEW): partition=workers on i; transfer input:sync, histogram:sync; full soundness-boundary docstring.
- nuc-nucleus/e2e-matrix.toml: 7 [[required]] distributed.scatter cells (all 7 tier-1 backends) + a doc block.
- example/scatter.sched docstrings updated (distributed scatter now exists).

AC#2 COMBINE (verified, NO new shape): emitted host combine is the TASK-0343 render_wait_assign(accumulate): `histogram[_k] = histogram[_k].wrapping_add(_tmp[_k])` x4 partials. Each worker has its own `let mut histogram: Vec<i32> = vec![0;16]`, iterates only its i-band (0..64 / 64..128 / 128..192 / 192..256), scatters into the private partial, pushes histogram.clone(). input i-bands via copy_from_slice of the band range. Confirmed by reading emitted src/main.rs.

AC#3+#4 BYTE-IDENTITY: all 7 backends emit AND run byte-identical to reference.bin (sha256 89f6dea9... identical across all 7 + reference). Strict-FIFO bufsync AND poll BOTH verified byte-identical (AC#4) — the Push/Wait FIFO order trap does NOT bite (input is the only host->worker broadcast, no gather-index array). Verified directly via 7 standalone `nucleus build`+run.sh invocations AND via the e2e harness (cells 204-210 all PASS as required).

GATE (re-run, not transcribed): build OK, clippy OK (-D warnings clean), dev test 1199 passed/0 failed, release test 1198 passed/0 failed (the -1 is the known TASK-0291 dev-only debug_assert #[should_panic], NOT a regression). e2e 364/307/0/57/0 (baseline was 357/300/0/57/0; +7 = the 7 new cells, existing baseline preserved). check-textual-replace / check-include-str / check-mega-files all OK. full `just ci` running.

GOTCHA / LIMITS: (1) The discriminator is conservative — it keeps FATAL on ANY partitioned-iv affine index into the target, including a NON-affine-but-partition-iv-mentioning index (treated as banding). (2) The repo working tree has PRE-EXISTING tree-wide rustfmt drift (50+ untouched files incl. mpi/embedded/dispatch, AND this file at HEAD); `just ci` does NOT gate on fmt; new test code matches the file's existing hand-wrapped multi-line call style (identical to the adjacent TASK-0373 tests) — did NOT run tree-wide cargo fmt (would create massive unrelated churn). (3) Soundness boundary unit-tested but the BIN-partition fatal arm is a synthetic fixture; the v2 grammar has no real bin-partition example today.

REVIEW-GATE FOLD-BACK (orchestrator, cycle close). Both reviewers GO. qa-test-runner independently re-ran (full just ci exit 0): test 1199/0/3 dev, 1198/0/3 release (-1 known TASK-0291), e2e 364/307/0/57/0 stable x2, all 7 distributed.scatter cells PASS on 7 backends incl. strict-FIFO mp-tcp-bufsync AND mp-tcp-poll (sha256 identical across all 7 + reference). mped-architect empirically verified discriminator soundness (mutation test: inverting it fails both unit arms), 7-backend byte-identity, combine correctness (host pre-init 0 + wrapping_add). Orchestrator fixed in-thread: (P2) the bin-partition fatal-test docstring + inventory comment claimed h gets BAND-partitioned and drops out-of-band scatters — FALSE: in that fixture h[input[i]] makes dim 0 opaque so h is broadcast WHOLE-ARRAY, the real unsoundness is the cross-band affine self-read h[i] not decomposing under replicate-then-sum; corrected both to state the precise mechanism. (P3a) softened the helper docstring overclaim that opacity is the only whole-array trigger. (P3b ROOT FIX) made algo_target_has_affine_partitioned_index descend lhs.indices (mirroring the RHS DataRef arm) so the at-any-depth contract holds and the LHS path is not an asymmetric blind spot; additive-conservative, unreachable today, bite-test follow-up filed as TASK-0390. Gate re-run GREEN after fixes (e2e 364/307/0/57/0, clippy 0 incl. a doc_lazy_continuation I introduced+fixed). Honest limit (accepted): the bin-partition unsound side is unit-tested only (no real grammar bin-partition example); the canonical input-index shape is fully e2e-verified 7-backend.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE (cycle-223). Distributed native scatter histogram[input[i]] <-- inc(histogram[input[i]]) under partition=workers(i) now lowers + emits + runs byte-identical to reference.bin across ALL 7 tier-1 backends. Landed exactly the option-(a) replicate-full-histogram-per-worker + element-wise-sum shape the TASK-0373 forward-carry predicted.

COMMITS: da716db (compiler: halo gate relax + helper + tests + doc-lie sweep), 785f698 (e2e+examples: distributed.scatter.sched.nuc + 7 cells + example docs).

PER-AC:
#1 DONE. error_is_fatal_under_partition DataDependentStride arm relaxed: `is_scatter_rmw && scope.any(partitioned) && !scatter_target_replicates_whole_array(linked, ref_name)`. New helper (halo_inference.rs) walks linked.algo.stmts and returns false (KEEP FATAL) iff any index into the scatter target — write LHS or read DataRef, any depth — affinely references a partitioned iv (a BIN/band partition); data-dependent index dims are skipped (opaque -> whole-array, the sound case). Discriminator recorded in the helper docstring + module/entry-point doc sweep. Unit-tested BOTH arms: task0384_input_index_partitioned_scatter_rmw_admits (ADVISORY) + task0384_bin_partitioned_scatter_rmw_stays_fatal (FATAL via the synthetic h[input[i]]<--inc(h[input[i]], h[i]) bin-band fixture).
#2 DONE. histogram replicates WHOLE-ARRAY (each worker `let mut histogram: Vec<i32> = vec![0;16]`); input i-bands (copy_from_slice of 0..64 / 64..128 / 128..192 / 192..256). The cross-worker combine REUSES the TASK-0343 render_wait_assign(accumulate) verbatim: `histogram[_k] = histogram[_k].wrapping_add(_tmp[_k])` x4 partials at the host. NO new combine shape (confirmed by reading emitted src/main.rs).
#3 DONE. schedules/distributed.scatter.sched.nuc (partition=workers on i; transfer input:sync, histogram:sync) + 7 [[required]] e2e cells. All 7 admitting backends run byte-identical to reference.bin (sha256 89f6dea9... identical across all 7 + reference) AND bit-identical cross-backend. NO [[skip]] needed — all 7 admit.
#4 DONE. Strict-FIFO mp-tcp-bufsync AND mp-tcp-poll BOTH verified byte-identical (not just the 5 demux backends). The Push/Wait FIFO order trap does NOT bite: input is the only host->worker broadcast, no gather-index array (TASK-0389-class issue does not arise here). Verified via 7 standalone nucleus build+run.sh runs AND via the e2e harness (cells 204-210 PASS).
#5 DONE. build OK; clippy -D warnings clean; dev test 1199 passed/0 failed; release test 1198 passed/0 failed (-1 = known TASK-0291 dev-only debug_assert #[should_panic], not a regression); e2e 364/307/0/57/0 (baseline 357/300/0/57/0 preserved, +7 new cells). Full `just ci` GREEN exit 0 (all check-* fences + 4 negative/determinism arms behaved correctly; the FAIL lines in the ci tail are the INTENTIONAL xbackend/required-coverage negative-arm injections, both caught -> OK). All numbers re-run, not transcribed.

LIMITS / honesty: (a) discriminator is conservative — keeps FATAL on any partitioned-iv affine index into the target, incl. a non-affine-but-partition-iv-mentioning index. (b) the bin-partition FATAL arm is a SYNTHETIC fixture; the v2 grammar has no real bin-partition histogram example today (the soundness boundary is unit-tested, not e2e-exercised on the unsound side). (c) the repo working tree has PRE-EXISTING tree-wide rustfmt drift (50+ untouched files + this file at HEAD); `just ci` does not gate fmt; new test code matches the file's existing hand-wrapped style — did NOT run tree-wide cargo fmt (would create unrelated churn). No follow-up tasks filed — the canonical shape is fully closed and no stub/workaround was needed.
<!-- SECTION:FINAL_SUMMARY:END -->
