---
id: TASK-0071
title: cargo-deny and cargo-audit setup
status: To Do
assignee: []
created_date: '2026-05-17 23:30'
labels:
  - M0
  - infra
  - tooling
  - security
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add cargo-deny (license, advisory, source allowlist) and cargo-audit to the dev shell and a 'just audit' recipe. Pin the deny.toml policy: allowed licenses, advisory database freshness, source allowlist (crates.io only). Acceptance: 'cargo deny check' and 'cargo audit' green inside nix develop on a clean tree. Both tools must come from nix, not curl|sh.
<!-- SECTION:DESCRIPTION:END -->
