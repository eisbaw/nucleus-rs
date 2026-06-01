---
id: TASK-0412
title: >-
  Drop dead ChanId pub re-export symmetrically across event_plan + mpi_plan
  substrates
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 17:39'
updated_date: '2026-06-01 19:52'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE cycle (commit 947f7de). Dropped ChanId from both event_plan/mod.rs + mpi_plan/mod.rs re-exports (keep Plan); narrowed pub type ChanId -> pub(crate) in both plan.rs with parallel doc comments. Symmetric across both substrates (no divergence).

VERIFIED (orchestrator in-thread per feedback-spawned-agents-refuse-code-edits): cargo build --workspace OK; clippy -p backend-common clean; cargo doc --workspace --no-deps warning count UNCHANGED (14 = baseline, per feedback-visibility-tighten-doclink-trap); cargo test -p backend-common 5+10 pass. Zero functional effect (no codegen path reads visibility; emit byte-identity untouched by construction).

REVIEW GATE: mped-architect read-only GO. Independently confirmed: no E0446 (all ChanId-exposing items pub(crate)/private), no [`ChanId`] intra-doc-link exists repo-wide (cargo doc -p backend-common --no-deps clean), symmetry holds, and the pre-existing event_plan/mod.rs "only the Plan API is re-exported pub" comment is now MORE accurate (ChanId removal closes a prior doc discrepancy). Heavy qa-test-runner e2e arm intentionally NOT run for a visibility-only change (batched per feedback-batch-qa-gate-not-per-task; provably cannot affect e2e — no symbol referenced anywhere).

ARCHITECT P2 SILENT-SIBLING -> filed TASK-0413: tcp_plan::XferId is the analogous dead external re-export (grep-verified zero consumers), but NOT trivially narrowable — tcp_plan::Plan exposes `pub xfer_ids: BTreeMap<DataId, XferId>` so narrowing needs the field tightened first (E0446). Correctly out of THIS task scope (ChanId / event_plan+mpi_plan only).

GOTCHA for TASK-0413 (forward-carry): the reason ChanId could be narrowed freely but XferId cannot is FIELD HYGIENE asymmetry — event_plan/mpi_plan made their chan_ids fields pub(crate)/private from the start; tcp_plan::Plan is all-pub. Narrowing a chan-id-style alias is only free when its exposing fields are already non-pub. P3 mention-only: event_plan/mod.rs "only the Plan API" also understates (EventTransport re-exported too); tighten wording if TASK-0413 touches those comments.
<!-- SECTION:NOTES:END -->
