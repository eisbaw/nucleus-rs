---
id: TASK-0354
title: >-
  Unit tests for collect_let_at_wait_data + is_whole_array_recv (TASK-0349 cycle
  220b architect P2.2)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-27 23:58'
updated_date: '2026-05-28 00:54'
labels:
  - tests
  - backend-common
  - defensive
  - cycle-220b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 architect P2.2: the new public helpers `collect_let_at_wait_data` (nucleus/backend-common/src/multi_worker_walker/collect.rs) and `is_whole_array_recv` (nucleus/backend-common/src/multi_worker_walker/wait.rs) have only INDIRECT e2e-gate coverage. Per project test discipline (TASK-0304 / TASK-0310 behaviour-pin precedent), add unit-level positive + negative tests:

1. Mixed-mode (one slice, one whole) Wait of the same data -> stays OUT of let_at_wait set.
2. Accumulate-fan-in data -> stays OUT (the wrapping_add identity needs the zero-init).
3. Indexed-Fire-written data -> stays OUT (the indexed assigns need the zero-init).
4. Empty Waits -> empty result.
5. Shape-error on wait_slice for one of the Waits -> .unwrap_or(false) propagates as 'not all whole' -> stays OUT.
6. Pure whole-array Waits inside an Event::Loop body -> classifier descends, data IN let_at_wait.
7. Scalar data (ty.dims empty) -> classifier handles correctly.

## Honest scope

Doc-only / test-only; no Rust code changes. Defensive against future drift of the let-at-wait classifier semantics.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implementation plan (cycle 221): create one new integration test file nucleus/backend-common/tests/collect_let_at_wait_data.rs with a private make_minimal_tables helper (DataId -> name + ResolvedType{ScalarType::I32, dims}) and 7 #[test] fns mapping 1:1 to the description's 7 cases. All tests drive the PUBLIC collect_let_at_wait_data entry point (exercising the pub(super) is_whole_array_recv indirectly via the documented call site at collect.rs:392). Test names: mixed_whole_and_slice_waits_excludes_data, accumulate_fan_in_data_excluded, indexed_fire_written_data_excluded, empty_waits_yields_empty_set, shape_error_on_wait_slice_excludes_data (out-of-bounds leading range trips wait_slice:269-278 guard -> Err -> unwrap_or(false) -> excluded), whole_array_wait_inside_event_loop_body_included (Event::Loop with whole-array Wait in body -> descent + included), scalar_data_no_dims_treated_as_whole_array. Gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e' must pass; e2e baseline 120/110/0/10/0 preserved (test-only additive); cargo test +7 delta.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 221 (commit 8ad74fa) landed all 7 cases as named in the plan; gate green end-to-end.

Measured numbers:
- cargo test (dev): 1019/0/3 → 1026/0/3 (+7 exactly as planned)
- cargo test (release): 1018/0/3 → 1025/0/3 (+7; preserves TASK-0291 1-test debug_assert delta)
- just e2e: 280/246/0/34/0 unchanged (preserved bit-identical baseline; test-only additive change touches no codegen)
- just clippy --all-targets -D warnings: clean
- just check-textual-replace-on-codegen / -include-str-coverage / -narrative-doc-lie / -mega-files: all clean

