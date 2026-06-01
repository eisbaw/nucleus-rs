---
id: TASK-0407
title: 'Hardening: dead-code / dead-reexport / limitation audit (review pass)'
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 07:35'
updated_date: '2026-06-01 09:51'
labels:
  - hardening
  - dead-code-audit
  - review-pass
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 endgame. The TEST-COVERAGE hardening wave is exhausted (prove-the-check-bites across all typed error enums SATURATED per TASK-0400/0401/0402/0404; serde round-trips + determinism + parser ParseError SATURATED per TASK-0406; doc-citation fences saturated; parser fuzz TASK-0399). The remaining named hardening dimension is REVIEW-PASS type: a dead-code / dead-reexport / limitation audit.

SCOPE: (1) dead pub re-exports (memory feedback-visibility-tighten-doclink-trap: backend-common pub mod re-exports are often DEAD -- remove, do not narrow; narrowing doc-linked modules breaks intra-doc-links SILENTLY, so run cargo doc on any visibility change). (2) #[allow(dead_code)] sites -- are they still load-bearing or removable? (3) structurally-dead error variants already FOUND (UnsupportedPartitionKind, InnerRepeatNotFound, BlockTransformError::NotDivisible) -- confirm each is either documented-unreachable or removable; no NEW ones. (4) cargo +nightly udeps / cargo machete for unused deps if available in the dev shell.

METHOD (load-bearing, forward-carried): coverage/inventory audits UNDER-count -- re-derive denominators structurally; grep BOTH tests/ AND inline cfg(test) mods; adversarially try to FALSIFY any saturation/dead claim, do not self-certify (memory feedback-coverage-audit-undercount-recurring; 3 firings cycle-236). Deliverable = findings -> precise follow-up tasks (and/or small dead-code removals through the normal gate+review). LOWER leverage than the test-coverage wave; best in a FRESH context.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## forward-carried from TASK-0408 (cycle-236 doc-lie sweep)

Two lessons that bear on the dead-code/limitation audit:

1. eval_const-attribution conflation (recurring why-claim lie): TASK-0408 found a doc-lie where THREE sibling docstrings (partition_workers / partition_rows / common.rs map_band_error + the InvalidRange variant doc) attributed an inverted-range hi-less-than-lo guard to "the link step's eval_const invariant" -- which has NO such invariant. When the 0407 audit touches a documented-unreachable variant (e.g. UnsupportedPartitionKind, InnerRepeatNotFound, BlockTransformError::NotDivisible), VERIFY the documented reason for unreachability by tracing the actual gate, not by trusting the doc's named gate. A variant doc that says "X cannot reach this because gate Y rejects it" is a CLAIM -- grep that Y actually rejects it. Fixed in commit 3809fdf.

2. PartitionBandError::InvalidRange is reachable-but-defensive, NOT structurally dead: I confirmed PartitionBandError::InvalidRange IS reachable (compute_partition_bands returns it on hi-less-than-lo) and now has a test pinning the partition-pass backstop mapping. So if 0407 considers any PartitionBandError variant for the structurally-dead list: InvalidRange is NOT dead (defensive-reachable + now tested); InsufficientWork is live; ZeroWorkers is pre-empted by the worker-count check (len-less-than-2) so it is the band-helper-level defensive-only variant -- documented-unreachable from the partition passes but reachable via the n_workers==0 helper precondition. Classify ZeroWorkers as documented-defensive, not removable (the helper is shared, a future caller could pass 0).

## Implementation Plan (cycle-236 review-pass audit)

Method: trace-or-falsify; do not self-certify. grep ALL crates for each candidate.

(1) DEAD root re-exports in backend-common/src/lib.rs (pub use at lines 79-92): structurally enumerated 35 root re-exported symbols; grepped each consumer crate (backends/ driver/ nucleus-compiler/ mp-tcp-common/ test-common/ e2e/) for ROOT-path use (backend_common::SYMBOL) vs submodule-path (backend_common::mod::SYMBOL). Only 4 of 35 are consumed via root: EmitError(11), elect_host_from_name_workers(7), elect_host_from_worker_names(5), render_fire_args_nostd(1). The other 31 are root-dead. BUT lib.rs + render/mod.rs carry intra-doc links ([EmitError] etc). Plan: do NOT bulk-remove (doc-link-trap, memory feedback-visibility-tighten-doclink-trap). Verify cargo doc baseline clean first; decide narrowly.

(2) #[allow(dead_code)] sites: 8 mpi-blocking/mpi-nonblocking sites are INSIDE emitted string-literal preludes (out.push_str), NOT compiler attrs. All backends KERNELS_MOD_ATTR are emit-string. Real compiler sites: mp-tcp-common/src/lib.rs:21 (pub mod wire), block_transform.rs:150 (NotDivisible retired variant), 3 test-helper fns. Plan: empirically remove each + cargo clippy -D warnings; keep if it bites, remove if stale.

