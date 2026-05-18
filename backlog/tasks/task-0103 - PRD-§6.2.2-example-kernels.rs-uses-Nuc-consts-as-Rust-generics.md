---
id: TASK-0103
title: PRD §6.2.2 example kernels.rs uses Nuc consts as Rust generics
status: To Do
assignee: []
created_date: '2026-05-18 00:55'
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
- [ ] #1 PRD §6.2.2 example is updated to a signature that compiles standalone as Rust.
- [ ] #2 Decision recorded in PRD: substitution by Nucleus / duplication in kernels.rs / dynamic shape.
- [ ] #3 Existing example kernels.rs (when they land for examples 5, 13, 14) follow the chosen convention.
- [ ] #4 TASK-0012 contract-check fixtures are revisited to match the chosen convention.
- [ ] #5 Test: PRD's exact example, copy-pasted into a kernels.rs, passes contract check.
- [ ] #6 Implementation notes record design questions encountered and the rejected alternatives.
- [ ] #7 Implementation notes record honest limitations (e.g. const substitution by Nucleus increases codegen complexity).
<!-- AC:END -->
