---
id: TASK-0457
title: >-
  Doc-lie sweep from the 2026-06-09 architecture review (5 witnesses) + delete
  dead petri_to_events wrapper
status: Done
assignee: []
created_date: '2026-06-09 21:59'
updated_date: '2026-06-10 09:59'
labels:
  - doc-lie
  - hygiene
  - compiler
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the review (P1.2 + P3.10 + P3.12). Five verified comment-doc lies (line numbers as of review; re-grep on pickup):
1. nucleus-compiler/src/event.rs:165-166 cites transfer_inject.rs:279-290 — that file was split into a directory (TASK-0340.13); the single-global-counter invariant is TRUE but lives at passes/transfer_inject/mod.rs:565-571. Load-bearing citation for the transfer_buffer_for_seq keying decision — fix it.
2. event.rs:786-787 AND sidecar.rs:33-34 both claim build_acfg panics on a non-const loop bound; stale since TASK-0398 made it typed (NonConstLoopBound/OverflowingLoopBound, acfg/build.rs:273-279).
3. backend-common/src/multi_worker_walker/event_walker.rs:2-5,27-31 say the walker is consumed by three backends; lines 75-77 of the SAME file say five; ground truth is seven (both MPI plans also call it, mpi_plan/plan.rs:612). Independently verified.
4. multi_worker_walker/wait.rs:286-291 carries the same stale consumer list.
5. petri.rs:4-7 claims per-worker EventLists are projections of the GlobalNet; in code projection reads the ACFG and the net parameter is unused (petri_to_events.rs:284-286). Also petri.rs module doc still cites the ~500-line budget at 790 lines.

Plus the structural half: DELETE the petri_to_events wrapper — zero production callers (tests only), and its stated later-milestones rationale (petri_to_events.rs:47-53) never materialised; make acfg_to_events the only entry point and fix the petri.rs docstring to say the EventList projects from the ACFG.

