---
id: TASK-0256
title: Add just fmt-check recipe for asymmetric-strict CI gate
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-23 23:36'
updated_date: '2026-05-24 09:54'
labels:
  - infra
  - tooling
  - hygiene
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
QA-review P2 finding of TASK-0042.05 cycle 79: just fmt recipe is 'cargo fmt --all' (writes); there is no fmt-check variant. A developer running 'just fmt' before commit can silently reformat unrelated workspace files into their PR. just clippy is -D warnings strict but fmt enforcement is asymmetric.

Scope:
- Add 'just fmt-check' recipe: 'cd nucleus && cargo fmt --all -- --check'
- Optionally extend 'just ci' to invoke fmt-check (currently 'just ci' = test + clippy + e2e; should it also fmt-check?). Decision call — pre-existing TASK-0069 closure said fmt is informational; if so, just fmt-check stays a dev-side check, not a CI gate.

Acceptance: 'just fmt-check' recipe exists with one-line comment, runs from nucleus/, returns non-zero on any drift.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 91: add `just fmt-check` recipe (5 lines + comment). Run it; gate bites on accumulated drift (~330 lines). Don't auto-fix — file follow-up TASK-0276 for the bulk fmt cleanup as a separate clean cycle. Two commits: code (justfile recipe) + tracker (notes + close).
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 91, commit dbeba6a: added `just fmt-check` recipe (5 lines + 5-line comment) at justfile:33-39. Runs `cargo fmt --all -- --check` from nucleus/; returns non-zero on drift; intentionally NOT wired into `just ci` (per TASK-0069 closure — fmt is dev-side informational).

GOTCHA discovered: first run BITES on ~330 lines of accumulated rustfmt drift across many crates in nucleus/. NOT a cycle-90 regression — pre-existing project-wide drift accumulated over many cycles before the gate existed. Filed as TASK-0276 (bulk fmt cleanup) for a separate clean cycle (large mechanical diff deserves its own session with a full gate re-run). The gate doing exactly what it's designed to do.

Acceptance: recipe exists with comment, runs from nucleus/, returns non-zero on drift — all met. Closes architect F-P2 of TASK-0042.05 cycle 79.

CYCLE-91 REVIEW DISCLOSURE (architect P3-b, 2026-05-24): repo IS NOT rustfmt-clean as of this commit. The new gate is currently a DISCOVERY tool (a developer running it can see drift); bulk apply + enforcement is deferred to TASK-0276. `just ci` does NOT yet invoke fmt-check, so the strict-asymmetry F-P2 of TASK-0042.05 is closed at the RECIPE level but the enforcement asymmetry remains until TASK-0276 lands + a decision is made about CI inclusion.
<!-- SECTION:FINAL_SUMMARY:END -->
