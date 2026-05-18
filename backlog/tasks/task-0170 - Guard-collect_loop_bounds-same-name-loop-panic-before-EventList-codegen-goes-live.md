---
id: TASK-0170
title: >-
  Guard collect_loop_bounds same-name-loop panic before EventList codegen goes
  live
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 23:26'
updated_date: '2026-05-18 23:32'
labels:
  - M2
  - compiler
  - robustness
  - fail-fast
dependencies:
  - TASK-0160
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect reviews of TASK-0160 and TASK-0169 (P2): build_sidecar/collect_loop_bounds in nucleus/compiler/src/sidecar.rs HARD-PANICS at compiler runtime if two loops share an IterVar (same loop-var name, PRD 6.2.3 one namespace) but have DIFFERENT bounds (keeps first; idempotent if identical). No current example (01/02/03/05/07) hits it, so it is a latent panic on a class of otherwise-valid input, currently tracked only as prose breadcrumbs across TASK-0124/0167 notes — not a first-class item. Per fail-fast discipline this must be a real guarded path before the EventList-only backend (TASK-0124) consumes loop_bounds. Decide: (a) make the same-name-different-bounds case a typed compile error surfaced via the driver (not a panic), or (b) prove it impossible upstream (lowering already rejects it) and add a should_panic/characterisation test pinning the contract, or (c) make Event::Loop/loop_bounds key on something that distinguishes the two loops. Add a regression/characterisation test either way.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Same-name-loop-differing-bounds is either a typed driver-surfaced error OR proven-impossible-upstream with a pinning test (no bare compiler panic on valid input)
- [ ] #2 A characterisation/regression test pins the chosen contract
- [ ] #3 TASK-0124 EventList path cannot reach the bare panic
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Empirically determine reachability: parse->lower->build_acfg->build_sidecar with two sibling for-loops reusing var `i` with DIFFERENT bounds (0..N then 0..M). SEE if lower_algo accepts it.
2. If reachable (likely per PRD 6.2.3 + lower.rs only rejecting const/data shadow): implement option (a) - SidecarError typed enum, build_sidecar -> Result<NameSidecar, SidecarError>, Display impl, std::error::Error. Update test callers + lib re-export. Wire into driver path (currently no caller) defensively via a comment for TASK-0124.
3. AC#2: characterisation/regression test in tests/petri_to_events.rs pinning the typed error for same-name-diff-bounds; plus a positive test that same-name-SAME-bounds is idempotent (no error).
4. AC#3: comment at the panic site + sidecar.rs doc; file option (c) follow-up task (deep IterVar-identity redesign) with dep edge task-0170 -> new.
5. Full gate inside nix develop: just test/e2e/determinism-check/-negative/clippy. e2e+determinism MUST be unchanged (no codegen consumer).
6. Commit per logical unit. Forward-carry findings to TASK-0124 notes.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
REACHABILITY FINDING (empirical, via tests/zz_probe_0170.rs probe):
- A VALID Nuc program reaches the panic. Two sequential sibling loops both named `i`, bounds `0..N` and `0..M` (N!=M consts), writing DISTINCT data arrays (so single-assignment holds), is accepted by parse_algo + lower_algo + link + build_acfg.
- lower.rs only rejects a loop var shadowing a declared const/data/kernel; same-name loop REUSE across siblings is allowed (PRD 6.2.3: loop vars one namespace, shadow at loop scope).
- name_iter_vars = {"i": IterVar(0)} -> ONE IterVar shared by both loops; build_sidecar then panics at sidecar.rs:401 "loop var `i` reused with DIFFERENT bounds".
- Option (b) [prove-impossible-upstream] is REFUTED. Implementing option (a): typed SidecarError from build_sidecar, surfaced cleanly (no bare panic on valid input). Filing option (c) [deep IterVar-identity redesign] as a follow-up with dep edge.
- Note: single-assignment DOES reject two loops writing the SAME data array in the same-named scope (DoubleAssignment, scope keyed "for i"), so the reachable witness must write distinct arrays.
Resolves a TASK-0124 open question: the EventList path CAN reach this on valid input -> must be a typed error before TASK-0124 goes live.
<!-- SECTION:NOTES:END -->
