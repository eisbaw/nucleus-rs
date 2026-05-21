---
id: TASK-0047
title: M9 — Tier 3 embedded skeleton (no_std codegen)
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-21 17:36'
labels:
  - M9
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
First tier-3 milestone: embedded-pattern backend emitting no_std Rust against a stub shim trait. Compile-only acceptance — no hardware or simulator yet. PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/embedded-pattern/ crate lands; emits no_std code.
- [ ] #2 Shim trait NucleusShim defined: methods for alloc-in-region, dma-push, dma-wait, irq-barrier.
- [ ] #3 Generated code compiles against a stub shim that does nothing (just satisfies the trait).
- [ ] #4 Test: 'cargo check --target thumbv7em-none-eabihf' succeeds for examples 1, 5 under M9 backend.
- [ ] #5 Implementation notes record design questions (e.g. shim trait shape; whether shims provide async or sync semantics).
- [ ] #6 Implementation notes record honest limitations (no DMA, no IRQ, no real timing; just compile-only).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Toolchain prereq satisfied by TASK-0062 (commit 9787412 / 2026-05-21): `nix develop .#embedded` provides thumbv7em-none-eabihf rust-std on the pinned 1.83.0 toolchain. AC#2 of TASK-0062 already verified a no_std hello-world cross-builds to ARM ELF inside that shell. Start this skeleton inside .#embedded, not the default shell.
<!-- SECTION:NOTES:END -->
