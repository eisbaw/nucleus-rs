---
id: TASK-0066
title: Re-verify MSRV pin once first Cargo.toml lands
status: To Do
assignee: []
created_date: '2026-05-17 23:24'
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
- [ ] #1 Cargo.toml has no rust-version field
- [ ] #2 cargo build of the first nucleus crate succeeds under the flake-pinned toolchain
- [ ] #3 If bumped: flake.nix rustChannel + sha256 updated; commit message states which dep forced the bump
<!-- AC:END -->
