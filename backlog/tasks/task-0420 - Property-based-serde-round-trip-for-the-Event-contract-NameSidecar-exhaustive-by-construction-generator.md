---
id: TASK-0420
title: >-
  Property-based serde round-trip for the Event contract + NameSidecar
  (exhaustive-by-construction generator)
status: Done
assignee:
  - '@mped'
created_date: '2026-06-02 01:06'
updated_date: '2026-06-02 02:08'
labels:
  - hardening
  - testing
  - proptest
  - serde
  - cycle-240-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
GENUINE residual hardening gap found by the cycle-240 typed-error/serde coverage audit (the ONLY one — see notes). The Event enum (nucleus-compiler/src/event.rs:679) is the codegen contract boundary; NameSidecar + the ACFG sidecar carry the partition/halo/reuse/buffer metadata. Their serde round-trip is currently pinned ONLY by ~40 hand-rolled example cases (tests/event.rs roundtrip helper ~line 198; tests/sidecar_*.rs per-field examples) plus a legacy-payload parse test. There is NO property-based (proptest) round-trip over arbitrary Event trees / sidecar values.

WHY THIS MATTERS (the specific risk): the memory project-event-sync-synctag flags a serde required-field contract-version caveat; serde-contract evolution is a real risk area. Hand-rolled examples do NOT break-to-update when a new Event variant or field is added (you must remember to add an example), so a variant added without round-trip safety can slip through silently. A generator written EXHAUSTIVE-BY-CONSTRUCTION (a match over every Event variant, so adding a variant forces a compile error / generator update) closes that completeness gap.

HONEST VALUE: MODERATE, not high. The 40 examples already cover every current variant; this adds (a) arbitrary nesting/combination coverage (nested Loop bodies, all field shapes) and (b) the break-to-update completeness guard. It does NOT close a correctness gap.

SCOPE:
1. Add a proptest strategy generating arbitrary Event trees: exhaustive-by-construction over all Event variants (Fire/Alloc/Push/Wait/Sync/Free/Loop + nested Loop bodies, bounded depth), all field shapes (IterTile bounds, FireBinding args, SyncTag, ranges incl degenerate/inverted, block_tag Some/None, check_frame). Property: from_str(to_string(e)) == e (round-trip identity; Event derives PartialEq).
2. Same for NameSidecar / the ACFG sidecar serde (partition_worker_ranges, halo_widths, reuse_widths, blocks2d pairs+grid, buffer): arbitrary value -> serialize -> deserialize -> equal.
3. Keep it in the proptest_* test-binary convention (mirror tests/proptest_parser.rs + tests/proptest_petri.rs structure + module docstring stating the invariants).
4. Gate: nix develop -c just build clippy test test-release e2e; e2e baseline 385/328/0/57/0 HELD (additive test only — no production code change expected). Run proptest enough cases to be meaningful; pin a PROPTEST_CASES floor in the module docs.

