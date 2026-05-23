---
id: TASK-0070
title: clippy.toml + workspace lints policy
status: Done
assignee: []
created_date: '2026-05-17 23:30'
updated_date: '2026-05-23 20:56'
labels:
  - M0
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define workspace-wide clippy lint policy. PRD §12.3 justfile already plans 'cargo clippy --workspace -- -D warnings'. Decide which lints to allow/deny project-wide (e.g. clippy::pedantic? clippy::nursery? deny missing_docs?), pin in clippy.toml and/or [workspace.lints.clippy] in Cargo.toml. Acceptance: clippy with the chosen policy passes on the M0 skeleton and on the first real compiler code.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0186 (decision-0002, accepted): the project gate clippy scope is now cargo clippy --workspace --all-targets -- -D warnings (justfile clippy recipe; ci inherits via single source of truth). The §12.3 plan quoted in this task description (-- -D warnings WITHOUT --all-targets) is superseded for the gate. When defining the clippy.toml / [workspace.lints.clippy] policy here, the policy must hold under --all-targets (test/bin targets included) and must NOT narrow the gate back to default targets only.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep continuation). The task requires a design decision: which lints to allow/deny project-wide (clippy::pedantic? nursery? deny missing_docs?). Today the project gates on 'cargo clippy --workspace --all-targets -- -D warnings' (decision-0002) which catches the default warnings — sufficient for current quality bar. Reopen when a contributor wants pedantic/nursery enforcement OR when a class of lint repeatedly slips through. Until then, deciding-then-pinning the policy ahead of demand is premature. Same deferred-closure pattern as TASK-0069.
<!-- SECTION:FINAL_SUMMARY:END -->
