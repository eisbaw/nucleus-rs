---
id: TASK-0404
title: >-
  Prove-the-check-bites audit: remaining error enums (LowerErrorKind,
  SchedLowerErrorKind, LinkErrorKind/Source, EmitError)
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 05:55'
labels:
  - hardening
  - testing
  - prove-the-check-bites
  - dead-code-audit
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 next-keystone, forward-carried from TASK-0402. The pass-error-enum prove-the-check-bites sub-wave is SATURATED (TASK-0400/0401/0402: every genuinely-reachable typed variant in the passes/ error enums has a bite test). This task extends the SAME discipline to the remaining error surfaces NOT yet systematically audited:
- LowerErrorKind (algo/ir.rs:319) -- partially worked by TASK-0396/0397/0398 (ConstOverflow/ShapeOverflow/NonConstLoopBound); audit the FULL variant set for residual gaps.
- SchedLowerErrorKind (sched/ir.rs:421) -- not audited.
- LinkErrorKind (link/errors.rs:52) + LinkErrorSource (link/errors.rs:238) -- not audited.
- EmitError (backend-common/render/error.rs:16) -- not audited; the 7 backends are its consumers.
- parser ParseError -- TASK-0399 fuzz-tested panic-freedom/non-empty/determinism but per-VARIANT bite coverage not confirmed.

METHOD (load-bearing lesson from TASK-0402): a coverage audit MUST grep BOTH the source-file inline #[cfg(test)] mods AND tests/ dirs. The TASK-0402 Explore pass scanned only tests/ and over-reported the halo/reuse gap by 11 variants (it missed ~1400 LoC of inline tests in halo_inference.rs / ~800 in reuse_inference.rs). Real gap was 1. Re-grep source inline mods before trusting any gap claim.

For each variant: classify BITE-TESTED / POSITIVE-ONLY / UNTESTED / UNREACHABLE-BY-DESIGN (at VARIANT and SITE granularity per TASK-0397 lesson). For each genuine gap: either add a bite test (input-driven if reachable from source; white-box poison if a defensive link-invariant, per TASK-0402 template) OR prove unreachable-by-construction + documented note (per TASK-0401 InnerRepeatNotFound template). Keep typed errors typed (panic-not-diagnostic). Mutation-prove each new test bites. Expect MOST to already be covered -- the deliverable is the saturation proof + any residual fills, not a presumed large gap.
<!-- SECTION:DESCRIPTION:END -->
