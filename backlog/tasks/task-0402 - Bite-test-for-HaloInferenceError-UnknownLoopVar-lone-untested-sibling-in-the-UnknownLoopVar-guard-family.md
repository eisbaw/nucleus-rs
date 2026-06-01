---
id: TASK-0402
title: >-
  Bite test for HaloInferenceError::UnknownLoopVar (lone untested sibling in the
  UnknownLoopVar guard family)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-01 05:13'
updated_date: '2026-06-01 05:13'
labels:
  - hardening
  - testing
  - prove-the-check-bites
  - silent-sibling
  - cycle-236
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 prove-the-check-bites hardening, sibling-completion of TASK-0400 (partition-pass UnknownLoopVar x3) + TASK-0397 (halo UnknownKernelInCall white-box).

GAP: halo_inference.rs:1437 constructs HaloInferenceError::UnknownLoopVar when ctx.name_iter_vars.get(iv_name) misses after the affine/coeff checks pass. A workspace-wide audit (cycle-236) of every compiler-pass error enum vs its bite-test coverage found this is the ONE remaining genuinely-reachable variant with ZERO test coverage anywhere (neither the inline cfg-test mod nor tests/sidecar_halo.rs). Every SIBLING UnknownLoopVar is bite-tested: reuse_inference (sidecar_reuse.rs:381), partition_workers/partition_rows/partition_blocks2d (TASK-0400). Textbook feedback-silent-sibling-defect: the lone untested member of a guard family.

REACHABILITY: white-box defensive guard (doc at 1428-1434 says it cannot happen for link-valid IR; exists so an inconsistently-constructed (LinkedIR, ACFG) pair fails closed with a typed error rather than panicking). Same class as the reuse UnknownLoopVar sibling.

TRIGGER PATH (verified by reading 1355-1450): body for y { out[y] gets K(grid[y+1]) } => scope=[y], single iv, affine coeff 1 (passes StridedAccessNotSupported at 1411), then name_iter_vars.get(y) must be None. Poison: build ACFG via build_acfg (name_iter_vars carries y), then acfg.name_iter_vars.remove(y) before apply_halo_inference. The pass collects the iv from the body For-loop scope (LinkedIR side), NOT from name_iter_vars, so the site is still reached.

TEMPLATE: reuse sibling sidecar_reuse.rs:357-386 + halo inline harness build_linked / build_acfg_and_apply (halo_inference.rs:1526/1641). Add to the inline cfg-test mod in halo_inference.rs where unknown_kernel_in_call_guard_bites_whitebox (2879) lives.

POLICY: keep typed error (panic-not-diagnostic, PRD 10); do NOT convert to unreachable. Mutation-prove it bites before done.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a white-box bite test to the inline #[cfg(test)] mod in halo_inference.rs (sibling to unknown_kernel_in_call_guard_bites_whitebox).
2. Construct body: for y in 1..15 { out[y] <-- K(grid[y+1]) } via build_linked. Build ACFG via build_acfg. Poison: acfg.name_iter_vars.remove("y"). Call apply_halo_inference(&linked, acfg).
3. Assert Err(HaloInferenceError::UnknownLoopVar { var }) with var == "y".
4. Mutation-prove: confirm test fails if the remove() is omitted (guard would not fire) and if the asserted var name is wrong.
5. Gate: nix develop -c just build && just clippy && just test && just test-release && just e2e (test and test-release SEQUENTIALLY per cycle-235 gate footgun). e2e baseline 385/328/0/57/0 must hold.
6. Parallel read-only review: qa-test-runner + mped-architect on the commit range.
7. Commit: nucleus-compiler: TASK-0402 ...
<!-- SECTION:PLAN:END -->