NON-GOAL / DO NOT re-file: the typed-error bite-test surface is ALREADY ~100% covered (see notes) — do not file bite-test tasks for it.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#1: new tests/proptest_serde.rs. event_strategy() EXHAUSTIVE-BY-CONSTRUCTION: a prop_oneof! with one arm per Event variant (Fire/Alloc/Push/Wait/Sync/Free/Loop). A trailing fn event_variant_completeness_guard(e: &Event) match with NO wildcard so adding a variant forces a compile error here. Sub-strategies: itertile_strategy (Vec<(IterVar, Range<i64>)>, incl degenerate/inverted/empty ranges via arbitrary i64 start/end), firebinding_strategy (recursive ArgBinding incl Nested), bounded-depth recursive event_leaf+event_recursive for Loop bodies (depth <= 2, leaf base case), block_tag Some/None, check_frame Some/None, multi-participant Sync sets. Property: from_str(to_string(e)) == e.
2. AC#2: namesidecar_strategy() arbitrary NameSidecar over ALL serde-bearing fields (data_types, consts, loop_bounds, kernel_sigs, partition_worker_ranges, transfer_buffer_for_seq, halo_widths, reuse_widths, partition_pairs, grid_shape_for_outer_iv, cumulative_data). Field-completeness guard match with no wildcard. Round-trip property.
3. AC#3: rich //! docstring mirroring proptest_petri.rs (Scope / Honest-failure path / Generator honest limits), PROPTEST_CASES floor documented.
4. AC#4 gate: nix develop -c just build clippy test test-release e2e; hold 385/328/0/57/0. Run just test >=2x non-flake. Do NOT commit proptest-regressions (repo commits none). Honest-failure: any found round-trip failure -> file prereq task + #[ignore] + STOP.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-240 COVERAGE-AUDIT EVIDENCE (why this is the ONLY genuine residual hardening gap). Filed as the output of a fresh-hardening-wave scope per phase3-ralph backlog-maturity. An Explore-agent typed-error coverage map initially claimed 8-10 positive-only gaps; ALL were verified to be coverage-audit-undercount artifacts (memory feedback-coverage-audit-undercount-recurring fired 3x in one investigation):
- EmitError::ContractGap -> BITE-TESTED backend-common/tests/block_tag_loop_header.rs:170 (panic expected ContractGap). Explore missed it (scanned nucleus-compiler/tests/, not backend-common/tests/).
- EmitError::AccumulatorShapeMismatch -> BITE-TESTED backend-common/tests/accumulator_cross_check.rs:127 (asserts msg names symbol + cites TASK-0343.03).
- EmitError I/O (KernelsReadFailed/OutputCreateFailed/WriteFailed) -> bite-tested canonical backend (TASK-0404); per-backend follow-up already tracked TASK-0405 (explicitly low-yield).
- PartitionBlocks2dError::InnerRepeatNotFound -> UNREACHABLE defensive belt; unreachability already pinned by unit test inner_repeat_not_found_unreachable_contains_repeat_iff_first_repeat_in (same correctly-handled pattern as TASK-0419). Not a gap.
- ReuseInferenceError 4 (DataDependentStride/MultipleIterVarsInIndex/NonAffineIndex/NonContiguousOffsets) + HaloInferenceError 4 (DataDependentStride/MultipleIterVarsInIndex/UnknownKernelInCall/UnknownLoopVar) -> ALL bite-tested in INLINE #[cfg(test)] mod blocks in reuse_inference.rs / halo_inference.rs (panic "expected X, got" at reuse 1284/1315/1346/1404 + halo 1902/1964/2967, match-assert halo 719/2902). Missed by tests/-scoped grep (the -g **/tests/** flag EXCLUDES inline mods).
VERIFIED CONCLUSION: the compiler typed-error bite-test surface is effectively 100% for all reachable variants; unreachable belts (InnerRepeatNotFound, TASK-0419) are properly unreachability-pinned. The serde round-trip is the only verified-absent PROPERTY test. GREP DISCIPLINE for any future coverage audit: grep tests/ AND inline #[cfg(test)] mod AND sibling-crate */tests/ (backend-common, backends/*, driver) — never -g **/tests/** alone.

