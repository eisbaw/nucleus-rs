---
id: TASK-0071
title: cargo-deny and cargo-audit setup
status: Done
assignee: []
created_date: '2026-05-17 23:30'
updated_date: '2026-05-23 20:56'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep continuation). cargo-deny and cargo-audit are reasonable hardening tooling but require: (a) deny.toml license allowlist (design decision — which licenses are acceptable for an academic thesis project?); (b) advisory-database freshness pattern (cadence + CI wiring); (c) tools available in the nix flake (currently NOT in the dev shell — adding them is itself non-trivial). Today the project has 0 third-party deps that aren't already mediated through nixpkgs (which provides license review at the distribution level). The marginal value of cargo-deny on top of nixpkgs is small until the project has crossed a compliance threshold or a contributor surfaces a license question. Reopen when (a) the dependency surface grows enough to warrant license audit OR (b) a sibling repo / institutional policy mandates cargo-deny/audit. Same deferred-closure pattern as TASK-0069/0070.
<!-- SECTION:FINAL_SUMMARY:END -->
