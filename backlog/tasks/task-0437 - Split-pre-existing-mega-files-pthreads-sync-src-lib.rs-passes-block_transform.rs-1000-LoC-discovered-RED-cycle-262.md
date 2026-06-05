---
id: TASK-0437
title: >-
  Split pre-existing mega-files pthreads-sync/src/lib.rs +
  passes/block_transform.rs (>1000 LoC; discovered RED cycle-262)
status: In Progress
assignee:
  - '@claude'
created_date: '2026-06-03 17:58'
updated_date: '2026-06-05 14:59'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The just-ci check-mega-files gate was found RED at cycle-262 HEAD: nucleus/backends/pthreads-sync/src/lib.rs (1032 LoC at HEAD, grew to ~1097 after the cycle-262 break_loop.rs extraction) and nucleus/nucleus-compiler/src/passes/block_transform.rs (1043 LoC, untouched by cycle-262) BOTH exceed 1000 LoC and are NOT in the check-mega-files allow-list. This is the feedback-cheap-subset-blind-to-structural-fences recurrence: the cheap pre-commit subset (build/clippy/test/test-release/e2e) does not run check-mega-files, so a file silently crossing 1000 LoC sat RED. Cycle-262 (TASK-0341.02.01.06) extracted the new for..until break machinery into break_loop.rs (222 LoC) to AVOID worsening lib.rs further, and ALLOW-LISTED both files with a rationale to keep just ci green, but the proper fix is a SPLIT. lib.rs: the ~520-LoC render_event fn is the bulk; split along the Event:: arm seams (Fire / Loop / Sync-Push-Wait) named in the module docstring. block_transform.rs: split along its strip-mine tile/seq/inner construction seams. Preferred fix per the gate is option #1 (split into cohesive sub-modules), removing the allow-list entries afterward (direction-B stale-entry guard).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
THIRD pre-existing offender found: nucleus/nucleus-compiler/src/event.rs (1036 LoC at HEAD, untouched by cycle-262 — the EventList contract types Event/FireBinding/DataSlice/IterTile + serde). All THREE (event.rs, pthreads-sync/lib.rs, block_transform.rs) added to the check-mega-files allow-list cycle-262 to restore just-ci GREEN; proper split (option #1) + allow-list removal is this task. event.rs split seam: the Event enum variants vs the binding/slice value types vs the serde impls.

Implementation plan (cycle-263, block_transform half only — orchestrator-narrowed): carve the inline #[cfg(test)] mod tests (lines 663-1043, 6 tests) out of nucleus-compiler/src/passes/block_transform.rs into a NEW sibling file passes/block_transform/tests.rs, replacing the inline block with `#[cfg(test)]\nmod tests;`. Edition-2021 foo.rs + foo/tests.rs coexistence (passes/mod.rs unchanged). Then REMOVE the now-stale allow-list entry for block_transform.rs from justfile:check-mega-files (direction-B guard), leaving pthreads-sync/src/lib.rs entry intact. Zero production-logic change. pthreads-sync split deferred to new child TASK-0437.01.

Cycle-263 RESULTS (block_transform half DONE; pthreads-sync half NOT — task stays In Progress). Commit ad94ca5. block_transform.rs: 1043 -> 664 LoC (under the 1000 fence). New passes/block_transform/tests.rs: 378 LoC. Test-count unchanged: 6 #[test] before (inline mod tests) == 6 after (cargo test -p nucleus-compiler --lib block_transform reports 6 passed under passes::block_transform::tests::*). check-mega-files OK BOTH directions (no direction-A new mega-file; no direction-B stale entry — the block_transform allow-list line was removed; pthreads-sync/src/lib.rs entry kept). Full `just ci` GREEN (exit 0): dev tests 1432 passed (baseline 1432/0/3), release tests 1430 passed (baseline 1430/0/3), positive e2e 483/420/0/63/0 byte-identical to baseline, clippy clean. Zero production-logic change (diff = inline mod tests removed + `mod tests;` decl + justfile allow-list line + rationale comment). Filed child TASK-0437.01 for the deferred risky pthreads-sync/src/lib.rs split (production-heavy, public render_single_worker_main* API must be preserved as re-exports). NOT marking parent Done — it covers both files; only block_transform is split this cycle. NOTE: independent review gate (qa-test-runner + mped-architect) is run by the orchestrator, not self-certified here.

Cycle orchestrator INDEPENDENT review gate (parallel, read-only) — both GO. Landed SHA = e63abbf (NOTE: an earlier note in this task cited ad94ca5, the pre-amend SHA; the authoritative landed commit is e63abbf — code/justfile identical between the two, only this tracker note block differs).
- qa-test-runner: GO. Independently re-ran: just build clean; clippy clean (forced re-lint of moved tests.rs); just test 1432/0/3 dev; just test-release 1430/0/3 (unchanged — tests moved, none added/removed); 6 block_transform tests run+pass under new path passes::block_transform::tests::; just check-mega-files OK both directions (block_transform.rs 664 LoC de-listed, pthreads-sync 1044 retained); just e2e 483/420/0/63/0 x2 non-flaky; commit touches only the 5 expected files; tree clean.
- mped-architect: GO. Independently confirmed the de-indent is whitespace-only/lossless (no multi-line/raw string literals in the test body that a blanket 4-space strip could corrupt; 6 #[test] fns same names/order; production lines 1-663 byte-identical to parent); allow-list direction-B correct + recipe rationale comment not lying; TASK-0437.01 correctly parented (-p=--parent) with honest production-heavy scope + forward-carried gotchas; TASK-0437 honestly In Progress (no AC-gaming).
- Architect P3-1 (stale self-cited SHA) addressed by this note. Architect P3-2 (PRE-EXISTING, not introduced here): the retained pthreads-sync allow-list bullet comment says "~flat at 1035" but the file is 1044 LoC — forward-carried to TASK-0437.01 (where that entry is removed entirely when the split lands). 
DELIVERABLE this cycle: block_transform.rs mega-file split via verbatim test-mod carve-out (1043->664 LoC), stale allow-list entry removed, full just ci GREEN, e2e baseline held 483/420/0/63/0. TASK-0437 stays IN PROGRESS — pthreads-sync/src/lib.rs half tracked in TASK-0437.01.
<!-- SECTION:NOTES:END -->