Per-case status (all covered):
1. mixed_whole_and_slice_waits_excludes_data — PASS (whole + slice tile on same DataId via two SeqTags)
2. accumulate_fan_in_data_excluded — PASS (whole Wait + accumulate_data membership)
3. indexed_fire_written_data_excluded — PASS (whole Wait + indexed set membership)
4. empty_waits_yields_empty_set — PASS (events: vec![])
5. shape_error_on_wait_slice_excludes_data — PASS (dims=[8], tile leading range 0..1024 trips wait_slice:269-278 out-of-bounds guard → Err propagates via is_whole_array_recv:381's ? → collect_let_at_wait_inner:392 .unwrap_or(false) → excluded)
6. whole_array_wait_inside_event_loop_body_included — PASS (Event::Loop wrapping a whole-array Wait → classifier descends per collect.rs:399-401 → included)
7. scalar_data_no_dims_treated_as_whole_array — PASS (dims=vec![], non-empty tile bounds to force the wait.rs:265-267 ty.dims.is_empty() arm to short-circuit before ty.dims[0] read)

Gotchas / non-obvious bits:
- Test 5: chose out-of-bounds range over rank-3 tile (the wait.rs:307 rank guard) because the leading-dim guard is cheaper to set up and reads clearer at the call site. The rank guard would also work observationally (same Err → same .unwrap_or(false) arm).
- Test 5 rejected approach: omitting the data_types entry would instead trip wait_slice:259-263 (NameSidecar::data_type returns None → ContractGap). ALSO observationally equivalent for this classifier (same Err propagation). Chose out-of-bounds range so the test's intent is unambiguous about WHICH guard fires.
- Test 7 ordering subtlety: a scalar (dims=[]) with a non-empty tile must NOT panic; the wait.rs:265-267 ty.dims.is_empty() check runs BEFORE the wait.rs:268 'let leading_dim = ty.dims[0] as i64' read. The test deliberately constructs that exact shape (non-empty tile + empty dims) to pin the ordering — if a future refactor reorders the dims read above the empty check, test 7 will panic loudly.
- make_minimal_tables helper deliberately omits the host worker entry that tests/wait_assign_slice.rs's helper carries — collect_let_at_wait_data reads only sidecar.data_types, never names.worker (verified by greppping the implementation). Per the silent-sibling-defect discipline this divergence is intentional and documented in the helper's docstring.

is_whole_array_recv visibility: kept pub(super) per architect P2.2; all 7 tests reach it only via collect_let_at_wait_data → collect_let_at_wait_inner → wait::is_whole_array_recv. No widening.

Cycle 221b — review-gate fold-back: architect P1.1 (phantom-Fire comment-doc-lie at tests/collect_let_at_wait_data.rs:207-210) removed; P2.3 (test 3 name oversells) renamed indexed_fire_written_data_excluded -> indexed_input_data_excluded + symmetric docstring clarification on test 2. P2.1 + P2.2 filed as TASK-0357 + TASK-0358 (not folded — would expand scope; precedent: cycle-220b filed TASK-0354/0355/0356 rather than expanding cycle-220).

Re-run gate after fold-back: cargo test 1026/0/3 preserved exactly (no test count drift), clippy clean, just e2e 280/246/0/34/0 bit-identical to cycle-221 baseline.

Commit: d3719e8.

Memory update: feedback-comment-doc-lie-recurring 18th firing; in this case ALSO caught independently by the orchestrator's pre-review skim before the architect report landed — two independent reads on a 338 LoC test file beat one.

All 7 numbered cases covered + cycle-221b architect-review findings folded back (P1.1 doc-lie + P2.3 test name + symmetric test 2 docstring clarification). P2.1 + P2.2 architect findings filed as TASK-0357 + TASK-0358 (test-only coverage extension + helper consolidation; do not block closure of TASK-0354 since they extend coverage beyond the 7 originally-scoped cases). Gate green end-to-end: cargo test 1026/0/3 (dev), 1025/0/3 (release), clippy clean, just e2e 280/246/0/34/0 bit-identical across 2 independent runs. Commits: 8ad74fa (cycle 221 implementation) + ded21ee (cycle 221 tracker notes) + d3719e8 (cycle 221b fold-back + 2 follow-up tasks filed).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
All 7 cases covered; gate green end-to-end; e2e 280/246/0/34/0 preserved bit-identical (test-only additive); cargo test +7 dev / +7 release. is_whole_array_recv visibility kept pub(super) per architect P2.2 — exercised indirectly through collect_let_at_wait_data per the documented call site at collect.rs:392. Forward-carried 5 (data, tile) shape coverage + 1 sibling-divergence keystone finding to TASK-0355 (unify is_whole_array_tile + is_whole_array_recv) so the future unification implementer doesn't re-derive them. Commit 8ad74fa.
<!-- SECTION:FINAL_SUMMARY:END -->
