---
id: TASK-0408
title: >-
  Hardening: documented-invariant-is-asserted audit + comment/doc-lie sweep
  (review pass)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 07:35'
updated_date: '2026-06-01 08:57'
labels:
  - hardening
  - doc-lie
  - review-pass
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 endgame, review-pass dimension. The doc-citation-STALENESS fences (path:line, test-name, cell-path) are saturated, but a DIFFERENT doc-validation gap remains: (a) every invariant a docstring CLAIMS the code enforces should have an actual assertion/test (memory feedback-comment-doc-lie-recurring: a multi-claim docstring saying X-happens-because-Y is a CLAIM not a FACT; spot-check 3-5 per review and verify against the code); (b) examples in docs/PRD that purport to run should run.

SCOPE: sweep high-traffic module docstrings (the passes, backend-common render, event/sidecar contracts) for X-because-Y claims and verify each against the code; where a documented invariant has no assertion, add one (a debug_assert or a test) or correct the doc. This is the recurring comment-doc-lie class CLAUDE.md flags as a per-review audit. Several already fixed reactively this session (the TASK-0402 link-step-vs-build_acfg attribution; the SchedLowerErrorKind count). This task does it PROACTIVELY as a sweep.

METHOD: verify-against-code every claim before trusting it (memory feedback-implementer-disclosure-mechanism-wrong + feedback-coverage-audit-undercount-recurring -- 3 under-counts cycle-236, all caught by the gate). Deliverable = corrections + assertion-additions through the normal gate, or precise follow-ups. LOWER leverage; best in a FRESH context.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation (cycle-236 follow-up, bounded spot-check)

Verified 10 load-bearing X-because-Y / documented-invariant claims by tracing the ACTUAL code path (never inferring from symbol name). Result: 9 verified-TRUE-and-backed, 1 doc-lie CORRECTED (with a 2nd borderline sibling + a 3rd echo tightened for consistency).

### VERIFIED-TRUE (no change needed; claim matches code + is tested where it asserts enforcement):
1. sync_inject.rs:119-124 UncoveredCrossPartitionReducer refuses loudly -- TRUE, code returns Err at line 529 on the exact shape; backed by tests/sync_inject.rs:653. TASK-0268 mechanism described accurately.
2. render/fire.rs:50 length-assert exists because LHS sub_len from sidecar vs RHS from kernel author -- TRUE, assert_eq! emitted at fire.rs:76, fires AFTER let _rhs as claimed.
3. render/fire.rs:393-395 grammar IDENT (\[EXPR\])* always indexes outer dims first / no skip-marker -- TRUE, both parser sites (parser.rs:560, :711) build indices from .repeated() in source order; IndexedLValue.indices is positional Vec (ast.rs:133).
4. inject_check_frames.rs:53-56 link step rejects unknown check-loop var -- TRUE, link/build.rs:117-130 pushes rejection on !loop_vars.contains. (TASK-0403 separately proves the silent-drop arm; not duplicated here.)
5. render/reuse.rs:75-80 name_data coverage invariant -- doc is HONEST: it frames the fallback as defensive (d<id> form), does NOT claim enforcement; code matches (reuse.rs:98-102).
6. host_election.rs:106 sorted-ascending invariant preserved by construction -- TRUE and EXEMPLARY: backed by debug_assert! (lines 122-126) AND #[should_panic] test (line 339).
7. collect.rs:419-482 check_accumulator_consistency fails loud / reuses collect_accumulate_waits verbatim / LHS-in-RHS via collect_dataref_names / Effect irrelevant / ContractGap on missing DataId / conservative-reject-never-silent -- ALL TRUE; backed by tests/accumulator_cross_check.rs.
8. wait.rs:293-308 axis-mapping upstream-enforced by compute_partition_bounds_with_dim_prefix -- TRUE; partition.rs:526-580 emits in data-dim order + drops sparse non-prefix coverage to whole-array (line 575); wait_slice deliberately does NOT consult _iv (line 344); doc honestly scopes open shapes.
9. partition_workers.rs:371 ZeroWorkers pre-empted by NoMultiWorkerBody (len<2) -- TRUE (line 238).

### CORRECTED (doc-lie -- the CODE was correct; only the WHY was false):
10. partition_workers.rs:372-373 (commit 3809fdf): claimed InvalidRange (hi<lo) is caught by the link step eval_const invariant before reaching the pass. FALSE: eval_const (acfg/build.rs:650) only evaluates bound exprs to i64, no range invariant; link step check order (build.rs steps 1-6) has no hi>=lo gate; grep of algo/ sched/ link/ found ZERO inverted-range gate. Real backstop = compute_partition_bands (common.rs:181) -> InvalidRange -> map_band_error -> InsufficientWork. Original doc was internally contradictory (next sentence already credited the band helper).
   - SIBLING partition_rows.rs:406-408 had the same imprecision (guarded upstream anyway) -- tightened to match + cross-ref.
   - ECHO common.rs:117-119 InvalidRange variant doc (hi>=lo from eval_const) -- same conflation, reworded.

### ASSERTION ADDED (half-(b)):
- NEW test partition_workers::inverted_range_maps_to_insufficient_work_not_panic. The InvalidRange=>InsufficientWork mapping arm (the backstop the corrected doc now claims) was UNTESTED -- only the InsufficientWork arm had coverage. Now pinned.

### GATE (re-run this cycle, NOT transcribed): build clean; clippy clean (no doc_lazy_continuation); just test 1237 (+1 from new test); test-release 1236 (the 1-test dev/release delta is the documented debug_assert-gated divergence, pre-existing, not mine); e2e 385/328/0/57/0 unchanged.

