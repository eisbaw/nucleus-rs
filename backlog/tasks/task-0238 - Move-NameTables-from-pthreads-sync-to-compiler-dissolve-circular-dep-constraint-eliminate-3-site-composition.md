---
id: TASK-0238
title: >-
  Move NameTables from pthreads-sync to compiler (dissolve circular-dep
  constraint; eliminate 3-site composition)
status: To Do
assignee: []
created_date: '2026-05-22 05:20'
labels:
  - tech-debt
  - M4
  - backend
  - refactor
dependencies:
  - TASK-0237
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-24 review-gate B.1 finding (commit 52da9ad): NameTables is a 'compiler::event'-typed struct (5 BTreeMaps + BTreeSet) with zero pthreads-sync-specific content. It lives in pthreads-sync only because of historical path-of-least-resistance during TASK-0124.

This creates two cost centers:
1. test-common cannot depend on pthreads-sync (would be a circular dep: pthreads-sync dev-deps test-common; test-common deps pthreads-sync). So lower_for_test returns 5 raw BTreeMaps + BTreeSet, and each backend test composes its own NameTables in a 5-line block. Currently 3 sites (multi_worker.rs, check_frame_emit.rs, skeleton.rs). Wave B-2 will add a 4th.

2. A future change to NameTables (e.g. adding a field during Wave B-2's multi-worker codegen — buffer_for_seq or barrier_participants exposed) requires updating: (a) struct definition, (b) driver composition, (c) the 3-or-4 test composition sites, (d) LowerForTestResult if the field derives from acfg.*. The struct is centralized but its CONSTRUCTION is not.

Move NameTables from pthreads-sync to compiler:
- Define  in compiler/src/lib.rs or compiler/src/name_tables.rs.
- pthreads-sync gets .
- mp-tcp-bufsync + pthreads-async re-exports continue working (transitive re-export from pthreads-sync).
- test-common can now depend on compiler (already does), import NameTables, and return a pre-built NameTables instead of raw maps. The 3-site composition collapses into one.

Side benefit: the driver's NameTables construction can move into a  helper at the same site — removing another duplicate 5-field block. Wave B-2's pthreads-async multi-worker emit will use the same helper.

Defer until: this is bounded work but not Wave B-2-blocking. Land BEFORE the field-add (whenever that happens) to keep the cost from compounding.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exports NameTables as a struct (with all 5 fields).
- [ ] #2 pthreads-sync uses 'pub use compiler::NameTables' instead of defining the struct.
- [ ] #3 mp-tcp-bufsync + pthreads-async re-exports continue working (no caller-side changes needed).
- [ ] #4 test-common::lower_for_test now returns a pre-built NameTables in LowerForTestResult, eliminating the 5-field composition in 3 backend tests.
- [ ] #5 Driver's NameTables construction (driver/src/main.rs ~line 398) becomes a single NameTables::from_acfg(acfg) call.
- [ ] #6 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved.
<!-- AC:END -->
