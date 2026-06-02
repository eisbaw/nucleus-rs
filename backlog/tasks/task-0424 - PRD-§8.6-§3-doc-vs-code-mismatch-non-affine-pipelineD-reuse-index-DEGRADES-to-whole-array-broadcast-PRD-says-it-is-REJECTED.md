---
id: TASK-0424
title: >-
  PRD §8.6/§3 doc-vs-code mismatch: non-affine pipeline=D/reuse index DEGRADES
  to whole-array broadcast, PRD says it is REJECTED
status: To Do
assignee: []
created_date: '2026-06-02 02:27'
labels:
  - compiler
  - docs
  - prd-invariant-audit
  - cycle-241
  - doc-code-mismatch
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-5, VERIFIED. PRD §8.6 says affine indices only, the Petri-net IR does not relax this (implying REJECTION of non-affine). The code does NOT reject: affine_decompose returning None silently falls back to whole-array broadcast (transfer_inject/partition.rs:257,558,574). This is VALUE-CORRECT (whole-array is the safe superset) so NOT a soundness bug, but it is a documentation/enforcement mismatch (PRD claims rejection; code does graceful degradation). RESOLUTION OPTIONS: (a, cheapest+honest) reconcile PRD wording to non-affine indices conservatively degrade to whole-array broadcast; OR (b) if the project wants fail-loud discipline here (cf. TASK-0366 CumulativeWholeArrayFallback which WAS made fail-loud), add an ADVISORY diagnostic (not hard error, since correctness holds) when a pipeline=D/reuse-tagged loop hits the non-affine fallback. Low value for correctness; flagged as the documented-X-code-does-Y class. Pointer: src/passes/transfer_inject/partition.rs (affine_decompose None branch); PRD §8.6 line ~941-943.
<!-- SECTION:DESCRIPTION:END -->
