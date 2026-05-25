---
id: TASK-0313
title: >-
  Class-wide stale absolute-line citation sweep — Phase B/tail follow-up
  (acfg.rs:697 + driver/main.rs:399 narrative + ~256 long-tail citations;
  TASK-0312 cycle-129 architect P2 #2 honest-scope follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 07:29'
labels:
  - M5
  - tracker-hygiene
  - silent-sibling
  - forward-carried-from-TASK-0312
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0312 (cycle 129) closed Phase A + selected Phase B of the architect-named top-frequency stale-citation sweep: partition_workers.rs:40 (9 sites), driver/src/main.rs:410/413/385 (7 sites), multi_worker_walker.rs:478/404 (5 sites). The cycle-129 mped-architect review-gate P2 #2 flagged honest-scope slippage: TASK-0312's AC#1 wording was maximally scoped ("ONLY inside intentional historical-lesson-preservation records") but execution was selective (~23 sites migrated; ~256 distinct `path/file.rs:NNN` tracker citations remain corpus-wide).

Same SHAPE as the [[feedback-silent-sibling-defect]] meta-recurrence noted in TASK-0311→TASK-0312: closing a class-wide sweep narrowly leaves the broader class as a silent sibling for the NEXT cycle's review-gate to file. TASK-0313 is the explicit follow-up filing AT SWEEP-CLOSE TIME (per the new cycle-129 forward-carry rule on the silent-sibling memory), NOT at next-cycle's review-gate close.

## Specific deferred Phase B targets

### Substantive-claim drift (high-priority within this LOW task)

1. **acfg.rs:697** — 8 stale-line hits across {task-0039 (3 hits), task-0179 (5 hits)}. **Doubly stale**: (a) line 697 today is unrelated; `eval_const` definition is at line 1351, call sites at lines 1089/1096. (b) The SUBSTANTIVE CLAIM ("panics rather than returning a clean LowerError") is itself STALE — the code at lines 1085-1101 has an explicit comment "Typed error, not a panic (TASK-0179)" and returns `BuildAcfgError::NonConstLoopBound`. **TASK-0179 (still listed as To Do) may itself be partially fulfilled** by the panic-to-error conversion that has since landed at the loop-bound site. Migration should:
   - Replace the line citation with a symbolic anchor (e.g. `(search for `BuildAcfgError::NonConstLoopBound` in acfg.rs)`).
   - Append a "Cycle-N PARTIAL CLOSURE ADDENDUM" block to task-0039 + task-0179 noting that the panic-not-diagnostic gap for the loop-bound site has been resolved; other TASK-0179 limitations (out[i-1] underflow with no boundary guard, single-assignment forbidding base-case+loop split, differing-constant-indices) may still be live — VERIFY each by code-read before claiming TASK-0179 closure.
   - If verification confirms all sub-limitations are closed, move TASK-0179 + close TASK-0039 dependency edge.

2. **driver/src/main.rs:399 halo-narrative drift** — 3 hits across {task-0280 (1 hit), task-0285 (2 hits)}. The :399 LINE is still correct today (nuc_trace! call inside the halo_errors_advisory for-loop at lines 396-403). The hits should be SPOT-CHECKED for narrative-tense currency (the task-0280 cite says "Landed cycle 96 (TASK-0275 (B) partition-policy-aware promotion)" — historically-annotated, charitable retention is appropriate). Migrate only if a substitution would strictly improve readability.

3. **Narrative-tense strand at task-0271 :26 + :36** — partially addressed in TASK-0312 cycle 129 via the Cycle-129 STATE-OF-WORLD ADDENDUM appended to task-0271 Description. Cross-check that the addendum sufficiently defends against [[feedback-implementer-disclosure-mechanism-wrong]] for a reader skimming Description-only; if a reader could still reach a wrong conclusion, tighten the addendum's prominence (cross-reference at the FIRST tense marker, not only at the end).

### Line-citation drift (lower-priority tail; selective migration only)

Top remaining offenders after cycle 129:
- halo_inference.rs:53 (~6 hits), halo_inference.rs:361 (~6), halo_inference.rs:89 (~3), halo_inference.rs:682 (~3) — most in task-0307/0308/0311/0312 historical self-references (charitable retention applies per cycle-127 closure). VERIFY each before re-touching.
- lower.rs:189 (~4 hits) — verify currentness.
- common/src/multi_worker_walker.rs:919 (~4 hits) — AXIS-MAPPING ASSUMPTION doc cited across TASK-0301/0302/0306; lines may have drifted with cycle-118/121 transfer_inject changes.
- sync/src/lib.rs:600 (~3), sched/lower.rs:1095 (~3), sched/ast.rs:67 (~3), petri_to_events.rs:113 (~3), lower.rs:109 (~3), halo_inference.rs:411 (~3), d.rs:443 (~3), compiler/src/sched/lower.rs:874+:1109+:1095 (~3 each), compiler/src/passes/partition_workers.rs:40 (~3 — these are absolute paths of the partition_workers.rs:40 set already substantially migrated; remainder may be in the literal-AC retained sites).

## Acceptance criteria

1. **acfg.rs:697 + TASK-0179 substantive-closure verification**: code-read each TASK-0179 limitation against the current acfg.rs / link step / lower.rs to determine which sub-limitations remain live. Document per-sub-limitation status in TASK-0179's Notes. Migrate the line citations to symbolic anchors regardless. If all sub-limitations are closed, close TASK-0179.
2. **driver/src/main.rs:399 + task-0280/0285 spot-check**: leave under charitable retention IF the narrative-tense is already historically annotated; migrate IF a fresh reader could draw a wrong-current-state conclusion.
3. **task-0271 narrative-tense addendum verification** (one-line spot-check): re-read the cycle-129 STATE-OF-WORLD ADDENDUM at task-0271 Description-end; if a Description-only skim still permits the implementer-disclosure-mechanism-wrong shape, prepend a one-line forward-reference at the FIRST tense marker.
4. **Cycle-126/127 substitution discipline** (mandatory): atomic per-string Edit, surrounding-context re-grep, greppability of new anchors, no AC mutation on Done tasks.
5. **Cycle-129 forward-carry** (this task's specific defense): the AC#1 SCOPE WORDING must match execution — choose either "top-30 tail offenders" (bounded) OR "corpus-wide tail" (file the next follow-up at sweep-close if executed selectively). Whichever choice, NAME IT in the AC.

## Cross-references

- TASK-0312 cycle 129 architect P2 #2.
- Memory: [[feedback-silent-sibling-defect]] (cycle 129 forward-carry — file the broader sweep at sweep-close time, not at next-cycle's gate-close).
- Memory: [[feedback-sed-batch-tracker-md-substitution]] (cycle 129 forward-carry — parenthetical annotation alone is insufficient when surrounding narrative-tense markers anchor the citation to a now-stale state).
- Memory: [[feedback-implementer-disclosure-mechanism-wrong]] (the TASK-0271:26 narrative-tense strand fix defends against this pattern).
- Memory: [[feedback-ac-rewrite-on-done-task]] (cycle 126 P3 rule; cycle-129 addendum-block precedent on task-0271 Description).

## Honest scope

LOW priority. Tail-end tracker hygiene. Cycle cost: estimated 1 medium cycle for AC#1 (acfg.rs substantive verification) + 0.5 cycle for AC#2-4. AC#1 is the load-bearing item — it may discover TASK-0179 is partially fulfilled and trigger a TASK-0179 closure cycle.

## Out of scope (deliberate)

- A `just check-tracker-line-citations` lint recipe: still rejected (cycle-127 + cycle-129 architect P3 stance unchanged).
- Wholesale corpus-wide sweep of the ~256 remaining citations: prohibitively wide; cycle-129 closure pattern (architect-named priority sets only, broader tail filed as follow-up) holds.
<!-- SECTION:DESCRIPTION:END -->