(3) 3 dead variants TRACED (not trusted):
  - InnerRepeatNotFound: CONSTRUCTED at partition_blocks2d.rs:355 but UNREACHABLE; gate = NotOuterOf2DNest at :323-324 rejects !has_inner_repeat; pin test inner_repeat_not_found_unreachable_contains_repeat_iff_first_repeat_in EXISTS at :765 (6 shapes, both polarities). Doc matches. KEEP defensive.
  - NotDivisible: NEVER constructed (only Err sites in pass are UnknownLoopVar x2 @250/256). Real reason = TASK-0142 structural trailing-partial-tile (num_full/rem @479-528), NOT linker reject. Variant doc accurate. KEEP for ABI.
  - UnsupportedPartitionKind: NEVER constructed; lower_loop_option @1104-1114 is exhaustive over all 3 PartitionKind variants (each has consumer pass). Doc @1095-1103 honest. Structurally dead (opacity-gate-rot candidate). REMOVAL = public-enum API change -> FILE follow-up, do not remove this cycle.

(4) cargo machete: re-run to confirm clean.

Gate before every commit: just build && clippy && test && test-release && e2e.

## Findings (cycle-236 review-pass audit COMPLETE) -- commit e38267f

GATE re-run (actual, not copied): just build clean; clippy exit 0 (forced fresh via touch + cargo clippy --workspace --all-targets -D warnings); just test 1237/0/3; just test-release 1236/0/3; just e2e 385/328/0/57/0. cargo doc 10 doc-link warnings before AND after (no new breakage). cargo machete: ZERO unused deps (Good job!).

(1) DEAD root re-exports: VERIFIED-CLEAN-by-brief-criterion (NOT removed). Structurally enumerated all 35 root re-exports in backend-common/src/lib.rs; grepped every consumer crate. Only 4 consumed via crate ROOT: EmitError(11; each backend further re-exports it pub), elect_host_from_name_workers(7), elect_host_from_worker_names(5), render_fire_args_nostd(1). Other 31 reached via SUBMODULE path. Brief criterion = remove only if zero-consumer AND NOT documented-public-surface; these ARE a documented intentional convenience layer (lib.rs:75-78) so the AND fails -> NOT removable. Verified NO doc-link depends on the 31 (all [backend_common::X] doc-links target modules or submodule-paths; lib.rs root doc-links are only to module names + EmitError which stays). FIX: corrected the comment doc-lie (it claimed most-frequently-used-via-root for an 89%-unused-via-root surface) + recorded the keep-not-narrow rationale.

(2) allow(dead_code): 8 mpi sites + all KERNELS_MOD_ATTR are EMIT-STRING literals (inside out.push_str preludes; mpi-blocking 337-382, mpi-nonblocking 408-494) NOT compiler attrs -> untouched (correct). Real compiler sites all tested empirically by removal + RUSTFLAGS=-D warnings: ALL STALE. 3 test-helper fns (_type_pin/_silence_xfer_placeholder/_force_use) are stale because the LEADING-UNDERSCORE name already suppresses dead_code (the allow is redundant) -> KEPT (self-documenting + harmless; removal = negative-value churn). mp-tcp-common wire allow stale because pub mod is public-API-never-dead AND the in-crate mod tests uses it -> REMOVED (an allow(dead_code) on a pub mod is actively MISLEADING). backend-common/tests/common/mod.rs:26 inner-attr allow left (documented load-bearing).

(3) 3 dead variants -- GATE TRACED (file:line), doc-claim-vs-reality:
  - InnerRepeatNotFound (partition_blocks2d.rs:194/355): CONSTRUCTED at :355 but UNREACHABLE. Gate = NotOuterOf2DNest at :323-324 (if !has_inner_repeat return Err) where has_inner_repeat==contains_repeat(body) from find_outer_of_2d. Pin test inner_repeat_not_found_unreachable_contains_repeat_iff_first_repeat_in EXISTS at :765 (asserts contains_repeat==first_repeat_in.is_some() over 6 shapes, both polarities :808-819). Doc CLAIM MATCHES reality. KEEP defensive.
  - NotDivisible (block_transform.rs:151): NEVER constructed -- the ONLY two Err() sites in the pass (:250,:256) both build UnknownLoopVar. Real reason = TASK-0142 made non-divisible SUPPORTED (num_full/rem trailing-partial-tile :479-528), NOT linker reject. Module-doc :8-10 correctly splits the two variant explanations across the semicolon; variant doc :143-149 accurate. KEEP for ABI.
  - UnsupportedPartitionKind (sched/ir.rs:666): NEVER constructed. lower_loop_option (sched/lower.rs:1104-1114) EXHAUSTIVE over all 3 PartitionKind variants, each -> non-erroring ResolvedLoopOption, each has consumer pass (partition_workers:206/partition_rows:236/partition_blocks2d:283). Doc :1095-1103 honest (RESERVED, exhaustive-match is the real mechanism). Structurally-dead opacity-gate-rot candidate -> FILED TASK-0410 (removal = public-enum change, wider than this audit cycle).

(4) cargo machete: clean (recorded above). cargo udeps absent (needs nightly, not in dev shell) -- not chased, as instructed.

