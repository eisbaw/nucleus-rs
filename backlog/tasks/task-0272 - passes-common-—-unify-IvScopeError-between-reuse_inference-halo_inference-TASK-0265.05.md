---
id: TASK-0272
title: >-
  passes::common — unify IvScopeError between reuse_inference + halo_inference
  (TASK-0265.05)
status: To Do
assignee: []
created_date: '2026-05-24 08:33'
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
