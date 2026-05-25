---
id: TASK-0317
title: >-
  Silent-bypass guard for rewrite_partition_tiles_inner default-order fall-back
  (TASK-0315 silent-sibling follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 09:55'
updated_date: '2026-05-25 10:14'
labels:
  - compiler
  - hardening
  - transfer_inject
  - silent-sibling
  - forward-carried-from-TASK-0315
dependencies:
  - TASK-0315
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle 134 (TASK-0315) added a `nuc_trace!` diagnostic on the default-order fall-back branch of `order_halo_strip_bounds_by_data_dim` in `nucleus/nucleus-compiler/src/passes/transfer_inject.rs` to surface axis-mapping-defence bypass on synthetic fixtures.

While reviewing TASK-0315 for silent-sibling defects per `feedback-silent-sibling-defect`, the orchestrator identified a structurally-identical fall-back pattern at `rewrite_partition_tiles_inner` (transfer_inject.rs line ~1731):

```rust
let bounds = match compute_partition_bounds_with_dim_prefix(...) {
    Some(b) => b,
    None => {
        // Pre-TASK-0301 fall-back: iterate the partition_axis_order
        // ... synthetic fixtures + bare-aggregate-only data.
        ...
    }
};
```

Same risk class as TASK-0315: when `data_dim_iv_map` does not carry an entry for the data symbol (no observed indexed accesses), the code falls back to a nest-order emit that bypasses the TASK-0301 axis-mapping defence. Production callers always observe accesses; synthetic test fixtures built via `DataflowEdge::new` reach the fall-back silently.

## Acceptance criteria

1. Add a `crate::nuc_trace!(...)` log line on the `None` arm of the `compute_partition_bounds_with_dim_prefix` match in `rewrite_partition_tiles_inner` (transfer_inject.rs ~line 1731). Diagnostic should identify the function, the data id, the worker id, and note that the TASK-0301 axis-mapping defence is bypassed on this call (expected only on synthetic fixtures).
2. Verify the trace is byte-silent on `NUC_TRACE` unset (cycle-134 / TASK-0315 has already verified the macro is byte-silent in this crate; this AC is satisfied by re-running e2e + determinism baselines).
3. Optionally: pin the fall-back path with a unit test that builds a synthetic ACFG with no observed accesses and asserts the fall-back returns the nest-order vector.

## Honest scope

LOW priority. Same defensive-observability rationale as TASK-0315. The cycle-115 (TASK-0294) and TASK-0301 axis-mapping defences in `compute_partition_bounds_with_dim_prefix` are correct today; this task adds visibility so a future regression masked by the default-order fall-back is observable in production.

## Forward-carried from TASK-0315 cycle 134 orchestrator inline architecture review

Sibling-grep audit identified `rewrite_partition_tiles_inner` as the only other call site that pattern-matches the cycle-133 axis-mapping defence shape; `order_halo_strip_bounds_by_data_dim` and `compute_partition_bounds_with_dim_prefix` are the two data-dim-aware emit helpers in transfer_inject.rs, both with the same fall-back-on-synthetic-fixtures policy.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 135 implementation summary

Landed by orchestrator inline (memory: feedback-spawned-agents-refuse-code-edits — implement directly, delegate only read-only review). Files touched:
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs:
  - Added crate::nuc_trace!(...) on the None-arm fall-back of rewrite_partition_tiles_inner. Trace fires only on NUC_TRACE; byte-silent default per trace.rs contract. Disambiguates None vs Some(empty) via the entry={absent | Some(empty)} matcher, mirroring cycle-134's identical disambiguator at order_halo_strip_bounds_by_data_dim's fall-back (architect P2-1 fold-back).
  - Added 4 inline unit tests in the existing #[cfg(test)] mod tests block targeting compute_partition_bounds_with_dim_prefix:
    - task0317_canonical_path_returns_dim_ordered_bounds_no_fallback
    - task0317_missing_entry_returns_none_drives_fallback
    - task0317_empty_per_dim_returns_none_drives_fallback
    - task0317_sparse_coverage_drops_to_whole_array
  - Added #[allow(clippy::type_complexity)] on the test-only helper make_partition_ranges (explicit nested slice signature is the table-driven shape the tests need).

## ACs

- AC#1 (nuc_trace! on the None arm of compute_partition_bounds_with_dim_prefix in rewrite_partition_tiles_inner) — GREEN. Smoke-tested: NUC_TRACE=1 cargo test --test transfer_inject -- --nocapture emits the trace correctly on existing synthetic-fixture integration tests (DataflowEdge::new is widely used in tests/transfer_inject.rs).
- AC#2 (byte-silent on NUC_TRACE unset) — GREEN. e2e baseline 108/92/0/16/0 bit-identical (production paths unaffected).
- AC#3 (optionally pin the fall-back path with a unit test) — GREEN via the stronger helper-level pin. The 4 inline tests pin compute_partition_bounds_with_dim_prefix's Option return rather than rewrite_partition_tiles_inner's emit Vec — see Cycle-135 architect P3-1 clarification below.

## Cycle-135 architect P3-1 clarification (AC#3 wording vs shipped artifact)

The original AC#3 text reads: 'Optionally: pin the fall-back path with a unit test that builds a synthetic ACFG with no observed accesses and asserts the fall-back returns the nest-order vector.'

What shipped: 4 unit tests pinning compute_partition_bounds_with_dim_prefix's Option<Vec<...>> return for the 4 arms (canonical-Some / missing-None / empty-per-dim-None / sparse-Some(empty)). This is a strictly STRONGER pin than the AC text: helper-level coverage is durable across caller refactors that may change how rewrite_partition_tiles_inner emits the nest-order vector, whereas a caller-level Vec assertion would couple the test to today's specific loop shape.

The implementation choice was correct (per architect P3-1 read: 'helper-level is more durable across caller refactors' + test docs explicitly justify the choice). The AC text is now slightly stale but the spirit (defensive coverage of the fall-back path) is satisfied. Per feedback-ac-rewrite-on-done-task discipline, not rewriting the AC text retroactively — recording the clarification here for tracker hygiene.

## Cycle-135 architect P3-2 (coverage gap — mention-only, NOT a follow-up)

The 4 inline tests do NOT pin:
(a) the helper's ambiguous-multi-iv-per-dim → Some(Vec::new()) branch (transfer_inject.rs ~line 1982-1986)
(b) the worker-missing-from-range-map slot collapse path (range.map at ~line 1980 followed by hole-treatment at ~line 1996)

Architect P3-2 explicitly recommends mention-only: 'Neither is on the cycle-135 critical path; (a) is documented as defensive in the helper docs; (b) is operationally hard to reach. Mention-only — promote to a follow-up if a future cycle touches this helper again.' Recorded here for the next-cycle audit when that helper is touched.

## Gate (orchestrator, cycle 135)

- just build: clean
- just clippy: clean (0 warnings)
- just test (dev): 870 / 0 / 3 (+4 new vs cycle-134 baseline 866)
- just test-release: 870 / 0 / 3 (matches dev — no debug_assert! skew)
- just e2e: 108 / 92 / 0 / 16 / 0 required-fail (exact baseline preserved)
- just check-textual-replace-on-codegen: OK
- just check-include-str-coverage: OK
- NUC_TRACE=1 smoke: trace emits expected diagnostic with entry={Some(empty)|absent} disambiguator. Verified Some(empty) firing on real synthetic-fixture integration tests.

## Parallel review gate

Both qa-test-runner + mped-architect returned GO this cycle:
- qa-test-runner: independently reproduced 870/0/3 + clippy clean; 3 P3 findings (style consistency on format-arg capture, trace side-effect not unit-asserted (matches TASK-0315 convention), narrative-claim spot-check). All non-blocking.
- mped-architect: independently reproduced 870/0/3; 1 P2 finding (P2-1, the absent/Some(empty) disambiguator — APPLIED IN-THREAD this cycle); 4 P3 findings (P3-1 AC#3 text-vs-test scope drift — captured above as clarification; P3-2 helper coverage gap — captured above as mention-only; P3-3 clippy::type_complexity placement OK; P3-4 trace identifier prefix consistent). Architect also performed an INDEPENDENT silent-sibling sweep on transfer_inject.rs ('all 4 other x.tile = IterTile::new(...) sites construct from enclosing_tile/halo-extension of an existing tile, NOT from partition_axis_order ... No silent-sibling round-3 found') — confirms cycle 134+135 closed the architecturally-identified pattern class.

API recovery: parallel review subagents both completed successfully this cycle, unlike cycle 134's sustained 529 overload. The feedback-api-overload-during-review-gate memory remains relevant for future occurrences.

## Disposition

DONE. AC#1 GREEN + AC#2 GREEN + AC#3 GREEN-with-clarification; review-gate full parallel GO from both qa-test-runner + mped-architect; P2-1 disambiguator folded in-thread; P3 findings captured here or in follow-ups.
<!-- SECTION:NOTES:END -->
