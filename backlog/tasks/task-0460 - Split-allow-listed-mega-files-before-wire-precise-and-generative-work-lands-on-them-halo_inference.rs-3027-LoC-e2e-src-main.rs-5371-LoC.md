---
id: TASK-0460
title: >-
  Split allow-listed mega-files before wire-precise and generative work lands on
  them: halo_inference.rs (3027 LoC) + e2e/src/main.rs (5371 LoC)
status: Done
assignee: []
created_date: '2026-06-09 22:01'
updated_date: '2026-06-10 10:51'
labels:
  - hygiene
  - refactor
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P2.8). Both files sit on the mega-file fence allow-list (justfile:1216-1237) exactly where upcoming epic work lands: TASK-0453.22 / TASK-0455.07 extend halo/tile inference, and TASK-0455.05 extends the e2e harness. Split along docstring seams BEFORE that work starts, so the carve is content-preserving rather than entangled with semantic changes.

Discipline: split-dont-allow-list (memory feedback-cheap-subset-blind-to-structural-fences — TASK-0383 precedent where an allow-listed file sat RED for cycles); content-preserving carve per the TASK-0437/.01 precedent; re-grep bare-filename references and classify per-hit (memory feedback-carve-out-bare-filename-deixis-double-classification); doc-citation fences must pass post-move.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Both files under the fence limit; their allow-list entries removed from the justfile
- [x] #2 Carves content-preserving: pure mod moves, production behaviour unchanged (e2e + just ci green)
- [x] #3 Doc-citation fences pass post-move; stale path/filename references swept and classified
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Content-preserving carve of two allow-listed mega-files along docstring seams.
1. halo_inference.rs (~3027 LoC) -> directory-form split: halo_inference/ submodules carved by docstring seam; keep halo_inference.rs (or mod.rs) as the facade re-exporting public API; production behaviour byte-unchanged.
2. e2e/src/main.rs (~5371 LoC) -> sibling modules under e2e/src/ carved by seam; main.rs keeps fn main + mod decls.
3. Remove the two justfile mega-file allow-list entries.
4. Re-grep moved file old-name / bare-filename references repo-wide; classify source-anchor vs downstream; fix stale citations.
5. Verify: cargo test -p nucleus-compiler --test halo_inference; cargo build/test -p e2e; clippy x2; just check-mega-files (green w/ entries removed) + check-doc-citation-staleness + check-doc-links.
Ownership: the two target files + new siblings + justfile allow-list lines for these two ONLY + lib/main mod glue. NOT touching e2e/tests/**.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0460 carve COMPLETE (left In Progress for the batched gate).

HALO carve (directory form, content-preserving):
- halo_inference.rs 3027 -> 649 (facade: docstring + imports + HaloInferenceError + 3 entry points + type aliases + mod decls).
- New: halo_inference/partition_policy.rs (365), walker.rs (544), tests/mod.rs (167) + tests/{stencil.rs 502, partition_aware.rs 230, gather_scatter.rs 642}.
- Cross-module helpers promoted to pub(super): infer_halo_widths, commit_halo_widths, collect_iter_var_refs (walker); error_is_fatal_under_partition, scatter_target_replicates_whole_array (partition_policy); WalkCtx + fields, collect_from_stmts (walker, for whitebox tests). Test mod root re-exports via pub(super) use.
- Production bodies verified BYTE-IDENTICAL to original modulo the pub(super) visibility widening (diffed each carved range).

E2E carve (sibling-module form along section banners, content-preserving):
- e2e/src/main.rs 5371 -> 893 (keeps docstring/imports + execute_cells_parallel/run/run_inner/main + mod decls + pub(crate) use re-exports).
- New: manifest.rs (392), cli.rs (357), paths.rs (246), plan.rs (466), run.rs (655), determinism.rs (958), report.rs (779), baseline.rs (723).
- Pattern: each module `use super::*`; root `pub(crate) use <mod>::*` re-exports so tests.rs (use super::*) and run_inner resolve unchanged. Top-level items + cross-module struct fields + impl methods widened to pub(crate); trait-impl methods (fmt/default) left bare; Milestone(u8) -> Milestone(pub(crate) u8); report::ansi made pub(crate) mod + re-exported.
- All 8 modules verified BYTE-IDENTICAL to original ranges modulo visibility; main.rs head(1-67)+tail byte-identical.
- DID NOT touch e2e/src/tests.rs (another agent owns test trees; confirmed byte-identical to HEAD).

ALLOW-LIST: removed both entries; `just check-mega-files` GREEN (no file >1000 outside list, no stale entry).

DEIXIS sweep: fixed in-ownership stale citation gather_scatter.rs (algo_target_has_affine_partitioned_index now in partition_policy.rs). Fixed broken intra-doc links in halo_inference.rs facade (IrExpr::DataRef/IndexedRef/ResolvedLoopOption::Partition/error_is_fatal_under_partition/collect_iter_var_refs) by fully-qualifying paths — these BROKE because the carve trimmed the imports the bare links resolved against; check-doc-links (broken_intra_doc_links denied) caught them.

VERIFY: cargo test -p nucleus-compiler --lib 192/0; sidecar_halo 16/0 + halo_strip_synth 9/0 (halo unit 23 ran); cargo test -p e2e --bin nucleus-e2e 101/0; clippy both crates --all-targets -D warnings CLEAN; just check-mega-files GREEN (entries removed); check-doc-citation-staleness + -bare + check-doc-links all GREEN.

GOTCHAS / FOLLOW-UPS:
- determinism.rs is 958 LoC (closest to the fence). TASK-0455.05 extends the e2e harness — if it grows the determinism path it could re-cross 1000; watch and sub-split (perturbation injectors vs det-check) if so.
- pthreads-sync/src/multi_worker.rs:397 has a soft "See nucleus/e2e/src/main.rs" pointer to NUC_NONDET_TEST handling that MOVED to e2e/src/determinism.rs. Out of my ownership; not stale per the bare fence (file still exists) but the precise content moved. Forward-carry to whoever next touches that backend.
- check-doc-links was already RED at HEAD from PRE-EXISTING warnings in untouched files (net_soundness, mpi-*, embedded-pattern, acfg) BUT those are private_intra_doc_links/redundant-target WARNINGS (not denied); the only DENIED errors were the broken links my carve introduced, now fixed -> fence GREEN.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Both mega-files split content-preservingly, allow-list entries removed, check-mega-files GREEN. halo_inference.rs 3027 -> facade 649 + partition_policy/walker + tests/{mod,stencil,partition_aware,gather_scatter}. e2e/src/main.rs 5371 -> facade 893 + 8 sibling modules (largest determinism.rs 958 — watch item forward-noted on TASK-0455.05). Architect spot-diffed 4 moved ranges byte-identical modulo visibility prefixes; public API unchanged (lib.rs re-export untouched). Doc-citation fences green; broken search-in-this-file deixis fixed in fold-in 6ac4bb9; stale pthreads-sync pointer to moved determinism code updated. Landed 39ecd0b; architect GO; gate 2938/0 + e2e baseline held.
<!-- SECTION:FINAL_SUMMARY:END -->
