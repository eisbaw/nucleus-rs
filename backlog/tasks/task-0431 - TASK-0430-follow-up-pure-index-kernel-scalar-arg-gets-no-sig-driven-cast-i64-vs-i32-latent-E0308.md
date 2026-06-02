---
id: TASK-0431
title: >-
  TASK-0430 follow-up: pure index-kernel scalar arg gets no sig-driven cast (i64
  vs i32 latent E0308)
status: To Do
assignee: []
created_date: '2026-06-02 23:43'
labels:
  - compiler
  - scatter
  - grammar-extension-epic
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
render_int_expr IrExpr::Call arm (TASK-0430, render/expr.rs) emits a pure index-kernel call kernels::callee(args) WITHOUT a per-param as-type cast, unlike render_fire_arg which casts iter-var scalars to the kernel sig param type. For the landed 08-histogram/textbook example the only arg is an i32 gather load (input[i]), which typechecks. But a pure index-kernel whose param is i32 and whose arg is a bare iter var (rendered i64 in generated code) would hit E0308 at build of the generated crate. Not a silent miscompile (rustc catches it loudly), but a usability gap. Fix needs plumbing the callee KernelId + sidecar kernel_sig param types into render_int_expr (today it takes only an IrExpr). Scope: thread an optional sig lookup or a richer ctx into render_int_expr Call arm; add an example/test exercising an iter-var scalar arg to an index kernel. LOW priority - no current example needs it; the bounded/textbook scatters use data-ref args only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 render_int_expr emits a per-param as-type cast for scalar args of a pure index-position kernel call, matching render_fire_arg
- [ ] #2 example/test exercises an iter-var (i64) scalar arg to an i32-param index kernel and builds clean across tier-1 backends
<!-- AC:END -->
