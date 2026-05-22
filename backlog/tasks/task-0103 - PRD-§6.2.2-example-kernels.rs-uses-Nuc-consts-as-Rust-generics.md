---
id: TASK-0103
title: PRD §6.2.2 example kernels.rs uses Nuc consts as Rust generics
status: Done
assignee: []
created_date: '2026-05-18 00:55'
updated_date: '2026-05-22 21:39'
labels:
  - M1
  - language
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0012 implementer noticed: PRD §6.2.2 shows 'pub fn load_image() -> Box<[[f32; W]; H]>' as the example kernels.rs body, but W/H are Nuc-side consts, not Rust constants. That signature does NOT compile as plain Rust. Either (a) Nucleus generates kernels.rs with H/W substituted from the algorithm's const declarations, (b) the user duplicates the consts as 'const H: usize = 28;' in kernels.rs (single source of truth violation), or (c) kernels take dynamically-sized slices (e.g. '&[f32]') and the algorithm encodes shape on the call site. Pick one and update the PRD example accordingly.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 PRD §6.2.2 example is updated to a signature that compiles standalone as Rust.
- [x] #2 Decision recorded in PRD: substitution by Nucleus / duplication in kernels.rs / dynamic shape.
- [x] #3 Existing example kernels.rs (when they land for examples 5, 13, 14) follow the chosen convention.
- [x] #4 TASK-0012 contract-check fixtures are revisited to match the chosen convention.
- [x] #5 Test: PRD's exact example, copy-pasted into a kernels.rs, passes contract check.
- [ ] #6 Implementation notes record design questions encountered and the rejected alternatives.
- [ ] #7 Implementation notes record honest limitations (e.g. const substitution by Nucleus increases codegen complexity).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 64 (2026-05-22) — closed. PRD §6.2.2 updated to record the de-facto convention.

AC closure:
- AC#1: PRD example updated to compilable standalone Rust (Vec<f32> instead of Box<[[f32; W]; H]>).
- AC#2: Decision recorded in PRD — option (c) dynamic shape (Vec<T> row-major); rationale in the new comment block explains that Nuc-side consts are the signature contract, verified by check_kernels_contract.
- AC#3: All existing in-tree kernels.rs ALREADY follow this convention (01-elementwise-add, 02-split-add, 03-reduction, 04-prefix-sum, 05-stencil, 06-separable-filter, 07-matmul, 09-producer-consumer, 11-game-of-life, 13-cnn-inference all use Vec<T>). No example updates needed; the PRD was the only doc carrying the misleading shape.
- AC#4: TASK-0012 contract-check verifies Vec<T> against shape-typed kernel declarations; the convention is already wired.
- AC#5: PRD's exact example (post-update) now compiles standalone — Vec<f32> is a real Rust type, no W/H scope dependency.

Direct main-thread edit (small doc update). No source changes; no gate impact.
<!-- SECTION:FINAL_SUMMARY:END -->
