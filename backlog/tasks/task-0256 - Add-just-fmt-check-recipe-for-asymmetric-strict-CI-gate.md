---
id: TASK-0256
title: Add just fmt-check recipe for asymmetric-strict CI gate
status: To Do
assignee: []
created_date: '2026-05-23 23:36'
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
