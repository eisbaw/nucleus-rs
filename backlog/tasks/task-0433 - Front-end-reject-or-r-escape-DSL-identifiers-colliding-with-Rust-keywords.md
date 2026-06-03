---
id: TASK-0433
title: 'Front-end: reject (or r#-escape) DSL identifiers colliding with Rust keywords'
status: To Do
assignee: []
created_date: '2026-06-03 03:33'
labels:
  - compiler
  - frontend
  - panic-not-diagnostic
  - codegen
  - cycle-248
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0431 cycle-248 architect P3-2 (real latent defect CLASS, not a one-off; surfaced when an example data symbol named `in` generated `let mut in = ...` and failed rustc — worked around by rename in->src). ROOT CAUSE: the DSL KEYWORDS list (nucleus/nucleus-compiler/src/algo/parser.rs:155-175) rejects only DSL grammar words (const/data/kernel/for/scalar-types); it does NOT reject Rust keywords. So a data/kernel/worker identifier named in/let/mut/match/move/ref/loop/fn/type/as/self/crate/... is ADMITTED by the front-end and then emitted as `let mut <kw> = ...` by the `let mut {name}` codegen present in EVERY backend (tcp_plan, event_plan, mpi_plan, pthreads-*, openmp-rs, embedded-pattern). The failure surfaces as a confusing rustc parse/type error pointing at GENERATED source the user never wrote, not at their .nuc line — the project panic-not-diagnostic / usability-footgun class. FIX (pick one): (a) fail-loud front-end check listing Rust strict (and ideally reserved) keywords, emitting an EmitError/parse diagnostic at the .nuc identifier site; or (b) r#-escape identifiers in the data_name / `let mut {name}` codegen path so any identifier is legal. Prefer (a) for a clearer diagnostic, or (b) for max DSL freedom. Add a negative test (a .nuc with `data in : ...`) proving the check bites with a .nuc-site diagnostic, not a generated-crate compile error. LOW priority (single observed instance) but blast radius is every identifier x every backend.
<!-- SECTION:DESCRIPTION:END -->