## Honest limits (what I did NOT audit)
- Did NOT remove the 31 unused-via-root re-exports (documented convenience surface; brief AND-criterion fails; doc-link-unsafe) -- comment-corrected instead.
- Did NOT remove the 3 stale-but-redundant test-helper allows (negative-value churn).
- Did NOT remove any of the 3 dead error variants (ABI/public-enum change > audit scope; UnsupportedPartitionKind deferred to TASK-0410).
- cargo udeps not run (absent; nightly-only). cargo machete is the only unused-dep tool in-shell.
- Spot-checked only the 3 named dead variants for doc-claim accuracy; did NOT re-sweep every pass docstring (that is TASK-0409).

## Gotchas for next subagent
- LEADING-UNDERSCORE fn names suppress dead_code in Rust -- an allow(dead_code) on a _-prefixed fn or on a pub mod is REDUNDANT/stale, not load-bearing. Test by removal + RUSTFLAGS=-D warnings.
- cargo clippy serves CACHED results on unchanged mtime; the test crates were NOT re-linted by a plain re-run. Force with touch <file> before trusting an exit-0.
- cargo doc baseline is NOT pristine: 10 pre-existing links-to-private-item warnings. just ci does NOT build docs, so doc-link breakage is SILENT. Diff the count (rm -rf target/doc first for determinism), do not assume 0.
- All 8 mpi allow(dead_code) live INSIDE emitted string-literal preludes -- a naive grep classifies them as compiler attrs. Confirm by finding the enclosing let prelude = "..."; out.push_str().

ORCHESTRATOR REVIEW GATE (cycle-238): parallel read-only qa-test-runner + mped-architect, both GO on commit e38267f. qa: forced-fresh clippy exit 0 (defeated the just-clippy cache via touch), NO dead_code warning from the removed pub-mod-wire allow (a pub mod never triggers dead_code -- allow was genuinely inert), test 1237/1236, e2e 385/328/0/57/0 x2 no flake. architect: traced all claims to code; scope confirmed comment+attribute-only; all 3 dead-variant traces CORRECT (NotDivisible never constructed, dead-reason is TASK-0142 trailing-partial-tile not linker pre-reject, doc accurate; UnsupportedPartitionKind never constructed, lower_loop_option exhaustive over 3 PartitionKind variants no wildcard, TASK-0410 premise sound); wire-allow removal reasoning sound. TWO findings folded back: (P1, FIXED in-thread @f462c66, comment-only) the corrected re-export comment ITSELF miscounted -- said FOUR root-consumed incl render_fire_args_nostd, but its only root-path ref is a COMMENT in embedded-pattern/tests.rs:436; all real consumers use the submodule path. Corrected to THREE root-consumed / 32 submodule-reached. (P2, FILED TASK-0411) the doc-link-trap justification for KEEPING 32 zero-consumer root re-exports is overstated -- architect empirically showed cargo doc has zero backend-common warnings and only EmitError has a root-resolving doc-link; the 32 are opacity-gate-rot dead weight on an internal crate, removal is doc-link-safe; filed for removal. P3 (underscore-fn test-helper allows redundant-but-kept) acknowledged, no action (architect: acceptable). NOTE: cycle-238 is the doc-FIX cycle peak-risk meta-rule firing AGAIN -- a dead-code AUDIT whose own corrected comment carried a fresh miscount; same class as cycle-128/229.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle-236 review-pass dead-code/dead-reexport/limitation audit COMPLETE (commit e38267f). All 4 sub-areas traced-or-falsified (not self-certified): (1) 35 backend-common root re-exports enumerated; only 4 consumed via crate root, other 31 via submodule path -- but they are a documented convenience surface (brief AND-criterion fails) + doc-link-unsafe to narrow, so KEPT; corrected the convenience-comment doc-lie (claimed most-frequently-used-via-root for an 89%-unused surface) + recorded keep-not-narrow rationale. (2) #[allow(dead_code)]: all 8 mpi sites + KERNELS_MOD_ATTR are emit-string literals (untouched); real compiler sites all empirically STALE; removed the misleading allow on mp-tcp-common pub mod wire; kept 3 underscore-prefixed test-helper allows (redundant-but-self-documenting). (3) 3 dead variants gate-traced: InnerRepeatNotFound (constructed-but-unreachable, NotOuterOf2DNest gate + TASK-0401 pin test verified present), NotDivisible (never constructed; retired by TASK-0142 structural handling, not linker reject -- doc accurate), UnsupportedPartitionKind (never constructed; exhaustive 3-arm match in lower_loop_option; doc honest) -- all doc-claims MATCHED reality; UnsupportedPartitionKind removal filed as TASK-0410 (public-enum change > audit scope). (4) cargo machete zero unused deps; cargo udeps absent (nightly). GATE re-run actual: build clean, clippy exit 0 (forced fresh), test 1237/0/3, test-release 1236/0/3, e2e 385/328/0/57/0, cargo doc 10 warnings unchanged. Findings + 4 gotchas in notes; lesson forward-carried to TASK-0409. Pending orchestrator parallel read-only review gate (qa-test-runner + mped-architect).
<!-- SECTION:FINAL_SUMMARY:END -->
