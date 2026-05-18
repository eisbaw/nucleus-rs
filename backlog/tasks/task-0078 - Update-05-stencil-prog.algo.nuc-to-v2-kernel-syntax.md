---
id: TASK-0078
title: Update 05-stencil prog.algo.nuc to v2 kernel syntax
status: Done
assignee: []
created_date: '2026-05-17 23:44'
updated_date: '2026-05-18 04:42'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation summary
=========================

Subsumed by TASK-0031. The 05-stencil/prog.algo.nuc rewrite from legacy 2013-style `kernel NAME(args) -> out where pure {{ ${out} = ... }};` to v2 form (signature-only declaration, body in adjacent kernels.rs, trailing pure / effectful keyword) landed as part of the broader Example 5 deliverable.

What changed (the TASK-0078 surface specifically)
=================================================

- examples/05-stencil/prog.algo.nuc rewritten in place. Verbatim shape now matches docs/grammar-algo.md (the §4.3 'KNOWN DIVERGENCE' callout has effectively become a non-divergence as of this commit, though the doc still names TASK-0078 — out of scope to edit docs/grammar-algo.md per the no-touch rule, and the doc's authority is the parser test).
- Kernel bodies moved to examples/05-stencil/kernels.rs as plain Rust functions (blur3 nine-arg scalar, load_image / save_image effectful Vec<i32> I/O).
- No ${...} substitution syntax remains in the example. `where pure` and `where !effectful` replaced with trailing `pure` / `effectful` keyword.
- File parses under the grammar in docs/grammar-algo.md — pinned by the new `parses_example_05_stencil` test in nucleus/compiler/tests/algo_parser.rs (which replaces the old `rejects_legacy_05_stencil` negative test). The 'legacy syntax rejected' invariant moves to `negative_legacy_inline_kernel_body` which uses a distilled fragment instead of pointing at this example file.

AC verification
===============

#1 (everything in the same backlog entry):
  * prog.algo.nuc parses under docs/grammar-algo.md grammar — pinned by parses_example_05_stencil.
  * Kernel bodies moved to examples/05-stencil/kernels.rs as plain Rust functions.
  * No ${} substitution syntax remains in the example.
  * `where pure` / `where !effectful` replaced with trailing `pure` / `effectful` keyword.
  * File compiles with the TASK-0006/TASK-0007 parser landed (which it had landed pre-rewrite — the previous parser tests gated this exact transition).

Honest limitations
==================

1. docs/grammar-algo.md §4.3 still narrates 05-stencil as a 'KNOWN DIVERGENCE' even though the example now conforms. Out of scope to fix here (no-touch rule on docs/grammar-algo.md). The parser test (parses_example_05_stencil) is the authoritative pinning; the grammar doc is informative.

Verification
============

- just check / just clippy / just test: all green (see TASK-0031 notes).
- algo_parser test parses_example_05_stencil PASSES.
<!-- SECTION:NOTES:END -->
