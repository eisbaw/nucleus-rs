---
id: TASK-0352
title: >-
  Backfill TASK-0341.03.01 description and ACs (stub follow-up to cycle-219
  architect P3.4)
status: To Do
assignee: []
created_date: '2026-05-27 22:51'
labels:
  - tracker-hygiene
  - grammar-extension
  - backlog-debt
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0341.03.01 (DSL grammar gap: data-dependent indirect read inexpressible) was filed cycle 210 as the AC#2 honest-BLOCKED follow-up for 17-spmv. The cycle-219 review-gate noted it has no description (only the title carries the gap statement) and no formal ACs (only prose in TASK-0341.03 AC#2 closure addendum + TASK-0341.03's plan-time grammar inspection block).

## Why this matters

Per memory project-grammar-deferred-cluster: TASK-0341.03.01 sits in an epic with TASK-0341.02.01 (data-dependent loop termination) + TASK-0179 (1D prefix scan) + TASK-0044.05.01 (2D wavefront) + TASK-0044.06.01 (bitonic stage-parallel). All five hit the v2 sublanguage bottleneck and will eventually be picked up as one grammar-extension wave. When that wave arrives, TASK-0341.03.01's stub description means the implementer cannot easily pick up the gap without re-discovering it from TASK-0341.03's description prose.

## Acceptance criteria

1. Backfill TASK-0341.03.01 description with: (a) the grammar inspection from TASK-0341.03 (IndexExpr.Atom rule and why x[col_idx[i][k]] is inexpressible); (b) the workaround used (rectangular masked-accumulator with j == c kernel-branch); (c) cross-link to TASK-0044.04 histogram companion + the broader epic siblings.
2. Add at-least-skeleton ACs to TASK-0341.03.01 via the backlog CLI --ac flag, matching the structural shape of TASK-0341.02.01's ACs (which already exist).
3. No code changes; tracker-only hygiene.

## Honest scope LIMITS

- Doc-only. No grammar work happens here (that's the epic deferred to the broader grammar wave).
- Low priority because TASK-0341.03.01 is itself low priority (deferred follow-up); the asymmetry vs TASK-0341.02.01 is cosmetic. File only when convenient or when the grammar epic is being picked up.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
(orchestrator note) TASK-0341.03.01 was filed cycle 210 as the SpMV honest-BLOCKED grammar-gap follow-up but has no description and no formal ACs (the gap is only described in the title and TASK-0341.03 AC#2 closure note). Cycle 219's parent-epic closure (TASK-0341) review-gate caught this asymmetry vs the sibling TASK-0341.02.01 which has a full description + scope. Backfill task description + at-least-skeleton ACs from TASK-0341.03's plan-time grammar inspection (IndexExpr.Atom = IntLit | Ident | (AddExpr); no nested IndexSuffix; x[col_idx[i][k]] inexpressible) and the project-grammar-deferred-cluster epic context.
<!-- SECTION:PLAN:END -->
