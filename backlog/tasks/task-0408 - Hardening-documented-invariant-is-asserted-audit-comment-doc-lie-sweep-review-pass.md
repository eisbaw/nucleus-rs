---
id: TASK-0408
title: >-
  Hardening: documented-invariant-is-asserted audit + comment/doc-lie sweep
  (review pass)
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 07:35'
labels:
  - hardening
  - doc-lie
  - review-pass
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 endgame, review-pass dimension. The doc-citation-STALENESS fences (path:line, test-name, cell-path) are saturated, but a DIFFERENT doc-validation gap remains: (a) every invariant a docstring CLAIMS the code enforces should have an actual assertion/test (memory feedback-comment-doc-lie-recurring: a multi-claim docstring saying X-happens-because-Y is a CLAIM not a FACT; spot-check 3-5 per review and verify against the code); (b) examples in docs/PRD that purport to run should run.

SCOPE: sweep high-traffic module docstrings (the passes, backend-common render, event/sidecar contracts) for X-because-Y claims and verify each against the code; where a documented invariant has no assertion, add one (a debug_assert or a test) or correct the doc. This is the recurring comment-doc-lie class CLAUDE.md flags as a per-review audit. Several already fixed reactively this session (the TASK-0402 link-step-vs-build_acfg attribution; the SchedLowerErrorKind count). This task does it PROACTIVELY as a sweep.

METHOD: verify-against-code every claim before trusting it (memory feedback-implementer-disclosure-mechanism-wrong + feedback-coverage-audit-undercount-recurring -- 3 under-counts cycle-236, all caught by the gate). Deliverable = corrections + assertion-additions through the normal gate, or precise follow-ups. LOWER leverage; best in a FRESH context.
<!-- SECTION:DESCRIPTION:END -->
