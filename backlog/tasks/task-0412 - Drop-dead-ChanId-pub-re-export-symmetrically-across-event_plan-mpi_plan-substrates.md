---
id: TASK-0412
title: >-
  Drop dead ChanId pub re-export symmetrically across event_plan + mpi_plan
  substrates
status: To Do
assignee: []
created_date: '2026-06-01 17:39'
labels:
  - tooling
  - dead-code
  - backend-common
  - cycle-0046.02-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0046.02 cycle architect P3-2. backend_common::mpi_plan::mod.rs:61 re-exports `pub use plan::{ChanId, Plan}` but ChanId has NO external consumer (no backend references backend_common::mpi_plan::ChanId). It EXACTLY mirrors the pre-existing sibling event_plan/mod.rs:70 `pub use plan::{ChanId, Plan}`, whose ChanId is ALSO externally unconsumed (TASK-0411 dead-reexport-removal cycle deliberately left it). Narrowing only mpi_plan would DIVERGE the two substrates (worse smell than matching precedent), so this cycle left mpi_plan consistent with event_plan. This task tracks the SYMMETRIC cleanup: drop ChanId from BOTH `pub use plan::{ChanId, Plan}` lines (keep Plan), leaving `pub type ChanId` at pub(crate) in each plan.rs (it is an intra-crate doc-link target only, so pub(crate) still resolves the link). Verify with cargo doc --workspace --no-deps that the warning set is unchanged (memory: feedback-visibility-tighten-doclink-trap). LOW / OPTIONAL: zero functional effect; pure dead-surface tidy in the dead-reexport class of TASK-0411. Do NOT narrow asymmetrically.
<!-- SECTION:DESCRIPTION:END -->
