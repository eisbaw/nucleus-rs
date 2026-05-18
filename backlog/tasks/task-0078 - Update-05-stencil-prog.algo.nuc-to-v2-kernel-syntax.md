---
id: TASK-0078
title: Update 05-stencil prog.algo.nuc to v2 kernel syntax
status: To Do
assignee: []
created_date: '2026-05-17 23:44'
labels:
  - M0
  - language
  - examples
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
examples/05-stencil/prog.algo.nuc uses the legacy 2013-style kernel-with-inline-body syntax (kernel NAME(args) -> out where pure {{ ${out} = ... }};) which is incompatible with PRD §6.2.2 (v2: signature-only declaration in .algo.nuc, body in adjacent kernels.rs, no ${} substitution). Rewrite the file so its kernel declarations match the v2 form already used by examples 13-cnn-inference and 14-hearing-aid, and add a sibling kernels.rs containing the blur3 / load_image / save_image bodies. Verified against the grammar in docs/grammar-algo.md (TASK-0005 §4.3).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/05-stencil/prog.algo.nuc parses under the grammar in docs/grammar-algo.md,Kernel bodies moved to examples/05-stencil/kernels.rs as plain Rust functions,No ${} substitution syntax remains anywhere in the example,'where pure' / 'where \!effectful' replaced with trailing 'pure' / 'effectful' keyword per PRD §6.2.2,File compiles when the parser from TASK-0006/TASK-0007 lands
<!-- AC:END -->
