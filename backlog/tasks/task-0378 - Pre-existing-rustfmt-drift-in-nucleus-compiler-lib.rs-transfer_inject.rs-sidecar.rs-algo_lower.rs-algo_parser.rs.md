---
id: TASK-0378
title: >-
  Pre-existing rustfmt drift in nucleus-compiler (lib.rs, transfer_inject.rs,
  sidecar.rs, algo_lower.rs, algo_parser.rs)
status: To Do
assignee: []
created_date: '2026-05-30 23:35'
updated_date: '2026-05-30 23:36'
labels:
  - fmt
  - hygiene
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered during TASK-0377 (cycle 218): cargo fmt --all -- --check reports formatting drift in several committed files that no recent task touched: nucleus-compiler/src/lib.rs (net_soundness re-export ordering), passes/transfer_inject.rs (~7 sites), src/sidecar.rs (2 sites), tests/algo_lower.rs (2 sites), tests/algo_parser.rs (1 site). These predate TASK-0377 (diff is vs HEAD; the files are unmodified by 0377). The everyday cheap gate (just build+clippy+test+test-release+e2e) does NOT include a fmt-check arm, so the drift went unnoticed; just ci (full gate) likely catches it via just fmt-check. Fix: run cargo fmt --all once on the tree, verify just ci stays green, commit as a formatting-only change. Low risk, mechanical.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 cargo fmt --all -- --check is clean on the whole nucleus workspace
- [ ] #2 just ci passes (fmt arm included)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Likely duplicate/overlap of the older TASK-0276 (Apply accumulated rustfmt drift, TASK-0256 follow-up). Both are the same recurring deferred fmt-cleanup condition. When picked up, dedupe against TASK-0276 — fix once, close both.
<!-- SECTION:NOTES:END -->
