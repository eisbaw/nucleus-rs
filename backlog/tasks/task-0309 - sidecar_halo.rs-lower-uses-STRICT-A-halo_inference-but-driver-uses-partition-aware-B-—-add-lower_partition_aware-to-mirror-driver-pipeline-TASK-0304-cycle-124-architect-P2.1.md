---
id: TASK-0309
title: >-
  sidecar_halo.rs::lower() uses STRICT-A halo_inference but driver uses
  partition-aware-B — add lower_partition_aware() to mirror driver pipeline
  (TASK-0304 cycle-124 architect P2.1)
status: To Do
assignee: []
created_date: '2026-05-25 05:03'
labels:
  - M5
  - test-coverage
  - halo_inference
  - driver-divergence
  - forward-carried-from-TASK-0304
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0304 cycle-124 architect review-gate (qa-test-runner + mped-architect) flagged a pre-existing divergence inherited by the new tests:

- The test helper at `nucleus/nucleus-compiler/tests/sidecar_halo.rs:46-68 lower()` calls `apply_halo_inference` (strict-A variant: fail-fast on any non-affine / strided / data-dependent index).
- The driver at `nucleus/driver/src/main.rs:396` calls `apply_halo_inference_partition_aware` (variant B: fatal only when the offending iv carries a Partition directive; otherwise recorded as advisory and lowering proceeds).

For shipped distributed schedules in M5 (05-stencil + 06-separable-filter + 07-matmul) the two variants AGREE on the halo_widths map because the inputs are fully affine; so the existing `task0299_*` / `task0303_*` / new `task0304_*` tests pass under both. But the tests do NOT exercise the SAME pipeline the driver does — a future regression that manifests only under the partition-aware-B path would slip through.

## Acceptance criteria

1. Either:
   - Migrate `sidecar_halo.rs::lower()` to call `apply_halo_inference_partition_aware` (potentially breaking the existing TASK-0275 strict-failure tests at lines 359-589; those need a separate idiom).
   - Or add a sibling `lower_partition_aware()` helper that mirrors the driver pipeline and migrate `task0299_*` / `task0303_*` / `task0304_*` to use it.
2. The TASK-0275 in-module tests for strict-A error behaviour (`task0275_partition_aware_rejects_*` + `task0275_partition_aware_accepts_*`) MUST continue to use the strict-A helper. Hint: a `lower()` + `lower_partition_aware()` split keeps each test idiom unambiguous.
3. e2e baseline 108/92/0/16/0 preserved (no production-code change; this is test-pipeline alignment).

## Honest scope

LOW priority. The divergence is currently observationally inert (the two variants agree on every shipped fixture). Filing because the cycle-119 memory `feedback-implementer-disclosure-mechanism-wrong` warned about exactly this kind of pipeline-divergence between test and driver, and a future M6+ schedule may exercise the partition-aware-B path differently.

## Cross-references

- TASK-0304 cycle 124 architect review-gate P2.1.
- Memory: `feedback-implementer-disclosure-mechanism-wrong` (cycle 119 — orchestrator note claimed driver uses strict-A; the actual code at driver/main.rs:396 uses partition-aware-B; the lesson includes the test-vs-driver divergence vector).
- `nucleus/nucleus-compiler/src/passes/halo_inference.rs` (module-doc section "## Strict vs advisory vs partition-policy-aware entry points"; search for that heading) — the contract paragraph documenting the 3 entry points (strict-A, advisory, partition-aware-B).
<!-- SECTION:DESCRIPTION:END -->