Discipline: feedback-comment-doc-lie-recurring — while in each file, spot-check the surrounding multi-claim docstrings against the code, not just the cited lines.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 All five witnesses fixed; surrounding doc claims in each touched file spot-checked (list what was checked in notes)
- [x] #2 petri_to_events wrapper deleted, tests migrated to acfg_to_events, grep shows zero remaining references
- [x] #3 just ci green including the doc-citation fences
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Re-grep found the wrapper-delete half ALREADY DONE in a prior cycle: no `fn petri_to_events` exists, acfg_to_events(&ACFG) is the sole one-arg entry point, zero `petri_to_events(` call sites. Several cited doc-lies (event.rs:159-169 transfer_inject path/fresh_seq; event.rs:782-788 typed BuildAcfgError not panic; petri.rs:7-16 projects-from-ACFG; petri.rs ~790-line note) were ALSO already corrected. Genuinely-live remaining doc-lies to fix in my owned files: (a) sidecar.rs:32 "(and *panics* on a non-const bound)"; (b) event_walker.rs THREE/FOUR-implied/FIVE consumer counts -> ground-truth SEVEN backends route through render_worker_events (pthreads-sync/async, openmp-rs direct; mp-tcp-event/mp-uds-event via event_plan; mpi-blocking/mpi-nonblocking via mpi_plan; mp-tcp-bufsync bypasses); (c) wait.rs:286-291 "all three" same stale list; (d) petri.rs:48 ~790->~802; (e) proptest_petri.rs:1757-61 stale two-arg petri_to_events(acfg,_net) signature note. Verify: cargo test -p nucleus-compiler + clippy nucleus-compiler & backend-common.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE FINDINGS (honest, concurrent-tree). Wrapper-delete half: ALREADY DONE by an earlier agent before pickup — no `fn petri_to_events` exists; `acfg_to_events(&ACFG)` is the sole one-arg entry point; zero `petri_to_events(` call sites (grep-verified). Test migration of nucleus-compiler/tests/petri_to_events.rs (delete `petri_wrapper_agrees_with_acfg_entry_point`, route `pipeline_to_events` through acfg_to_events) was done by ANOTHER agent at 10:55 but left a dangling unused `NotifyMode` import that broke `clippy --all-targets` (also filed by yet another agent as TASK-0462) — I COMPLETED that migration by removing only the now-unused NotifyMode symbol (verified 0 remaining uses; all other imports still used). Witnesses 1 (event.rs:159-169 transfer_inject path -> mod.rs/State::fresh_seq) and 2-event-half (event.rs:782-788 typed BuildAcfgError not panic) were ALREADY corrected in the working tree (event.rs mtime 10:57, before my edits) — I did NOT edit event.rs, but VERIFIED both against ground truth (State.next_seq mod.rs:556 + fresh_seq mod.rs:566-571; BuildAcfgError::NonConstLoopBound/OverflowingLoopBound in acfg/errors.rs + build.rs:195-206; transfer_buffer_for_seq is a BTreeMap<SeqTag,u64> field sidecar.rs:239 keyed by SeqTag alone). MY edits (5 files): (2-sidecar) sidecar.rs:32 "(and *panics*)" -> typed BuildAcfgError (TASK-0398); (3) event_walker.rs THREE/FOUR-implied/FIVE consumer claims (module doc, fn doc, fn-body comment) all -> SEVEN backends route through render_worker_events (pthreads-sync/async + openmp-rs direct; mp-tcp-event/mp-uds-event via event_plan; mpi-blocking/mpi-nonblocking via mpi_plan; mp-tcp-bufsync bypasses) — ground-truth verified by grepping call sites; (4) wait.rs:286-291 "all three" -> same SEVEN; (5b) petri.rs ~790 -> ~800 (file is 802 lines); proptest_petri.rs:1754-61 stale two-arg `petri_to_events(acfg,_net)` signature note rewritten to say the wrapper no longer exists. Witness 5 (petri.rs:7-16 projects-from-ACFG) was ALREADY corrected (petri.rs module-doc rewrite present in working tree at pickup). SPOT-CHECKS (comment-doc-lie discipline): petri.rs surrounding claims verified accurate (FireError::CapacityExceeded petri.rs:200; capacity Option<NonZeroU32> petri.rs:105; acfg_to_net acfg_to_petri.rs:279). event_walker.rs strip-mine/partition/check_frame surrounding claims left as-is (not cited witnesses, no contrary evidence; over-editing untested claims risks new lies). VERIFICATION: clippy backend-common --all-targets GREEN; clippy nucleus-compiler --all-targets GREEN (after NotifyMode fix); cargo test -p nucleus-compiler --lib 192/0; --test petri_to_events 26/0; --test proptest_petri 16/0; backend-common --lib 27/0. PRE-EXISTING UNRELATED FAILURE (NOT mine): nucleus-compiler --test sched_lower::positive_reordered_distinct_loop_options_still_lower fails with UnrollUnimplemented{var:x} — caused by another agent in-flight TASK-0458 sched/lower.rs unrollN loud-reject; outside my ownership, untouched by me.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
All five review witnesses fixed and each replacement claim independently re-verified by architect review (GO): SeqTag counter citation -> transfer_inject/mod.rs State::fresh_seq; build_acfg panic claims -> typed BuildAcfgError (event.rs + sidecar.rs); consumer counts -> ground-truth SEVEN backends via render_worker_events (3 direct + event_plan x2 + mpi_plan x2; mp-tcp-bufsync bypasses); petri.rs module doc no longer claims net-projection. Dead petri_to_events wrapper deleted (zero production callers), tests migrated, lib.rs re-export dropped. Review-found silent siblings fixed in fold-in e907f63: parent mod.rs hard counts -> grep-pointer pattern, collect.rs caller census, stale flat transfer_inject.rs citations in backend-common tests. Landed b6869e7 + e907f63; all doc-citation/structural fences green; wave gate 2912/0 + e2e baseline held.
<!-- SECTION:FINAL_SUMMARY:END -->
