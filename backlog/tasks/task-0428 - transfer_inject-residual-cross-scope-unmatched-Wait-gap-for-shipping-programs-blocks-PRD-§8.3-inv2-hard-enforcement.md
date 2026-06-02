---
id: TASK-0428
title: >-
  transfer_inject: residual cross-scope unmatched-Wait gap for shipping programs
  blocks PRD §8.3 inv(2) hard-enforcement
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 10:04'
updated_date: '2026-06-02 16:07'
labels:
  - compiler
  - event-contract
  - transfer_inject
  - prd-invariant-audit
  - cycle-242
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Encodes the prerequisite that TASK-0422 currently only NARRATES in prose (cycle-241 GAP-2 deferral). PRD §8.3 inv(2) (Push/Wait events form matched pairs) cannot be hard-asserted on shipping output, and the full validator validate_event_lists cannot be wired to a production caller, because transfer_inject still leaves LEGITIMATE unmatched Wait events for currently-shipping programs (reproducer: 02-split-add). Hard-asserting inv(2) today would crash debug builds on VALID input — the panic-on-valid-input class this project rejects (see event_validate.rs deferral rationale + petri_to_events.rs strict-subset debug_assert that EXCLUDES inv(2)).

PRIOR ART that did NOT fully close this: TASK-0136 (splice Push across Sequence/Repeat boundaries), TASK-0149 (splice across nested sequences for hoisted Waits), TASK-0151 (cross-scope finalisation gate is whole-program coarse), TASK-0364 (scope-aware let-at-wait classification / typed EmitError) are ALL Done, yet TASK-0422 (filed AFTER them, 2026-06-02) verified the residual unmatched-Wait still occurs. So this is a residual structural gap beyond those, not a duplicate.

SCOPE: (1) characterise WHY 02-split-add (and any sibling shipping program) still produces an unmatched Wait after all splice work landed — static trace, do not assume; (2) close it at root so unmatched Waits no longer occur for valid programs, OR prove it is a fundamental property of the event model and TASK-0422 must instead reshape inv(2) enforcement (e.g. a participant-scoped matched-pair check). On closure, TASK-0422 becomes actionable (wire validate_event_lists as the EventList-consuming backends entry gate) and TASK-0423 (SyncTag participant-set agreement) follows. Pointers: src/passes/transfer_inject/, src/event_validate.rs (validate_event_lists + :48-84 deferral), src/passes/petri_to_events.rs:225-241 (strict-subset debug_assert).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Investigation plan (cycle-242): (1) Run real 02-split-add pipeline via tests/petri_to_events.rs::full_pipeline_acfg -> acfg_to_events, then call event_validate::validate_event_lists on the result; record EXACT Ok/Err. (2) Branch: if Ok, premise stale (TASK-0136 et al. closed it) -> sweep multi-worker examples corpus for any unmatched pair, fix stale docstrings in event_validate.rs + petri_to_events.rs, unblock TASK-0422; if Err, trace root scope-boundary splice_pushes_for_waits misses + decide root-fix-vs-reshape. Throwaway probe first; keep as regression pin if it encodes the finding.

FINDING (cycle-242, commit e2a4ecd): the deferral premise is STALE. Ran the REAL front-end pipeline (parse->lower->link->build_acfg->block_transforms->partition_{workers,rows,blocks2d}->halo->reuse->inject_syncs->inject_transfers->acfg_to_events) and called validate_event_lists (FULL surface incl inv(2)).

EMPIRICAL GROUND TRUTH:
- 02-split-add (the cited reproducer): validate_event_lists => Ok(()). a/b input Waits are hoisted OUT of the `for` by TASK-0136 Pass A and matched by host-side Pushes; c is pushed by w0 and matched by host Wait. No unmatched Wait.
- ENTIRE example corpus: 55/55 schedules inv(2)-OK, 0 violations, 0 pipeline-errored. (Each schedule resolved to its `schedule for ".."` algo — 7 pair with prog.<variant>.algo.nuc not the default; my first sweep wrongly forced prog.algo.nuc and produced 7 spurious LinkErrors, fixed.)

So TASK-0136 + 0149/0151/0364 already closed the cross-scope splice gap. The decision: branch A (premise stale). NO root-fix needed; NO inv(2) reshape needed.

DELIVERED: two regression pins in tests/petri_to_events.rs (task0428_inv2_holds_for_entire_example_corpus broad sweep + task0428_inv2_clean_for_02_split_add_reproducer named). Corrected the stale docstrings in event_validate.rs (module doc + strict_per_worker doc) and petri_to_events.rs (module doc + acfg_to_events debug_assert rationale).

HONEST LIMIT (forward-carried to TASK-0422): the sweep covers the backend-agnostic pthreads-{sync,async} chain. The mp-tcp-{bufsync,event,poll} / mp-uds-event backends additionally run host_mediation_inject + host_data_relay_inject AFTER inject_transfers; those re-route Push/Wait through host and were NOT exercised. inv(2) over THAT post-mediation EventList is NOT proven here. The acfg_to_events debug_assert was deliberately LEFT as the per-worker subset for that reason (a) + because gate-wiring is TASK-0422 scope (b) — NOT the stale reason.

GATE: build+clippy clean; test dev 1256/0/3 (+2); test-release 1254/0/3 (+2); e2e 385/328/0/57/0 (unchanged, docs+tests only).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
CHARACTERIZATION DELIVERABLE (branch A: premise stale). The residual cross-scope unmatched-Wait gap this task was filed to characterize DOES NOT EXIST for any shipping program. transfer_inject (post TASK-0136 Pass A hoist + Pass B cross-scope splice, with siblings 0149/0151/0364) produces matched Push/Wait pairs; verified empirically via the real front-end pipeline that validate_event_lists returns Ok on 02-split-add and across all 55 example-corpus schedules (0 violations). Outcome: stale docstrings corrected (event_validate.rs + petri_to_events.rs), two regression pins added, TASK-0422 unblocked with a concrete two-step path (confirm post-mediation inv(2) for mp-* backends, then wire validate_event_lists). Honest residual: the proof covers the pthreads-{sync,async} backend-agnostic chain; the mp-tcp/uds host_mediation_inject + host_data_relay_inject post-passes are unverified and carried to TASK-0422 step (1). Commit e2a4ecd. Gate: dev 1256/0/3, release 1254/0/3, e2e 385/328/0/57/0.
<!-- SECTION:FINAL_SUMMARY:END -->
