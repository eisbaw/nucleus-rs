---
id: TASK-0346
title: >-
  Extend e2e milestone enum from M0..M6 to M0..M11 (TASK-0054 cycle-201
  architect P3.4 follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-27 11:27'
updated_date: '2026-05-28 04:23'
labels:
  - e2e
  - validation
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect cycle-201 P3.4: nucleus/e2e/src/main.rs:194-207 clamps the [[required]]/[[skip]] entry's 'milestone' field to the M0..M6 tier-1 range. Per the PRD §11 milestone enum, M7..M11 are valid future milestones (M7 MPI blocking, M8 MPI non-blocking, M9 embedded skeleton, M10 STM32H7 Renode, M11 multi-MCU Renode).

The TASK-0054 cycle-201 [[skip]] entries for embedded_multimcu × 7 backends had to use milestone="M6" with the M11-deferred reason inline (clamp made literal "M11" tag rejected). This makes 'what's M11-deferred' non-greppable on the milestone field.

Fix: extend the parser at nucleus/e2e/src/main.rs:194-207 to accept M0..M11 (or the full PRD §11 milestone range), update the harness's --milestone filter to handle the wider range correctly, and update the e2e-matrix.toml [[skip]] entries that were workaround-tagged with M6 (currently the 7 embedded_multimcu cells) to use their real M11 tag.

Priority: LOW. Standalone follow-up, not specifically a cycle-201 fold-back but discovered during cycle-201.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 228 plan (orchestrator-direct)
Extend e2e milestone enum M0..M6 -> M0..M11 (PRD §11: M7 MPI-block, M8 MPI-nonblock, M9 embedded skeleton, M10 STM32H7 Renode, M11 multi-MCU Renode).
1. nucleus/e2e/src/main.rs Milestone::parse: change clamp k>6 -> k>11; rewrite error message from 'tier-1 range M0..M6' to PRD §11 range M0..M11 (note M0..M6 tier-1 / M7..M11 tier-2/3); update parse() + struct doc comments that say 'k=0..=6 for tier-1'.
2. milestone_in_gate / Manifest parse already u8-PartialOrd range-agnostic — no other clamp; verify no other M6 assumption.
3. nucleus/e2e/src/tests.rs: arg_parser_rejects_bad_milestone uses --milestone M9 expecting rejection (BREAKS — M9 now valid). Change the rejection probe to M12 (just above new max) and update err.contains('tier-1 range') to the new stable substring. Add positive boundary pins: M7 and M11 now parse; M12 rejected.
4. nuc-nucleus/e2e-matrix.toml: the 7 14-hearing-aid embedded_multimcu [[skip]] entries carry milestone='M6' with a 'Reinstate at M11' reason (workaround — non-greppable). Change those 7 (and only those) to milestone='M11' via the unique reason-line+milestone pair. Update the M11-deferred prose comment if it cites the M6 tag.
Gate: clippy/test/test-release/e2e; e2e totals MUST stay 280/246/0/34/0 (changing a [[skip]] cell's milestone tag does not change skip status under bare 'just e2e' — no --milestone filter; verify).

## Cycle 228 review gate (parallel read-only) — GO
qa-test-runner (independent): clippy clean; just test no failures; the 6 e2e milestone tests pass (incl. real_manifest_has_no_coverage_gaps + required_counts_strictly_grow which PARSE the real toml with new M11 tags); just test-release 0 fail; just e2e 280/246/0/34/0 unchanged. FUNCTIONAL smoke of widened filter: --milestone M7 runs (266 cells, was rejected pre-change), --milestone M11 runs (273 cells — the 7 M11-retagged embedded_multimcu now eligible), --milestone M12 fails loud with exact 'out of the PRD §11 range M0..M11 (M0..M6 tier-1, M7..M11 tier-2/3)'. GO.
mped-architect (read-only): GO, no P1/P2. Silent-sibling CLEAN — Milestone::parse (k>Self::MAX) is the SINGLE range bound; milestone_in_gate is range-agnostic u8 PartialOrd; no other clamp/loop/array; CLI help strings use M1/M3 as examples only; zero Milestone refs outside the e2e crate. Doc accuracy CLEAN — tier split matches PRD §11 exactly, M11 is correct ceiling, error-text vs test substring 'M0..M11' match (no drift, test passes for the right reason). Toml retag CLEAN — exactly 7, all-and-only embedded_multimcu skips, prose comments consistent. Test boundary CLEAN — M11 last-accepted pinned, M12 rejected in both unit + CLI paths; no monotonicity/coverage break (required max tag = M6, strict-grow loop never reaches M11).
Architect P3 (advisory, non-blocking, NOT introduced here): the two coverage tests iterate hardcoded gate bands assuming top==M6 (real_manifest_has_no_coverage_gaps stops at M4; required_counts_strictly_grow loops 1..=top off [[required]] rows). Correct today (M7..M11 skip-only). Filed as TASK-0359 (dep TASK-0045 M7) to widen off Milestone::MAX when the first M7+ [[required]] cell lands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle 228. Extended the e2e harness milestone enum from M0..M6 to the full PRD §11 range M0..M11 (added Milestone::MAX=11; clamp k>6 -> k>Self::MAX; error message now 'out of the PRD §11 range M0..M11 (M0..M6 tier-1, M7..M11 tier-2/3)'; struct+parse doc comments rewritten). The 7 14-hearing-aid embedded_multimcu [[skip]] cells, previously workaround-tagged milestone='M6' with an M11-deferred reason inline (the old clamp rejected a literal M11), now carry their REAL milestone='M11' tag — 'what is M11-deferred' is greppable on the milestone field. The --milestone gate (milestone_in_gate, u8 PartialOrd) needed no change — verified range-agnostic by the cumulative-gate unit test + functional smoke (--milestone M7/M11 run, M12 fails loud). Tests: added M6/M7/M11 positive + M12 negative boundary pins; arg_parser_rejects_bad_milestone reject-probe M9->M12. Parallel review gate GO (qa + architect, no P1/P2); gate green clippy/test/test-release + e2e 280/246/0/34/0 unchanged. Architect P3 (hardcoded coverage-test gate bands assuming top==M6) filed as TASK-0359 (dep M7).
<!-- SECTION:FINAL_SUMMARY:END -->
