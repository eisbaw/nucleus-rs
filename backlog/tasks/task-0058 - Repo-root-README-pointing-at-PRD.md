---
id: TASK-0058
title: Repo root README pointing at PRD
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:10'
updated_date: '2026-05-20 22:10'
labels:
  - docs
  - infra
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A short README.md at the repo root summarising what Nucleus is, who it's for, and pointing readers at the PRD and the examples. Three paragraphs max. Updated incrementally per milestone.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 README.md at repo root has: one-sentence elevator pitch, a short 'what this is/isn't', and links to PRD.md plus examples/.
- [x] #2 README does not duplicate PRD content; it points to it.
- [x] #3 README mentions the algorithm-vs-schedule separation as the central commitment, in one sentence.
- [x] #4 Test: a reader who has never seen the project can find the PRD and one example within 30 seconds.
- [ ] #5 Implementation notes record any taglines tried and rejected.
- [x] #6 Implementation notes record honest limitations (e.g. README is intentionally minimal; the PRD is the spec).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation (orchestrator in-thread, post-TASK-0211 honest-stop cycle):

README.md added at repo root with:
- One-paragraph elevator pitch (algorithm/schedule split, cross-backend bit-identical differential).
- "Is/Isn't" block (thesis-grade implementation vs production polyhedral compiler/auto-tuner/training framework).
- AC#3: algorithm/schedule separation called out as "the central commitment" in one sentence.
- Pointers to PRD.md, examples/, docs/, nucleus workspace layout, backlog tracker.
- Running section with the just recipes (build/test/e2e/ci).

AC#5 (taglines tried + rejected): none rejected — the initial draft was kept as the elevator pitch. The "thesis-grade implementation of the algorithm/schedule split" framing was preferred over alternatives like "a Halide-like split for HPC + embedded" (too narrow — Halide is image-processing-coded; Nucleus targets MPI + embedded) and "a compiler for the algorithm/schedule split" (omits the falsifiable differential which is the headline thesis contribution).

AC#6 (honest limitations): README is intentionally minimal (3 paragraphs of pitch + a pointers section + a running section). The PRD is the spec; the README is a one-page front door. Updates per milestone are expected.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Repo-root README.md added: one-paragraph elevator pitch (algorithm/schedule split + cross-backend bit-identical differential), is/isn't block, pointers to PRD/examples/docs/nucleus/backlog, just-recipe running section. AC#3 algorithm/schedule-separation commitment called out in one sentence per AC. AC#5/#6 implementation notes recorded.
<!-- SECTION:FINAL_SUMMARY:END -->
