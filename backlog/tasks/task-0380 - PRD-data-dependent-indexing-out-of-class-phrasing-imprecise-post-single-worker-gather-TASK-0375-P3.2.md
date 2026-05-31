---
id: TASK-0380
title: >-
  PRD data-dependent-indexing out-of-class phrasing imprecise post single-worker
  gather (TASK-0375 P3.2)
status: To Do
assignee: []
created_date: '2026-05-31 01:30'
labels:
  - docs
  - doc-lie
  - prd
dependencies:
  - TASK-0341.03.01
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DOC-HONESTY follow-up to TASK-0375 (architect P3.2, gather review gate). PRD.md:118 and PRD.md:1300 flatly list data-dependent indexing as out-of-class. After TASK-0341.03.01 a single-worker gather (17-spmv/gather, x[col[k]]) DOES compile and is 7-backend bit-identical, so the unqualified phrasing overshoots. The lines are framed around static decomposition / portability, and a DISTRIBUTED gather genuinely remains unsupported (halo_inference DataDependentStride fatal-under-partition; TASK-0373), so the statements are accurate about DISTRIBUTION but imprecise at the single-worker level. Tighten the PRD phrasing to data-dependent indexing does not DISTRIBUTE (single-worker gather supported; distributed broadcast is TASK-0373) so the PRD does not read as a doc-lie against the shipped 17-spmv/gather example. Recurring-defect-pattern #1.
<!-- SECTION:DESCRIPTION:END -->
