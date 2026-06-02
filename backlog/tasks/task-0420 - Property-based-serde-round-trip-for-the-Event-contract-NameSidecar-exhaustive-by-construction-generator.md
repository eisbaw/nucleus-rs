---
id: TASK-0420
title: >-
  Property-based serde round-trip for the Event contract + NameSidecar
  (exhaustive-by-construction generator)
status: To Do
assignee: []
created_date: '2026-06-02 01:06'
updated_date: '2026-06-02 01:07'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-240 COVERAGE-AUDIT EVIDENCE (why this is the ONLY genuine residual hardening gap). Filed as the output of a fresh-hardening-wave scope per phase3-ralph backlog-maturity. An Explore-agent typed-error coverage map initially claimed 8-10 positive-only gaps; ALL were verified to be coverage-audit-undercount artifacts (memory feedback-coverage-audit-undercount-recurring fired 3x in one investigation):
- EmitError::ContractGap -> BITE-TESTED backend-common/tests/block_tag_loop_header.rs:170 (panic expected ContractGap). Explore missed it (scanned nucleus-compiler/tests/, not backend-common/tests/).
- EmitError::AccumulatorShapeMismatch -> BITE-TESTED backend-common/tests/accumulator_cross_check.rs:127 (asserts msg names symbol + cites TASK-0343.03).
- EmitError I/O (KernelsReadFailed/OutputCreateFailed/WriteFailed) -> bite-tested canonical backend (TASK-0404); per-backend follow-up already tracked TASK-0405 (explicitly low-yield).
- PartitionBlocks2dError::InnerRepeatNotFound -> UNREACHABLE defensive belt; unreachability already pinned by unit test inner_repeat_not_found_unreachable_contains_repeat_iff_first_repeat_in (same correctly-handled pattern as TASK-0419). Not a gap.
- ReuseInferenceError 4 (DataDependentStride/MultipleIterVarsInIndex/NonAffineIndex/NonContiguousOffsets) + HaloInferenceError 4 (DataDependentStride/MultipleIterVarsInIndex/UnknownKernelInCall/UnknownLoopVar) -> ALL bite-tested in INLINE #[cfg(test)] mod blocks in reuse_inference.rs / halo_inference.rs (panic "expected X, got" at reuse 1284/1315/1346/1404 + halo 1902/1964/2967, match-assert halo 719/2902). Missed by tests/-scoped grep (the -g **/tests/** flag EXCLUDES inline mods).
VERIFIED CONCLUSION: the compiler typed-error bite-test surface is effectively 100% for all reachable variants; unreachable belts (InnerRepeatNotFound, TASK-0419) are properly unreachability-pinned. The serde round-trip is the only verified-absent PROPERTY test. GREP DISCIPLINE for any future coverage audit: grep tests/ AND inline #[cfg(test)] mod AND sibling-crate */tests/ (backend-common, backends/*, driver) — never -g **/tests/** alone.
<!-- SECTION:NOTES:END -->
