---
id: TASK-0456
title: >-
  block_transform: typed error for the synthetic __tile iter-var collision
  (panic on valid input)
status: Done
assignee: []
created_date: '2026-06-09 21:59'
updated_date: '2026-06-10 09:59'
labels:
  - panic-not-diagnostic
  - compiler
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P1.1), verified verbatim in the working tree: passes/block_transform.rs:349-354 panics when a user algorithm declares an iteration variable literally named <var>__tile while the schedule puts block=N on loop <var> — a valid (if obscure) program, hitting panic! instead of a diagnostic. The pass already has a typed BlockTransformError surface and driver mapping, so this is a routing fix, not new machinery. Optionally mangle the synthetic name instead of erroring; either way the outcome must be deterministic and loud.

Recurring class: feedback-panic-not-diagnostic-recurring. While in the file, grep remaining panic!/unwrap/expect on user-reachable paths and justify or fix each.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Collision produces a typed BlockTransformError naming both variables (or a documented mangle policy), never a panic
- [x] #2 Negative test compiles such a program end-to-end and pins the diagnostic
- [x] #3 Remaining panic-class sites in block_transform.rs audited: fixed or justified-unreachable, list in notes
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE NOTES (implementer):

DESIGN DECISION — typed error, NOT mangling. Chose to reject loud with a new typed variant SyntheticTileVarCollision { tiled_var, tile_var } rather than auto-mangle the synthetic <var>__tile name. Rationale: (a) the pass is on the typed-Result side per decision-0003; a clean nucleus: error line is the convention. (b) Mangling would need a fresh collision-free name threaded consistently into every downstream consumer that assumes the literal <var>__tile shape (transfer_inject hoist, sidecar, etc.) — far larger surface for an obscure input. (c) The fix is deterministic and loud either way; erroring names BOTH variables so the user renames one. Display tells the user exactly how to fix it (rename the user var or the block= target loop).

ROUTING — no driver change needed. The panic was the ONLY non-typed exit. The new variant routes through the EXISTING driver mapping: driver/src/main.rs:346 PreMediationError::BlockTransform(e) => format!("block-transform error: {e}") formats via Display. PreMediationError already wraps BlockTransformError (pipeline.rs:65/125). So I added ZERO lines to driver/src/main.rs. The PreMediationError driver match is exhaustive at the OUTER level only; inner BlockTransformError Display handles the new variant. AC#1 satisfied without touching the driver.

AC#2 — end-to-end negative test. block_synthetic_tile_var_collision_is_typed_error_not_panic drives the REAL pass entry point via crate::test_support::build_pre_mediation_acfg (parse -> lower -> link -> run_pre_mediation_passes which calls apply_block_transforms). The fixture algorithm declares BOTH a y loop and a y__tile loop (y__tile is a legal identifier — _ allowed after first char, confirmed in lexical::ident_chars). Schedule puts loop y : block=64. The test ALSO compiles the IDENTICAL algorithm with block= REMOVED and asserts it succeeds — proving the program is genuinely VALID and the collision is the sole failure (not an unrelated parse/link defect). Plus a Display-wording unit test synthetic_tile_var_collision_display_names_both_vars. Both pass.

AC#3 — panic-class audit of block_transform.rs (grep panic!/unwrap/expect/unreachable/todo):
  - L350 panic! (collision) — FIXED -> typed SyntheticTileVarCollision. This was the user-reachable one.
  - L347 (now ~393) .expect("validated above") — JUSTIFIED unreachable: step 2 validation (loop over block_by_var.keys) returns UnknownLoopVar if name_iter_vars lacks the key; the map is not mutated (only next_id cursor) before this site. True invariant, not user input. Added a comment documenting this.
  - L629 .expect("len==1") — JUSTIFIED unreachable: inside the tiles.len() match arm 1 =>, so tiles has exactly one element; into_iter().next() is Some by construction.
  - L343/384 .unwrap_or(0) — not a panic; default for empty iter-var map. Safe.

ALSO FIXED (doc-lie hygiene): the module-level error-convention docstring claimed UnknownLoopVar was "its one live variant" — STALE (omitted BlockOnUntilLoop, a live constructed variant added later for epic S4, and now SyntheticTileVarCollision). Rewrote it to enumerate all three live variants + the retired NotDivisible. Caught a clippy doc_lazy_continuation on the bullet list (needed a blank //! before the trailing NotDivisible paragraph) — fixed.

VERIFICATION:
  - cargo test -p nucleus-compiler block_transform: inline module 8/8 pass (incl 2 new), integration tests/block_transform.rs 10/10 pass.
  - cargo clippy -p nucleus-compiler --lib: clean (exit 0) — covers block_transform.rs + my doc edits.

OWNERSHIP-BOUNDARY FINDING (not mine, do not touch): cargo clippy --all-targets currently RED on an UNRELATED file: tests/petri_to_events.rs:21 unused import NotifyMode. That file shows git status M (another wave is mid-edit). NOT caused by my change and outside my ownership; left untouched. Filing as a follow-up.

TOUCHED FILES: nucleus/nucleus-compiler/src/passes/block_transform.rs, nucleus/nucleus-compiler/src/passes/block_transform/tests.rs. (driver/src/main.rs NOT touched — existing mapping sufficed.)
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Panic at block_transform.rs:350 (user iter var named <var>__tile + block=N on <var>, a valid program) replaced with typed BlockTransformError::SyntheticTileVarCollision naming both vars + rename hint; routes through the existing driver Display mapping, no driver change. End-to-end negative test via the real pass entry point + Display pin; same fixture without block= proven to compile clean. Panic-class audit: 2 expect() justified-unreachable inline, unwrap_or(0) safe. Stale module docstring (one-live-variant claim) fixed. Landed 88b4dbf; architect review GO; wave gate 2912/0 + e2e 497/428/0/69/0.
<!-- SECTION:FINAL_SUMMARY:END -->
