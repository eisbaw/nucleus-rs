---
id: TASK-0283
title: >-
  Reuse codegen hygiene: lift try_reuse_axis_offset onto affine_decompose
  (TASK-0269 follow-up)
status: Done
assignee: []
created_date: '2026-05-24 15:49'
updated_date: '2026-05-24 17:12'
labels:
  - reuse
  - hygiene
  - follow-up
  - TASK-0269
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P2.5 from TASK-0269 cycle-103 review: nucleus/backend-common/src/render.rs::try_reuse_axis_offset(e, iv_name) at render.rs:1015+ re-implements a subset of nucleus_compiler::passes::reuse_inference::affine_decompose. Both decode 'iv + b' for the same iv, but the render-side helper takes the iv by name (a String comparison on IrExpr::Ident) while the inference-side takes the IterVar directly. The two MUST stay consistent — any future widening of the affine grammar (e.g. constant Mod folding for example 11) needs to be applied in both sites or the codegen rewrite will skip reads that inference accepted.

## Scope (one of two paths)
1. Make passes::reuse_inference::affine_decompose pub (or move it to passes::common alongside the lifted version from TASK-0261 cycle 82); have render.rs call it directly. Need a thin name-to-IterVar shim since render-time only has names.
2. Add a unit test pair in passes::reuse_inference asserting affine_decompose(e, iv) and try_reuse_axis_offset(e, name_of(iv)) produce the same Some/None decision on a representative set of expressions. Cheaper but only catches divergence at test time.

## Acceptance
- Either (1) achieves single source of truth, or (2) adds explicit cross-pass divergence detection.
- Reuse-axis offset decoding has ONE behaviour-defining site OR a regression test that bites if two diverge.

## Honest scope
This is hygiene, NOT a current correctness defect — the two helpers agree today on every shipped fixture. The risk surfaces when (a) the affine grammar widens (TASK-0260's constant-Mod-fold path for example 11), or (b) a new reuse-axis index shape is introduced. File for TASK-0270 cycle or the multi-outer-coord task TASK-0282 (whichever lands first) to address before either materially extends the grammar.

## Dependencies
None (independent hygiene).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## CYCLE-106 LANDING (orchestrator-led, 2026-05-24)

TASK-0283 closed in commit **29ae72c**. Lift  onto  (the same function Stage 1 reuse_inference + halo_inference use). Both stages now share one definition of 'what is iv + b on the reuse axis'; cross-pass divergence is structurally impossible.

### Concrete divergence pre-cycle-106

The cycle-103 inlined re-impl rejected  (Ident-Ident) when STRIDE was a declared const. Stage 1 affine_decompose accepts it (const-folding to coefficient 1, offset STRIDE.value). Result pre-cycle-106: Stage 1 records the slot, marker fires, but Stage 2's narrow matcher silently skips discovery → buffer decl absent → raw body read. Silent codegen mismatch (output still correct because raw read works; the 'reuse codegen happened' claim was structurally false).

### Fix shape

1. Promote  from  to . Re-export at .
2. Rewrite  as a 4-line wrapper: call affine_decompose, filter for coeff == 1.
3. Thread the consts table through discover_reuse_groups → walk_event_for_reuse → walk_arg_for_reuse. Add  helper at the boundary (sidecar carries BTreeMap<String, ConstValue>; affine_decompose wants BTreeMap<String, ResolvedConst>; cheap O(consts.len()) conversion).
4. New test  in pthreads-sync/tests/reuse_marker.rs: builds the exact fixture that triggered the divergence (const STRIDE=1, body reads data[iv + STRIDE]), asserts buffer decl is emitted AND body read is rewritten. Bite-verified: stashing the commit and running the test fails on the buffer-decl assertion with the marker firing but no buffer + raw img_in body read visible.

### Gate post-lift

- cargo test --workspace: 817 / 0 / 3 (+1 vs cycle 105 baseline 816).
- cargo clippy: clean.
- just e2e: 92 / 79 / 0 / 13 / 0 required-fail (preserved).
- just determinism-check: 92 / 79 / 0 / 13 (GREEN).

### Both ACs MET

- AC#1 (SOSO via path 1): MET. affine_decompose is single source of truth.
- AC#2 (regression test that bites): MET via bite-verified test.

Status: Done.

## Cycle-106 notes (re-added with proper escaping; previous append had backtick stripping)

Lift try_reuse_axis_offset (in backend-common/src/render.rs) onto nucleus_compiler::affine_decompose (the same function passes::reuse_inference + passes::halo_inference call). Both stages now share ONE definition of affine 'iv + b' decoding; cross-pass divergence is structurally impossible.

### Concrete divergence pre-cycle-106

Pre-cycle-106 the codegen had an inlined re-impl that rejected the Ident-Ident shape on the RHS of Add (e.g. for x : reuse with body data[x + STRIDE] where const STRIDE = 1 was declared). Stage 1 affine_decompose accepted this via const-folding (coefficient 1, offset STRIDE.value); Stage 2's narrow matcher silently rejected it. Result: marker fires, buffer decl skipped, body read stays raw. Silent codegen mismatch (output still correct because raw read works; the 'reuse codegen happened' claim was structurally false).

### Fix shape

1. Promote passes::common::affine_decompose from pub(crate) to pub. Re-export at nucleus_compiler::affine_decompose.
2. Rewrite try_reuse_axis_offset as a 4-line wrapper: call affine_decompose, filter for coeff == 1.
3. Thread the consts table through discover_reuse_groups → walk_event_for_reuse → walk_arg_for_reuse. New helper sidecar_consts_to_resolved at the boundary (sidecar carries BTreeMap<String, ConstValue>; affine_decompose wants BTreeMap<String, ResolvedConst>; cheap O(consts.len()) conversion).
4. New regression test 'codegen_recognises_const_named_offset_via_affine_decompose' in pthreads-sync/tests/reuse_marker.rs builds the exact fixture that triggered the divergence (const STRIDE=1, body reads data[iv + STRIDE]). Asserts buffer decl emitted AND body read rewritten. Bite-verified by stashing the commit and re-running: fails on the buffer-decl assertion with the marker firing but no buffer + raw img_in body read visible.

### Gate post-lift

- cargo test --workspace: 817 / 0 / 3 (+1 vs cycle 105 baseline 816).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 92 / 79 / 0 / 13 / 0 required-fail (preserved).
- just determinism-check: 92 / 79 / 0 / 13 (GREEN).

### ACs MET

- Single source of truth on affine 'iv + b' decoding: MET via path (1) — affine_decompose is now called from both inference and codegen.
- Regression test that bites if two diverge: MET via the new bite-verified test in pthreads-sync/tests/reuse_marker.rs.

Status: Done. Commit: 29ae72c.
<!-- SECTION:NOTES:END -->
