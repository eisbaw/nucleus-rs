---
id: TASK-0044
title: M6 — Full tier-1 backend matrix
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-26 10:20'
labels:
  - M6
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-1 milestone capstone: openmp-rs, mp-tcp-poll, mp-uds-event land. All 12 examples × required schedules × all 7 tier-1 backends green. PRD §11.

## Decomposition (cycle 171; orchestrator-direct planning, no code change)

Following the M3 / M4 / M5 capstone pattern (TASK-0041 / TASK-0042 cluster / TASK-0043 cluster), TASK-0044 decomposes into 7 sub-tasks + 2 pre-existing M6-tagged tasks:

| Sub-task           | Scope                                                | Template           | Priority |
| ------------------ | ---------------------------------------------------- | ------------------ | -------- |
| TASK-0044.01       | openmp-rs backend (rayon + shared mem + sync)        | pthreads-sync      | HIGH     |
| TASK-0044.02       | mp-tcp-poll backend (TCP + nonblocking poll + sync)  | mp-tcp-bufsync     | MEDIUM   |
| TASK-0044.02.01    | Schema validator widens notify allow-list to include 'poll' (precursor of .02; cycle 171 architect P2.3 fold-back) | (schema) | HIGH |
| TASK-0044.03       | mp-uds-event backend (UDS + mio/epoll + async + buf) | mp-tcp-event       | MEDIUM   |
| TASK-0044.04       | Example 8 — histogram                                | 03-reduction       | MEDIUM   |
| TASK-0044.05       | Example 10 — wavefront                               | 05-stencil         | LOW      |
| TASK-0044.06       | Example 12 — bitonic sort                            | 02-split-add       | LOW      |
| TASK-0044.07       | M6 capstone (matrix green, depends on all above)     | TASK-0041 / 0042   | MEDIUM   |
| TASK-0053 (extant) | Example 13 (CNN) tier-1 differential                 | (In Progress)      | —        |
| TASK-0054 (extant) | Example 14 (hearing aid) tier-1 naive               | (Done-as-deferred; reopen) | — |

## Execution order

- TASK-0044.01 (HIGH) first: simplest backend, opens up the M6 scope by adding a 3rd sync-shared-memory row.
- TASK-0044.02.01 (HIGH) before TASK-0044.02: the schema-validator widening (allow `notify="poll"`) is the trivial precursor that unblocks the mp-tcp-poll capability surface.
- TASK-0044.02 + TASK-0044.03 in parallel (after .02.01 lands): independent backend crates with disjoint template surfaces.
- TASK-0044.04 / TASK-0044.05 / TASK-0044.06 in parallel: examples only need pthreads-sync to author + smoke-test; no backend dependency.
- TASK-0053 already In Progress; pick up its remaining AC#4 + AC#5 work at M6 entry.
- TASK-0054 needs explicit reopen at M6 entry (its final-summary says exactly that).
- TASK-0044.07 (capstone) closes last; depends on .01..0044.06 + TASK-0053 + TASK-0054.

## Known sublanguage limit risks

- TASK-0044.05 (wavefront) may hit the single-assignment rule (PRD §6.2.1) — close cousin of TASK-0179 (in-array prefix scan). Honest outcome: ship naive only + file grammar-extension follow-up.
- TASK-0044.06 (bitonic sort) may need log²(N)-stage-parametric scheduling; if the schedule sublanguage cannot express it, ship naive + file grammar-extension follow-up.

## Inherited CI runner limitation

`just e2e --milestone M6` is GATE-LOGIC-COMPLETE locally; the literal 'CI runs the full matrix every commit' AC is environment-blocked (no git remote). Standing limitation tracked on TASK-0166 / TASK-0041 / TASK-0057. The M6 capstone closes if the local gate is green; the runner gap is acknowledged + tracked but does not block M6 declaration. Same precedent M3 / M4 / M5 followed.

## Forward-carried defect patterns (apply to every sub-task)

- feedback-comment-doc-lie-recurring + just check-narrative-doc-lie — applies to any [[skip]] reason text + README narrative.
- feedback-silent-sibling-defect + check-siblings — when fixing a defect at one call site, grep for every sibling.
- feedback-textual-replace-codegen-unsafe + just check-textual-replace-on-codegen — never String::replace on a rendered Rust expression.
- feedback-include-str-compile-coverage + just check-include-str-coverage — pair every include_str! with a #[cfg(test)] mod or include!.
- feedback-panic-not-diagnostic-recurring — compiler-pass panics on valid input ship as latent crashes; use EmitError / typed-error surfaces.
- feedback-opacity-gate-rot — when newer precise-tracking machinery lands, audit older deferral facilities for redundancy.

(See ~/.claude/projects/-home-mpedersen-topics-mark-thesis/memory/MEMORY.md for the canonical list.)
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 openmp-rs backend (rayon) lands with capabilities.toml.
- [ ] #2 mp-tcp-poll backend lands (nonblocking sockets, busy/yield poll).
- [ ] #3 mp-uds-event backend lands (Unix domain sockets + mio).
- [ ] #4 Remaining examples (8 histogram, 10 wavefront, 12 bitonic sort) land with reference impls.
- [ ] #5 Examples 13 (CNN inference) and 14 (hearing aid) compile and pass tier-1 differential test.
- [ ] #6 Test: 'just e2e --milestone M6' shows full matrix green.
- [ ] #7 Implementation notes record any examples dropped or rescoped for tier-1 feasibility.
- [ ] #8 Implementation notes record honest limitations (perf is not measured; correctness only).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 175 status update

All 3 M6 backend SKELETON slices landed (cycles 173 openmp-rs / 174 mp-tcp-poll / 175 mp-uds-event); the workspace now has 7 tier-1 backends registered. The decomposition table at the top of this brief still holds; the "execution order" notes are now historical (the sequencing they describe has been executed).

Remaining work on TASK-0044:
- Codegen cycles on each of TASK-0044.01 / 0044.02 / 0044.03 (substantive emit body + e2e cells + bit-identical differential validation).
- TASK-0044.04 / 0044.05 / 0044.06 examples 8 / 10 / 12 (independent of backend codegen — only need pthreads-sync to author).
- TASK-0053 (example 13 CNN tier-1 differential — In Progress, partial).
- TASK-0054 (example 14 hearing aid — needs reopen at M6 entry).
- TASK-0044.07 capstone — closes when all above land + `just e2e --milestone M6` green.

Each skeleton's phased-AC addendum (in the respective TASK-0044.0N notes) records skeleton-cycle ACs DONE vs codegen-cycle ACs PENDING so the next-cycle implementer cannot silently close on un-met ACs.
<!-- SECTION:NOTES:END -->
