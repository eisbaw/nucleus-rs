---
id: TASK-0272
title: >-
  passes::common — unify IvScopeError between reuse_inference + halo_inference
  (TASK-0265.05)
status: Done
assignee: []
created_date: '2026-05-24 08:33'
updated_date: '2026-05-24 10:56'
labels:
  - M5
  - passes
  - refactor
  - forward-carried-from-TASK-0265
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0265 cycle 87 — review item 2 of 5.

reuse_inference uses ReuseInferenceError::UnknownLoopVar { var }. halo_inference uses HaloInferenceError::UnknownIterVarInScope { iter_var }. Both name the SAME shape: a link-pass invariant violation (an iv referenced by a directive is not in ACFG::name_iter_vars). The two variants exist in parallel because the passes landed cycles apart.

When Stage 2 of both passes lands (TASK-0265 + TASK-0263, both partly done), a unified driver may want to display either pass's diagnostic via one shared variant — passes::common::IvScopeError. The lift would:
- Rename HaloInferenceError::UnknownIterVarInScope -> IvScopeError + re-export wrapper.
- Rename ReuseInferenceError::UnknownLoopVar -> IvScopeError + re-export wrapper.
- Existing call sites + tests update to match.

LOW PRIORITY — cosmetic / consistency. Not urgent; can wait until a third pass needs the same shape. Filed so it does not get forgotten across stateless subagent boundaries.

## AC
1. Decide whether to lift (vs leave as-is for low ROI).
2. If lifted: shared variant in passes::common, both passes re-export, defensive tests updated.
3. cargo test --workspace stays GREEN; cargo clippy stays clean.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-94 SCOPE-REFINEMENT (orchestrator, 2026-05-24): the cycle-87 brief framed this as a 2-pass cosmetic (reuse_inference vs halo_inference). Cycle-94 grep reveals the picture is bigger:

\`UnknownLoopVar\` (or close-synonym) appears in SIX passes:
1. partition_workers::PartitionError::UnknownLoopVar
2. partition_blocks2d::PartitionBlocks2dError::UnknownLoopVar
3. block_transform::BlockTransformError::UnknownLoopVar
4. partition_rows::PartitionRowsError::UnknownLoopVar
5. reuse_inference::ReuseInferenceError::UnknownLoopVar
6. halo_inference::HaloInferenceError::UnknownIterVarInScope (the differently-named twin)

So the actual choice is:
- **A (minimum)**: rename halo's UnknownIterVarInScope → UnknownLoopVar to match the other 5. Single-file rename + Display + tests. Probably 1-cycle scope.
- **B (lift)**: extract shared \`passes::common::IvScopeError\` (file exists at nucleus/nucleus-compiler/src/passes/common.rs); refactor all 6 enums to wrap/re-export. Real cross-pass refactor.

The cycle-87 LOW priority labelling assumed scope A. Scope B is genuinely a multi-day refactor.

Recommendation: do scope A in a single fresh cycle with thorough grep discipline (per the cycle-93 reliability signal — \`UnknownLoopVar\` appears in many places and a half-done rename would mirror cycle 93's defect). Defer scope B until a third pass needs the variant OR the existing 6 passes accumulate enough diagnostic-formatting drift to justify centralisation.

STAYS To Do at LOW priority. The scope-A path is the precise next-implementer cycle.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 95, commit f8a3267 + ecf191b (cycle-95 review-hardening to be added): scope-A LANDED. Halo's UnknownIterVarInScope renamed to UnknownLoopVar matching the convention of 5 sibling passes (partition_workers, partition_blocks2d, partition_rows, block_transform, reuse_inference). 3 sites changed in halo_inference.rs + 1 doc-cross-ref in reuse_inference.rs. Gate: cargo build + just test (0 failed) + just e2e 92/77/0/15/0 + just determinism-check green + clippy -D warnings clean.

Scope-B (lift to passes::common::IvScopeError shared across all 6 passes) explicitly DEFERRED per cycle-94 scope-refinement: real cross-pass refactor warranting fresh session. Refile as separate task if/when the variant duplication accumulates enough drift to justify the lift (the 6 passes currently agree on shape; the principled fix has clear cost but no urgent forcing function).

CYCLE-95 REVIEW-HARDENING (architect NO-GO repeat of cycle-93 sibling-grep failure mode): the in-source rename was clean (zero residual references in nucleus/) BUT my commit-message claim of "full-tree grep" missed 4 forward-carry references in backlog/. Architect's parallel review caught it — exactly the cycle-93 lesson firing TWICE. Cycle-95 closure notes appended to TASK-0260, TASK-0261, TASK-0265 marking those stale references as resolved-via-scope-A (commits to follow).

The recurring pattern is now confirmed: the loop's "review-gate-caught-overconfidence" stop signal has fired in TWO consecutive cycles (93 + 95) on the same sibling-grep failure mode. This is a HARD stop signal for the session. Future sessions should treat any "comprehensive grep" claim with heightened scrutiny and scope grep beyond src/ explicitly.

Defensive-test gap for HaloInferenceError::UnknownLoopVar (no direct test pin; sibling exists for ReuseInferenceError at sidecar_reuse.rs:381) is filed in TASK-0265's notes as still-open; could become its own follow-up if a future cycle wants to close it.
<!-- SECTION:FINAL_SUMMARY:END -->
