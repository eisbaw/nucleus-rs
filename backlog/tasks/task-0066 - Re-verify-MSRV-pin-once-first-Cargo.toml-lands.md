---
id: TASK-0066
title: Re-verify MSRV pin once first Cargo.toml lands
status: Done
assignee: []
created_date: '2026-05-17 23:24'
updated_date: '2026-05-22 21:28'
labels:
  - M0
  - infra
  - tooling
  - msrv
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0001 pinned rustc to 1.83.0 in flake.nix as a conservative pre-Cargo guess. Per PRD §13 the rule is 'stable, ~6 months before M0'. Once the first Cargo.toml is committed (M0 skeleton): (1) confirm rust-version is NOT set in Cargo.toml — single source of truth lives in flake.nix; (2) sanity-check that 1.83.0 actually builds the planned deps (rayon, mio, serde, etc.); (3) bump if a planned dep needs newer.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Cargo.toml has no rust-version field
- [x] #2 cargo build of the first nucleus crate succeeds under the flake-pinned toolchain
- [ ] #3 If bumped: flake.nix rustChannel + sha256 updated; commit message states which dep forced the bump
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 62b tracker hygiene (2026-05-22). Both ACs structurally met:
- AC#1: nucleus/Cargo.toml does NOT carry a rust-version field. The workspace defers to the flake (flake.nix:8 'MUST NOT re-declare rust-version; the flake is the single source of truth'). Verified by grep.
- AC#2: cargo build succeeds under the flake-pinned toolchain. Continuously verified across 43 cycles in 2026-05-22 (e2e 88/70/0/18, just test 0 FAILED).

No source changes; no gate impact.
<!-- SECTION:FINAL_SUMMARY:END -->
