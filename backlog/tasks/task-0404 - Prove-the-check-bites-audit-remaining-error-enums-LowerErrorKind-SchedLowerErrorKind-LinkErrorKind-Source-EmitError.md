---
id: TASK-0404
title: >-
  Prove-the-check-bites audit: remaining error enums (LowerErrorKind,
  SchedLowerErrorKind, LinkErrorKind/Source, EmitError)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 05:55'
updated_date: '2026-06-01 06:41'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DELIVERED (commit 95d4240): 3 bite tests in pthreads-sync/tests/emit.rs for the only untested EmitError variants -- the I/O wrappers KernelsReadFailed (missing kernels path -> NotFound), OutputCreateFailed (out_dir under a regular file -> create_dir_all fails), WriteFailed (out_dir/Cargo.toml pre-created as a directory -> first fs::write fails). Deterministic + portable (structural triggers, no permission-bit reliance -> fire even as root). EmitError is Debug-only (io::Error not PartialEq) so each matches the variant pattern. All 3 mutation-proven (invert trigger -> test fails at expect_err lines 431/462/489). qa GO (gate re-run: build/clippy/1234dev/1233rel/e2e 385/328/0/57/0 x3 stable) + architect GO.

AUDIT RESULT (the real deliverable -- prove-the-check-bites across the non-pass error enums):
- LowerErrorKind: 19/19 bite-tested (tests/algo_lower.rs).
- LinkErrorKind: 8/8 bite-tested (tests/link.rs). LinkErrorSource: 2-tag discriminant (Schedule/Algorithm), N/A.
- SchedLowerErrorKind: 27/28 bite-tested (tests/sched_lower.rs). The 28th, UnsupportedPartitionKind, is UNREACHABLE-BY-CONSTRUCTION (never constructed; TASK-0258/0259 gave all 3 PartitionKind variants real consumers; documented sched/lower.rs:160 [bite-freq table = never] + 1097). N/A-dead, same class as InnerRepeatNotFound (TASK-0401). No bite test possible/needed.
- EmitError: was 3/6 (UnsupportedFeature/ContractGap/AccumulatorShapeMismatch covered); now 6/6 with this cycle.
CONCLUSION: the prove-the-check-bites wave across ALL error enums (passes + lowering + sched + link + emit) is SATURATED. Every genuinely-reachable typed error variant in the compiler has a bite test; the only uncovered variants are unreachable-by-construction (documented) or N/A discriminants.

SILENT-SIBLING DISCHARGE: the 3 fs op classes recur in every backend (embedded-pattern has 6+ sites across lib+bin). Grep audit: ALL use .map_err(EmitError::...); NONE uses unwrap/expect/?-on-raw-io -> no backend panics on I/O failure. pthreads-sync (canonical) proves the variants at VARIANT granularity; per-backend SITE-granularity sweep = optional TASK-0405 (low; mechanical duplication).

HONEST-FAILURE DISCLOSURE (architect P1, corrected not hidden): my first-pass audit claimed SchedLowerErrorKind 19/19 -- WRONG on both numbers. The enum has 28 variants (the awk range 421..560 truncated it at line 560; the 19 was a coincidental match to LowerErrorKind's count). Architect independently re-counted (28, span 421..678) and found UnsupportedPartitionKind untested; I then verified the other 8 missed variants ARE tested and that UnsupportedPartitionKind is genuinely dead. The saturation conclusion survived but only after honest reclassification. LESSON (feed-forward to TASK-0405 + any future enum audit): when counting enum variants with awk/sed, bound the scan by the enum's closing brace -- grep the `pub enum` start line, then the next `}` at column 0 -- NOT a guessed line range; a too-narrow window silently truncates and a coincidental count match hides it. Mirrors the TASK-0402 lesson (Explore agent missed inline test mods): coverage/inventory audits are HIGH-risk for silent under-counting; always re-derive the denominator from a structural boundary, never trust a guessed window.
<!-- SECTION:FINAL_SUMMARY:END -->
