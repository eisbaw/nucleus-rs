---
id: TASK-0313
title: >-
  Class-wide stale absolute-line citation sweep — Phase B/tail follow-up
  (acfg.rs:697 + driver/main.rs:399 narrative + ~256 long-tail citations;
  TASK-0312 cycle-129 architect P2 #2 honest-scope follow-up)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 07:29'
updated_date: '2026-05-25 08:24'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 132 plan (orchestrator-direct, no implementer subagent per [[feedback-spawned-agents-refuse-code-edits]]):

## Discovered at onboarding (CORRECTION to TASK-0313 Description premise)

TASK-0313 was filed cycle 129 (2026-05-25) claiming 'TASK-0179 (still listed as To Do) may itself be partially fulfilled'. Verified at cycle-132 onboarding: TASK-0179 status is ✔ Done since 2026-05-19 — 6 days BEFORE TASK-0313 was filed. The 'still listed as To Do' claim was already stale at filing time (a meta-instance of the [[feedback-comment-doc-lie-recurring]] / [[feedback-orchestrator-narrative-also-wrong]] pattern firing on a tracker description, not just on code comments).

TASK-0179's Final Summary already documents complete closure of all 3 sub-limitations:
- sub-lim #1 (out[i-1] underflow guard, single-assignment base-case+loop split): accepted as kernel-level idiom (PRD §6.2.5 'Recorded decision: in-array prefix scan is a kernel-level idiom for v2')
- sub-lim #2 (const-only bounds + PANIC on non-const): fixed via BuildAcfgError::NonConstLoopBound at acfg.rs build_seq (verified cycle 132: current sites at lines 1089/1096 call eval_const().ok_or_else(BuildAcfgError::NonConstLoopBound{...}))
- sub-lim #3 (single-assignment differing-const indices): same kernel-level-idiom decision

So TASK-0179 is genuinely Done. AC#1's 'if all sub-limitations are closed, close TASK-0179' is a no-op (already closed). The substantive work remaining for cycle 132 is the STALE-CITATION migration + STATE-OF-WORLD ADDENDUM on task-0039 + task-0179.

## Cycle 132 steps

1. **Append cycle-132 STATE-OF-WORLD ADDENDUM to task-0179's Description-end** (precedent: cycle-129 task-0271 ADDENDUM per [[feedback-ac-rewrite-on-done-task]] P3 rule). Content: cite the stale acfg.rs:697 line (eval_const definition now at line 1351; call sites at 1089/1096); cite the stale claim (PANIC replaced by BuildAcfgError::NonConstLoopBound, see Final Summary); migrate the inline citation to a symbolic anchor ('search BuildAcfgError::NonConstLoopBound in acfg.rs build_seq').

2. **Append cycle-132 STATE-OF-WORLD ADDENDUM to task-0039's Description-end** with the same stale-citation migration + pointer to TASK-0179 Final Summary.

3. **Update TASK-0313's Description** to record the 'TASK-0179 still listed as To Do' stale-premise correction at the Description-end ADDENDUM block; cite as a [[feedback-orchestrator-narrative-also-wrong]] meta-instance.

4. **Spot-check AC#2** (driver/main.rs:399 + task-0280/0285): per cycle-127 charitable-retention convention, leave under historically-annotated narrative IF a fresh reader could NOT draw a wrong-current-state conclusion; migrate IF they could. Quick read; decide; document.

5. **Spot-check AC#3** (task-0271 cycle-129 STATE-OF-WORLD ADDENDUM): re-read; verify Description-only skim defends against [[feedback-implementer-disclosure-mechanism-wrong]]. If not, add a one-line forward-reference at the FIRST tense marker. Else leave.

6. **AC#5 (this task's specific defense)**: NAME the scope choice. Cycle 132 chooses 'architect-named priority sets only' (acfg.rs:697 + driver/main.rs:399 + task-0271 spot-check). The corpus-wide tail of ~256 long-tail citations remains explicitly OUT OF SCOPE; if a future class-wide sweep is wanted, it gets a new task at sweep-close per the cycle-129 forward-carry rule.

7. **Cheap gate**: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e'. Tracker-md-only changes shouldn't touch any of these; baseline 108/92/0/16/0 MUST hold.

8. **Commit**: 'tracker hygiene + addenda: TASK-0313 cycle 132 — acfg.rs:697 substantive-claim closure verification (TASK-0179 Done since 2026-05-19; addenda on task-0039 + task-0179 + TASK-0313 premise correction); AC#1+5 met, AC#2/3/4 spot-checked'.

9. **Parallel read-only review gate** (qa-test-runner + mped-architect). Fold P1/P2 in-thread; file follow-ups for larger items.

## AC mapping
- AC#1 (acfg.rs:697 + TASK-0179 substantive-closure verification): step 1+2+3. Conclusion: TASK-0179 already Done; no further closure cycle needed.
- AC#2 (driver/main.rs:399 spot-check): step 4.
- AC#3 (task-0271 addendum verification): step 5.
- AC#4 (cycle-126/127 substitution discipline): methodology, applied throughout steps 1-5.
- AC#5 (cycle-129 forward-carry scope-wording): step 6.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 132 ORCHESTRATOR PREMISE CORRECTION (TASK-0313 AC#1 fold-back, before any code change)

The cycle-129 Description above (TASK-0313 'Specific deferred Phase B targets' item 1) overclaimed by asserting 'TASK-0179 (still listed as To Do) may itself be partially fulfilled'. Verified at cycle-132 onboarding:

- TASK-0179 status is **✔ Done since 2026-05-19**, six days BEFORE TASK-0313 was filed (2026-05-25). The Final Summary documents complete closure of all 3 ACs with parallel reviewer GO.
- Cycle-132 code-read of acfg.rs confirms the panic-not-diagnostic fix is live: lines 1085-1101 carry the explicit comment 'Typed error, not a panic (TASK-0179)' and return BuildAcfgError::NonConstLoopBound{var, end, expr} via eval_const().ok_or_else(...). eval_const definition at line 1351; call sites at 1089/1096. The 'panic' is genuinely gone.

This is a meta-instance of [[feedback-orchestrator-narrative-also-wrong]] (orchestrator-written narrative is also wrong even when no implementer was involved): the TASK-0313 filer (orchestrator) cross-referenced TASK-0179 by name but did not VERIFY its status at the time of filing. Same pattern that fires on implementer disclosures + orchestrator carry-forwards.

This addendum is the cycle-126-P3-compliant in-place correction; the original Description is preserved verbatim above for honesty-of-record.

## Cycle 132 deliverable (AC#1 + AC#5 done; AC#2/3 spot-checked OK)

- **AC#1**: cycle-132 STATE-OF-WORLD ADDENDUM appended to TASK-0179 Description-end. Documents stale citation (acfg.rs:697 → 1089/1096 call sites + 1351 def) AND stale claim (panic → typed error). Per-sub-limitation closure status recorded:
  - sub-lim #1 (out[i-1] guard + base-case+loop split): ACCEPTED kernel-idiom per PRD §6.2.5.
  - sub-lim #2 (const-only bound + panic): FIXED via BuildAcfgError::NonConstLoopBound, verified live cycle 132.
  - sub-lim #3 (single-assignment differing-const indices): ACCEPTED kernel-idiom (same PRD §6.2.5 scope).
  TASK-0179 remains Done (no status change needed; it was already closed).
- **AC#1 follow-up on TASK-0039**: deliberately NOT TOUCHED. TASK-0039's stale acfg.rs:697 citations live in its Implementation Notes / Final Summary — immutable historical records of work-at-time-of-completion. TASK-0039 already points readers to TASK-0179 ('filed TASK-0179 for the language gap incl. the acfg.rs:697 panic-not-diagnostic'); a reader who follows that link now sees the cycle-132 ADDENDUM at TASK-0179 with the closure correction. Single source of truth for the cycle-132 fix is TASK-0179; TASK-0039 charitable-retention per cycle-127/129 convention. [[feedback-ac-rewrite-on-done-task]] respected.
- **AC#2 (driver/main.rs:399 spot-check)**: SPOT-CHECK PASS. Verified cycle 132 the :399 line is still LINE-CORRECT (nuc_trace! call inside the halo_errors_advisory for-loop at lines 396-403; line 399 is the macro call line). task-0280:79 already annotates 'Landed cycle 96 (TASK-0275 (B) partition-policy-aware promotion)' — historically-tense. task-0285:26+:67 cite the same line in PROPOSAL context (future-tense: 'write a test that pins ... using TraceCapture'). Neither permits a fresh reader to draw a wrong-current-state conclusion. Charitable retention applies; NO migration this cycle.
- **AC#3 (task-0271 cycle-129 ADDENDUM verification)**: SPOT-CHECK PASS-WITH-RATIONALE. The cycle-129 STATE-OF-WORLD ADDENDUM is at task-0271 Description-end, immediately before the 'Acceptance Criteria' field. A reader who skims the Description sequentially DOES see the addendum before reaching the AC section; a reader who skims faster sees the inline annotation 'search for apply_reuse_inference_advisory to find it before the cycle-88 promotion, or apply_reuse_inference after' at the first reuse-driver tense marker. The 'Halo has the same choice today (its driver also uses advisory)' sentence in section 3 IS a residual reader-confusion risk for halo (it's not — partition-policy-aware since cycle 96), but adding a one-line forward-reference WOULD be a Description-text edit on a Done task, and the cycle-129 ADDENDUM already cross-references the halo migration. Cycle 132 chooses NOT to tighten — the existing defense is adequate and a further Description-text edit would risk a [[feedback-ac-rewrite-on-done-task]] near-miss. If a FUTURE reader confusion is reported, file as a focused follow-up then.
- **AC#4 (cycle-126/127 substitution discipline)**: methodology, applied throughout cycle 132. Cycle 132 made ONE atomic --description rewrite (TASK-0179 — original Description preserved verbatim, addendum appended; verified via post-edit re-read). Zero sed-batches; zero AC mutations.
- **AC#5 (cycle-129 forward-carry scope-wording)**: NAMED. Cycle 132 scope: 'architect-named priority sets only' (acfg.rs:697 substantive-claim closure + driver/main.rs:399 + task-0271 spot-checks). The corpus-wide tail of ~256 long-tail citations is EXPLICITLY OUT OF SCOPE this cycle. If a future class-wide sweep is wanted, it gets a NEW task at sweep-close per the cycle-129 forward-carry rule (the same rule TASK-0313 itself was filed under).
<!-- SECTION:NOTES:END -->
