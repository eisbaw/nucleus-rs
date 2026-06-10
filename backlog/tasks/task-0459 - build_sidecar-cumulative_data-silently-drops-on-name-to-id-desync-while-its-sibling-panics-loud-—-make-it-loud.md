---
id: TASK-0459
title: >-
  build_sidecar: cumulative_data silently drops on name-to-id desync while its
  sibling panics loud — make it loud
status: Done
assignee: []
created_date: '2026-06-09 22:00'
updated_date: '2026-06-10 10:50'
labels:
  - silent-sibling
  - compiler
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P3.11). build_sidecar treats the SAME name<->id desync invariant two different ways: data_decl_order fails loud (sidecar.rs:858-866) while cumulative_data filter_maps the desync away silently (sidecar.rs:837-840). A silently-dropped cumulative symbol would skip the COPY-not-accumulate exclusion — the xN-double-count protection that is value-correctness-load-bearing (see the 16-jacobi cumulative-array memory: whole-array accumulate was xN-wrong for cumulative cross-iteration state until the discriminator landed).

Recurring classes: feedback-silent-sibling-defect + feedback-option-none-skip-arm-silent-drop. Work: make the cumulative_data path fail loud identically to its sibling; add a unit test constructing the desync; then audit the rest of build_sidecar for further filter_map / skip-arm siblings of the same invariant and fix or justify each.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 cumulative_data desync is loud (typed error or panic matching the data_decl_order treatment), never a silent drop
- [x] #2 Unit test constructs the desync and pins the loud outcome
- [x] #3 build_sidecar audited for sibling skip-arms; each fixed or justified in notes
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Plan:
1. Re-grep cumulative_data filter_map + data_decl_order panic sibling + (a)/(d) name->id blocks in build_sidecar (line numbers are review-dated).
2. Make cumulative_data desync fail loud IDENTICALLY in kind to data_decl_order sibling (panic-with-context; collect_cumulative_data_names returns names, name_data is the lookup table — same .unwrap_or_else(panic!) mechanism). Do not invent a third mechanism.
3. Add unit test (inline mod or new tests/sidecar_build_desync.rs) constructing the name<->id desync and pinning the loud outcome.
4. Audit EVERY map built in build_sidecar: enumerate (a)..(l), record each one whose construction crosses the name<->id boundary and its desync disposition (panic / filter_map-justified). Full disposition list in task notes.
5. Verify: cargo test -p nucleus-compiler --lib + new test; cargo clippy -p nucleus-compiler --all-targets -D warnings.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-WORK (TASK-0459):

FIX: build_sidecar block (k) cumulative_data (nucleus-compiler/src/sidecar.rs) changed from a SILENT filter_map(name_data.get) drop to a LOUD map(...).unwrap_or_else(panic!) with context, IDENTICAL in kind to sibling (j) data_decl_order. Panic msg contains "cumulative<->id table desync". Did NOT invent a third mechanism: the desync guards are documented (build_sidecar docstring) as panics because they are unreachable for link-valid IR; SidecarError is reserved for the valid-Nuc same-name-diff-bounds case, so a typed error would have been the wrong surface here. A cumulative name is ALWAYS a declared data LHS => must be in name_data => absence is internal desync (same totality argument as (a)/(d)/(j)).

TEST: new owned tests/sidecar_build_desync.rs (2 tests, both green):
 - jacobi_field_is_cumulative_and_carried_in_clean_build (positive control: field IS cumulative + clean build OK, so the desync test is non-vacuous)
 - cumulative_data_name_id_desync_panics_loud (#[should_panic(expected="cumulative<->id table desync")]; removes field from acfg.name_data on a real 16-jacobi pipeline build, asserts loud panic)

STRUCTURAL AUDIT of EVERY map built in build_sidecar (denominator re-derived by reading the (a)..(l) blocks, not trusting review line numbers):
 (a) data_types          name->id (name_data->algo.data)   LOUD panic   OK
 (b) consts              NO id crossing (name-keyed copy)   n/a
 (c) loop_bounds         name->IterVar (name_iter_vars)     LOUD panic on miss in collect_loop_bounds (collectors.rs ~270); the match out.get(iv) at ~280 is a DIFFERENT invariant (same-name-diff-bounds) surfaced as typed SidecarError -- correct
 (d) kernel_sigs         name->KernelId (name_kernels)      LOUD panic   OK
 (e) partition_worker_ranges  id-keyed clone                n/a
 (f) transfer_buffer_for_seq  SeqTag-keyed (ACFG walk)      n/a
 (f) transfer_transport_for_seq SeqTag-keyed                n/a
 (g) halo_widths         id-keyed clone                     n/a
 (h) reuse_widths        id-keyed clone                     n/a
 (i) partition_pairs     id-keyed clone                     n/a
 (j-grid) grid_shape_for_outer_iv id-keyed clone            n/a
 (k) cumulative_data     name->DataId (name_data)           WAS silent filter_map -> NOW LOUD panic (THIS FIX)
 (j) data_decl_order     name->DataId (name_data)           LOUD panic   OK (the named sibling)
 (l) combine_for_data    name->DataId in collect_combine_for_accumulators (collectors.rs ~66, name_data.get inside if-let-Some) = filter_map. JUSTIFIED (NOT a sibling defect): an accumulator-shape source walk can name a symbol the partition pass ELIDED from name_data (legitimately absent, unlike a declared cumulative LHS); the downstream gate RE-DERIVES the accumulator set (backend-common::multi_worker_walker::collect_accumulate_waits, collect.rs ~235) and reports the missing-combine reject with full context (backend-common render/error.rs ~33). Justification is grep-witnessed, not just docstring-claimed. Cross-referenced from the new (k) comment so the (k)-vs-(l) distinction is explicit.

CONCLUSION: 6 maps cross the name->id boundary: (a)(c)(d)(j)(k)(l). (a)(c)(d)(j) already loud; (k) fixed loud; (l) filter_map justified+witnessed. No remaining silent name<->id drop in build_sidecar.

VERIFY: cargo test -p nucleus-compiler --lib = 192 passed/0 failed; cargo test --test sidecar_build_desync = 2 passed; clippy -p nucleus-compiler --lib --test sidecar_build_desync -D warnings clean.

GOTCHA (not mine to fix): cargo clippy --all-targets on nucleus-compiler currently FAILS to compile tests/capabilities.rs (E0063 missing host_data_relay/reorderable_push/star_topology_host_mediation on Capabilities) -- a CONCURRENT foreign edit (TASK-0455.09 capabilities work in this same tree); outside my ownership, unrelated to the sidecar path. Scoped clippy on my owned targets is clean.

TOUCHED (mine): nucleus/nucleus-compiler/src/sidecar.rs (block (k) only) + new nucleus/nucleus-compiler/tests/sidecar_build_desync.rs. Left In Progress for the batched gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
cumulative_data (build_sidecar block k) now fails loud on name<->id desync, same mechanism as the data_decl_order sibling; silent filter_map gone. Full (a)..(l)+(m relabel) audit: 6 name->id maps, 5 loud, (l) combine_for_data justified with grep-witness (downstream check_accumulator_consistency re-derives + EmitError reject; verified by architect review). New should_panic pin on real 16-jacobi build + non-vacuous positive control. Landed ca270c8 (+6ac4bb9 relabel); architect GO; wave gate 2938/0, e2e baseline held.
<!-- SECTION:FINAL_SUMMARY:END -->
