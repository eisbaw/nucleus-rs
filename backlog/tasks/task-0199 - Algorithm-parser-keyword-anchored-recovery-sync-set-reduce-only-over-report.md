---
id: TASK-0199
title: >-
  Algorithm parser: keyword-anchored recovery sync set (reduce ;-only
  over-report)
status: To Do
assignee: []
created_date: '2026-05-19 19:44'
updated_date: '2026-05-19 19:58'
labels:
  - compiler
  - language
  - follow-up
dependencies:
  - TASK-0080
  - TASK-0081
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0080/0081. The algo parser recovers at the ; terminator only (chumsky 0.9 skip_until([;]).consume_end()). Bounded + deterministic + correct-multi-error, but coarse: a malformed item whose failure is structural-at-EOF, or a broken stmt inside a for { ... } body, produces UP TO TWO bounded secondary follow-on errors beyond the primary — a stray } (after the inner ; is consumed) AND/OR a trailing UnexpectedEof; for a malformed item in a for{} body near EOF BOTH can appear simultaneously (3 errors total). [Count corrected by the TASK-0080/0081 review gate: the original "ONE follow-on" was an undercount; max is two.] This is real, deterministic, does NOT scale with program size (valid trailing code absorbs the recovery; the realistic typo+valid-tail case is exactly 1), and the primary error is always correct — acceptable for a research compiler, NOT a cascade. Refine by anchoring the recovery sync set on item-start keywords (kernel/data/const/for) as well as ;, so recovery resyncs at the next ITEM rather than the next semicolon. Referenced by code comments in compiler/src/algo/parser.rs (program_parser doc + module Known-limitations). Also (low-pri, from the same review): consider pub(crate)/smart-constructor for error::ParseErrors.0 so the non-empty invariant is unbreakable in-crate (currently enforced only by the single map_all_chumsky_errors constructor).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Recovery sync set is anchored on item-start keywords (kernel/data/const/for) in addition to ; so recovery resyncs at the next ITEM boundary, not the next inner semicolon
- [ ] #2 A malformed stmt inside a for{} body near EOF produces ONLY the primary error (no stray-} / no trailing-UnexpectedEof follow-on) — pinned by a regression test that asserts exactly 1 error for that shape; the realistic typo+valid-tail case stays exactly 1
- [ ] #3 Multi-error reporting for genuinely-independent errors is preserved (>=2 distinct errors still reported with correct per-error line:col); recovery stays bounded + deterministic (same source -> identical error set+order)
- [ ] #4 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci); zero behaviour change for valid input
<!-- AC:END -->
