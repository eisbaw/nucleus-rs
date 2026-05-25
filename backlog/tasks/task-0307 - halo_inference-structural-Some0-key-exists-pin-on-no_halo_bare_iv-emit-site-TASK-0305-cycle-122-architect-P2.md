---
id: TASK-0307
title: >-
  halo_inference: structural Some(0) key-exists pin on no_halo_bare_iv emit site
  (TASK-0305 cycle-122 architect P2)
status: To Do
assignee: []
created_date: '2026-05-25 04:05'
labels:
  - M5
  - compiler
  - test-coverage
  - halo_inference
  - contract-pin
  - forward-carried-from-TASK-0305
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0305 (cycle 122) decided Option B (preserve halo_inference's `absent ≡ explicit-0` contract degree of freedom). The architect's review-gate flagged a real coverage gap: NO existing test asserts `halo_widths[K][iv] == Some(0)` for a bare-iv access. Both `no_halo_bare_iv` (in-module test) and `elementwise_add_records_only_zero_halos` use `.unwrap_or(0)` patterns — vacuous-tolerant under silent-skip. The `stencil_3x3_produces_halo_one_on_both_axes` test uses `Some(1)` (key-exists) but only on non-zero offsets.

This means a future walker regression that silently emits NO entries for a bare-iv access would NOT be caught by any current test. The narrative pins (task0299_*, task0303_*) would pass vacuously.

## Acceptance criteria

1. Add a single one-line structural pin in the in-module tests of `nucleus/nucleus-compiler/src/passes/halo_inference.rs` near `no_halo_bare_iv` (line ~1199):
   ```
   assert_eq!(acfg.halo_widths.get(&k_id).and_then(|m| m.get(&iv_id)).copied(), Some(0));
   ```
2. The pin must fail LOUD if the production `record_halo` walker (at the `per_iv.entry(iv).or_insert(0)` line in halo_inference.rs) is silently regressed to omit entries for inspected bare-iv accesses.
3. Update the cross-references in halo_inference.rs:53-71 and sidecar_halo.rs's task0303_07 comment to point at the new contract-form sentinel test (replacing the current `record_halo` text search hint).

## Honest scope

LOW priority. The vacuous-pass risk is judged unlikely (today's walker DOES always emit explicit-0). This task is a defensive sentinel — single-line pin, no contract change. Compatible with the Option B decision.

## Cross-references

- TASK-0305 (cycle 122) — the Option B decision this defends.
- halo_inference.rs:53-71 — the contract paragraph (Option B project decision marker).
- halo_inference.rs:848 / `record_halo` — the emit site whose silent-skip the pin would catch.
- halo_inference.rs:1199 — `no_halo_bare_iv` in-module test (the location where the pin should land).
<!-- SECTION:DESCRIPTION:END -->