### HONEST LIMITS (what I did NOT sweep):
- This is a SPOT-CHECK of 10 claims (intended scope), NOT exhaustive. ~66 grep hits in the 8 passes alone remain unverified.
- NOT deeply swept: halo_inference.rs (132KB, largest), reuse_inference.rs (65KB), host_data_relay_inject.rs (54KB), acfg_to_petri.rs body, petri_to_events.rs, event_plan/plan.rs claims beyond the grep listing. These are candidates for a future bounded pass.
- The backend-common render asymmetry (only 2 asserts across all render/*.rs) is REAL but the docstrings I checked there (reuse.rs, fire.rs) correctly do NOT overclaim enforcement, so no soften-or-assert was warranted on the ones I read.

### GOTCHAS for next subagent:
- The doc-lie I found was an internally-contradictory docstring: paragraph 1 named the wrong gate (eval_const), paragraph 2 credited the right one (band helper). When a docstring explains the SAME safety property twice, cross-check the two explanations against each other -- a mismatch is a tell.
- eval_const-attribution is a recurring conflation here: it appeared in THREE places (partition_workers, partition_rows, common.rs variant doc) as verbatim-ish copies. Grep for the symbol named in a why-claim across siblings before declaring closure.
- The target modules are in GOOD shape -- 9/10 claims verified true and several (host_election, collect, sync_inject) are exemplary (doc + debug_assert + should_panic/test). The staleness fences really are saturated; the remaining gap is narrative-WHY accuracy, which is lower-density than expected.

ORCHESTRATOR REVIEW GATE (cycle-237): parallel read-only qa-test-runner + mped-architect, both GO on commit 3809fdf. qa: re-ran full gate -- build clean, clippy exit 0 (forced fresh re-check, caught just-clippy serving a 0.14s cached result; no doc_lazy_continuation), test 1237 dev / 1236 release, e2e 385/328/0/57/0 on TWO non-flaking runs (baseline held). architect: INDEPENDENTLY traced all 3 corrected-doc claims to code (eval_const has no range invariant; link build.rs steps 1-7 have no hi>=lo gate; compute_partition_bands->InvalidRange->map_band_error->InsufficientWork is the real fail-LOUD backstop, Err not panic) -- ALL TRUE, the correction does not replace one lie with another. New test confirmed meaningful+additive, not vacuous. Honesty spot-checks of 2 verified-true claims (fire.rs length-assert, host_election sorted-ascending debug_assert+should_panic) confirmed genuinely backed. Two P3 fold-backs applied IN-THREAD (commit cb5fc51, comment-only, gate re-run green): (P3a SILENT-SIBLING) the architect found a MISSED 4th eval_const-conflation site -- the comment inside test bands_inverted_range_rejects (common.rs:650) still carried the old lie; the implementer-3809fdf /// grep could not see it because it is a test-body comment not a /// docstring. Reworded; grep confirms all 5 sites now consistent, ZERO remaining. (P3b) map_band_error doc said (len, workers) payload but variant carries {var, lo, hi, workers}; corrected. METHOD LESSON for next audit: comment-doc-lie sweeps must grep test-body + match-arm comments, NOT only /// docstrings -- the canonical-narrative conflation hid in a #[test] body.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Bounded proactive comment-doc-lie + documented-invariant-asserted sweep across the high-traffic passes and backend-common render/walker/host_election modules. Spot-checked 10 load-bearing X-because-Y / enforced-invariant claims by tracing the ACTUAL code path (not inferring from symbol names). Result: 9 verified-TRUE-and-backed (sync_inject UncoveredCrossPartitionReducer refusal+test; fire.rs length-assert + outer-dims-first grammar claim; inject_check_frames link-step rejection; reuse.rs honest-defensive fallback; host_election sorted-ascending debug_assert+should_panic; collect.rs accumulator cross-check + tests; wait.rs axis-mapping upstream-enforcement; partition_workers ZeroWorkers pre-empt). 1 doc-lie CORRECTED + 2 sibling/echo tightenings (commit 3809fdf): the map_band_error docstrings wrongly attributed the inverted-range (hi<lo) guard to the link step eval_const invariant -- verified there is NO such gate upstream (eval_const only evaluates bounds; link build.rs steps 1-6 have no range check); the real fail-closed backstop is compute_partition_bands -> InvalidRange -> map_band_error -> InsufficientWork. Fixed in partition_workers.rs + sibling partition_rows.rs + the common.rs InvalidRange variant doc (verbatim-ish conflation in all three). Added the previously-UNTESTED InvalidRange=>InsufficientWork mapping test (half-(b): documented backstop now asserted). Self-reviewed adversarially: confirmed this was a wrong-WHY doc-lie, NOT papering over a bug -- the mapping behavior is the deliberate narrow-error-surface design, fail-loud on an unreachable-from-valid-input defensive path. Gate re-run this cycle: build+clippy clean (no doc_lazy_continuation), just test 1237 (+1), test-release 1236 (pre-existing dev/release delta), e2e 385/328/0/57/0 unchanged. HONEST LIMIT: this is a 10-claim spot-check (intended scope), NOT exhaustive; halo_inference/reuse_inference/host_data_relay_inject/acfg_to_petri/petri_to_events/event_plan bodies remain unswept and are candidates for a future bounded pass. Lessons forward-carried to TASK-0407 (eval_const-attribution conflation recurrence + PartitionBandError variant reachability classification).
<!-- SECTION:FINAL_SUMMARY:END -->