LANDED (cycle-241). New file: nucleus/nucleus-compiler/tests/proptest_serde.rs (additive test-only; ZERO production-code change). Two proptest properties:
- event_serde_roundtrip: arbitrary Event trees, from_str(to_string(e))==e. event_strategy() is EXHAUSTIVE-BY-CONSTRUCTION (prop_oneof! one arm per variant Fire/Alloc/Push/Wait/Sync/Free + recursive Loop) + event_variant_completeness_guard() wildcard-free match = break-to-update teeth. Covers: all 7 variants; nested Loop bodies (prop_recursive depth<=2, leaf base case); degenerate/inverted/empty Range<i64> (any::<i64>() start/end); block_tag Some/None x check_frame Some/None (arbitrary combos incl no-real-pass shapes); multi-participant Sync sets 0..=4 incl empty; recursive ArgBinding::Nested + IrExpr (depth<=2); IterTile incl empty.
- sidecar_serde_roundtrip: arbitrary NameSidecar over ALL 11 serde-bearing fields (data_types, consts, loop_bounds, kernel_sigs, partition_worker_ranges, transfer_buffer_for_seq, halo_widths, reuse_widths deep-nest, partition_pairs, grid_shape_for_outer_iv, cumulative_data) + sidecar_field_completeness_guard() no-.. destructure = break-to-update teeth. NOT scope-narrowed (full field set covered) -> no honest-limit declared on the sidecar arm beyond small map sizes/depth.
PROPTEST_CASES=256 floor (documented in //!). Rich //! mirrors proptest_petri.rs (Scope / break-to-update guard / serde-fidelity-not-validity / Honest-failure path / Generator honest limits / Case count).
NO round-trip failure found (256 + 256 + 4000-case stress all green) -> honest-failure path NOT triggered; clean additive outcome. No proptest-regressions/ written (repo commits none; none tracked/gitignored -> matched convention).
GATE (actual): just build clean; just clippy clean -D warnings (removed an unused BTreeSet import that would have RED-ed clippy); just test 1247 passed/0 failed/3 ignored (proptest_serde 2 ok); just test-release 1245 passed/0 failed/3 ignored (proptest_serde 2 ok; 2 fewer than dev = expected debug_assert-gated #[should_panic] dev-only tests); just e2e 385/328/0/57/0 HELD EXACTLY. Non-flake: just test run 2x + 4000-case stress, all green, no seed file.
GOTCHAS/SUBTLETIES: (1) ScalarType F32/F64 are FIELDLESS tags (no float payload) -> no NaN-equality hazard; no f32/f64 VALUE carried by either contract type. (2) BlockTag/ReuseSlot are NOT crate-root re-exported -> referenced via event::BlockTag / passes::reuse_inference::ReuseSlot. (3) String fields use small ASCII ident alphabet not arbitrary UTF-8 (full-Unicode JSON-escape fidelity is serde_json own contract). (4) recursion depth<=2 limit: a serde-recursion bug only at depth>=3 would be missed (no evidence such class exists; serde derive is depth-agnostic; tests/event.rs pins one hand-built depth-2 nest too).

ORCHESTRATOR REVIEW GATE (phase3-ralph, parallel read-only, commit fcc3dff) — both GO. qa-test-runner INDEPENDENTLY RE-RAN: build+clippy clean (forced recompile, no doc_lazy_continuation on the large //! docstring); just test 1247/0/3; just test-release 1245/0/3; the proptest_serde binary genuinely runs its 2 properties (event_serde_roundtrip + sidecar_serde_roundtrip), passing across 4 invocations (non-flake), NOT a 0-case no-op; just e2e 385/328/0/57/0 x2 byte-identical; no proptest-regressions seed file leaked; tree clean. mped-architect INDEPENDENTLY VERIFIED the load-bearing break-to-update guard: Event 7/7 wildcard-free match (cross-checked enum); NameSidecar 11/11 serde fields no .. (independently enumerated src/sidecar.rs); round-trip is a real prop_assert_eq! with serde active (default=[serde]); ALL doc claims accurate incl the NaN-non-hazard (no f32/f64 VALUE carried by either contract — ScalarType F32/F64 are fieldless tags). 

ORCHESTRATOR FOLD-BACK (commit f5a2856, in-thread, re-gated): architect P3.1 (SyncKind generator hardcoded Just(Barrier), un-guarded — a future Rendezvous/Quorum would silently never generate) + P3.2 (scalar_type doc OVERSTATED: claimed prop_oneof break-to-update enforcement that a Just(..) list does NOT have — a minor doc-lie). Fixed BOTH by adding wildcard-free *_completeness_guard matches for SyncKind/ScalarType/IrBinOp/ViolationKind (making the break-to-update claim TRUE rather than weakening it) + corrected the scalar_type doc + Sync-arm comment + module docstring. Re-gate: build+clippy clean (no doc_lazy_continuation), proptest_serde 2 props pass dev+release, just test 0 failed, e2e 385/328/0/57/0 HELD. RECORDED NUMBERS ARE REVIEWER-RE-RUN / orchestrator-re-run, not implementer-claimed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Added nucleus-compiler/tests/proptest_serde.rs: two exhaustive-by-construction proptest round-trips (Event + NameSidecar) with wildcard-free completeness guards as the break-to-update teeth over the 40 hand-rolled examples. AC#1 (Event): MET. AC#2 (NameSidecar all 11 fields, not narrowed): MET. AC#3 (proptest_* convention + rich //! mirror): MET. AC#4 (gate, e2e 385/328/0/57/0 held): MET. No round-trip bug found (clean additive hardening; not a found-bug cycle). Independent qa+architect review gate pending.
<!-- SECTION:FINAL_SUMMARY:END -->
