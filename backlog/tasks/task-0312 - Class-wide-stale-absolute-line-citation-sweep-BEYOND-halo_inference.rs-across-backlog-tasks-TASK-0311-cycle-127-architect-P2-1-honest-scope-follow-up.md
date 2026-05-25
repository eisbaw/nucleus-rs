---
id: TASK-0312
title: >-
  Class-wide stale absolute-line citation sweep BEYOND halo_inference.rs across
  backlog/tasks/ (TASK-0311 cycle-127 architect P2 #1 honest-scope follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 06:25'
updated_date: '2026-05-25 07:31'
labels:
  - forward-carried-from-TASK-0311
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0311 (cycle 127) closed the class-wide stale `halo_inference.rs:[0-9]` absolute-line citation sweep across `backlog/tasks/`. The cycle-127 architect (`mped-architect`) review-gate P2 #1 flagged that the SAME DEFECT CLASS fires on OTHER production files cited by tracker md:

- `partition_workers.rs:40` — 10 hits across {task-0249, task-0258, task-0259}; verified STALE (line 40 today is spillover; the "all three PartitionKind variants now have consumers" comment those records describe lives at lines 60-66 today, refactored sometime after TASK-0258/0259 landed).
- `driver/src/main.rs:410` + `:413` — 7 hits across {task-0265, task-0271, task-0280, task-0274:69 (already self-corrected; the remaining 3 task records carry stale citations); `apply_reuse_inference` call site is now at line ~418 (verified via cycle-127 grep).
- Full corpus: `grep -rn '[a-zA-Z_]\+\.rs:[0-9]\+' backlog/tasks/` returns ~365 distinct citations across the project. The top-frequency offenders (partition_workers.rs:40 ×10, multi_worker_walker.rs:478 ×4, driver/main.rs:410 ×4 + :413 ×3) are the high-priority sweep targets.

## Why filed (silent-sibling recurrence at the meta level)

The TASK-0311 AC#1 narrowly enumerated halo_inference.rs; closing it left the structurally identical sibling files silently skipped. Same shape as the [[feedback-silent-sibling-defect]] cycle-127 update: when a "class-wide" sweep is scoped narrowly, the broader class becomes a silent sibling. Filing the broader-sweep follow-up keeps the thread tracked.

## Acceptance criteria

1. `grep -rn '[a-zA-Z_]\+\.rs:[0-9]\+' backlog/tasks/` returns hits ONLY inside intentional historical-lesson-preservation records (same cycle-126 charitable rule that TASK-0311 used).
2. Each migrated citation uses the cycle-122 symbolic-anchor convention.
3. The cycle-126 P1 + P2 #1 substitution-defect lessons MUST be applied during the substitution:
   - Atomic per-string Edit (never sed-batch).
   - Surrounding-context re-grep after each.
   - Greppability verification of every new symbolic anchor (`grep -rn '<anchor>' nucleus/` returns ≥1 hit).
   - No dangling articles, no duplicated articles, no AC inversion, no non-greppable descriptive coinage.
4. The cycle-125 heredoc-quoting discipline applies for any commit shell heredoc.
5. **Sweep ordering** (architect recommendation): start with the highest-frequency offenders (partition_workers.rs:40 ×10, driver/main.rs:410/413 ×7), spot-check freshness on the top-10, then sweep the long tail.

## Honest scope

LOW priority. These citations are reading aids — they drift over months but rarely block work. The cycle-127 cost was 1 cycle for 4 sites of one file's slice; the full corpus is ~365 hits across many files. A realistic single-cycle scope is maybe 30-50 sites (the verified-stale subset). The "tail" of historical-record hits stays under the charitable rule.

## Cross-references

- TASK-0311 cycle-127 architect P2 #1 finding.
- Memory: `feedback-sed-batch-tracker-md-substitution` (cycle-127 epilogue) — substitution discipline confirmed working.
- Memory: `feedback-silent-sibling-defect` (cycle-127 update) — the meta-level recurrence shape.

## Out of scope (deliberate)

- A `just check-tracker-line-citations` lint recipe: the cycle-127 architect P3 noted the cost/benefit is unfavourable as a CI gate (high false-positive rate from legitimate historical records). Do NOT add as part of this task.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Cycle-129 plan (orchestrator-led, 2026-05-25)

Orchestrator-led for the same reason as TASK-0311 cycle 127: ~17 atomic markdown edits across 6 tracker md files with explicit cycle-126/127 lessons-to-apply. Briefing a fresh implementer risks re-introducing the sed-batch / dangling-article defects the lessons forbid.

### Phase A (mandatory): architect-named top-frequency offenders

1. **partition_workers.rs:40** (10 hits across {task-0249, task-0258, task-0259}; line 40 today is the floor-with-spillover remainder-policy bullet; the 'all three PartitionKind variants now have consumers' caveat moved to lines 59-66 inside '## Honest limitations' / '**1D partition axis only.**' bullet)
   - Symbolic anchor: `(module-doc section '## Honest limitations' — the '**1D partition axis only.**' bullet)`
   - Greppability verified: `## Honest limitations` + `**1D partition axis only.**` both 1-hit in nucleus/src/.

2. **driver/src/main.rs:410 + :413** (7 hits across {task-0265, task-0271, task-0280}; task-0274 already self-corrected to `apply_reuse_inference` symbol-grep cycle ~99)
   - Symbolic anchor: `(search for `apply_reuse_inference` in driver/src/main.rs)`
   - Today's actual line: 452 (cycle-127 description's '~418' was already stale by 6 weeks).
   - Greppability verified: 1 hit at driver/src/main.rs:452 + 1 at the use block at line 54.

### Phase B (spot-check next-tier offenders, decide migrate vs charitable-retain):

3. **multi_worker_walker.rs:478 + :404** (5 hits across {task-0273, task-0278, task-0279}; today render_reuse_marker_comment calls are at lines 566 + 671)
   - Symbolic anchor: `(search for `render_reuse_marker_comment` in multi_worker_walker.rs — the non-strip-mine call site is the block_tag=None arm; strip-mine call site is the `if let Some(tag) = block_tag` arm)`

4. **acfg.rs:697** (5 hits across {task-0039, task-0179}; eval_const definition is at line 1351 today; non-const-loop-bound call sites are at lines 1089/1096 AND the historical 'panics rather than diagnoses' claim is also STALE — code at line 1085-1101 already returns `BuildAcfgError::NonConstLoopBound` with explicit comment 'Typed error, not a panic (TASK-0179)').
   - Substantive finding: the panic-not-diagnostic case TASK-0179 documented HAS BEEN FIXED for the loop-bound site (other TASK-0179 limitations may not be).
   - Migration: replace `acfg.rs:697 eval_const, panics rather than diagnosing` with `(see `BuildAcfgError::NonConstLoopBound` in acfg.rs build_seq — the panic was replaced with this typed error; cycle reference inline)`.
   - File a separate FINDING follow-up if scope discovery reveals other TASK-0179 limitations also addressed.

5. **halo_inference.rs:53/89/361** — already addressed by TASK-0311 cycle 127. Remaining hits qualify for cycle-126 charitable retention (in task-0307/0308/0311 self-references).

### Cycle-126/127 substitution discipline (mandatory for every edit)

- Per-string atomic Edit (never sed-batch).
- After each Edit: re-grep the file to verify substitution landed AND did not propagate (P1 #1 dangling article, P1 #3 duplicated article).
- After each Edit: surrounding-context re-read to defend against semantic inversion (P1 #2) and silent ricochet at sibling sites.
- Greppability verification of every new symbolic anchor (>=1 hit in nucleus/src/).
- AC-rewrite-on-Done-task rule: all target tasks except 0258/0259/0265 are Done; per cycle-126 P3 rule, prefer addendum over in-place mutation IF the citation is in a literal AC. **None of the target sites are in literal ACs** (all are in Description / Implementation Notes / Notes); in-place migration is the safer choice (cycle-127 precedent task-0260:132, task-0275:27 both Done-task Description / Notes).

### Verification gate

After all edits land:
1. `grep -rn '[a-zA-Z_/.]\+\.rs:[0-9]\+' backlog/tasks/` returns only intentional historical-record hits.
2. `just build && just clippy && just test && just test-release && just e2e` preserves baseline (108/92/0/16/0).
3. Parallel review gate: qa-test-runner + mped-architect read-only.

### Out of scope (deliberate)

- Long-tail sweep below the top-5 frequency (>250 remaining sites). The cycle-126 charitable rule covers most; the architectural cost / benefit at frequency=2 or 1 is unfavourable.
- A `just check-tracker-line-citations` lint recipe: rejected explicitly in TASK-0312 description (cycle-127 architect P3).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 129 LANDED (orchestrator-led, 2026-05-25)

### What landed (tracker-md hygiene only; production code untouched)

23 atomic per-string Edits across 10 tracker md files + 1 self-update on TASK-0312 + 1 new file TASK-0313. No production code touched.

Files migrated:
- task-0249 (5 sites: Description-quote line 32; Plan step 5; Plan AC-mapping line 85; Notes line 107; Final-Summary line 158)
- task-0258 (2 sites: Plan step 9; Notes line 128)
- task-0259 (2 sites: Plan step file-layout line 89; Notes line 158)
- task-0263 (1 site: Forward-carry line 67; halo-driver counterpart with cycle-96 promotion annotation)
- task-0265 (1 site: P2-2 finding line 138)
- task-0271 (5 sites: Description line 26; Plan steps line 58 + line 59; Notes lines 86 + 103 + 104; Forward-carry line 118; + cycle-129 STATE-OF-WORLD ADDENDUM appended to Description after architect P2 #1)
- task-0273 (2 sites: Per-AC AC#1 line 95; Cycle 98 review-hardening line 123)
- task-0274 (1 site: Description line 28)
- task-0278 (2 sites: Description line 24; Commit hash line 49)
- task-0279 (2 sites within one block: lines 83 + 84)

Retained under cycle-126 charitable rule (literal ACs in Done tasks):
- task-0249 AC#5 line 56
- task-0271 AC#2 line 40
- task-0275 AC#1 line 44
- task-0274 line 69 (self-annotated historical fix-record)

### Per-AC verdict

- **AC#1 — closure metric (corpus-wide "only historical-record")**: GREEN FOR THE ARCHITECT-NAMED PRIORITY SETS only. Post-migration grep returns hits in {3 retained literal ACs, task-0274:69 historical-fix, task-0311 + task-0312 self-references}. AC#1 WORDING exceeds EXECUTION on the long tail (~256 corpus citations un-touched); the mismatch is recorded as cycle-129 architect P2 #2 + filed as TASK-0313 at sweep-close time (per cycle-129 forward-carry to [[feedback-silent-sibling-defect]]).
- **AC#2 — cycle-122 symbolic-anchor convention**: GREEN. New anchors ("## Honest limitations", "**1D partition axis only.**", "apply_reuse_inference", "apply_halo_inference_partition_aware", "render_reuse_marker_comment") all verified ≥1 hit in nucleus/src/ (re-verified by qa-test-runner GO).
- **AC#3 — cycle-126/127 substitution-defect lessons**: GREEN. Per-string atomic Edit + surrounding-context re-grep + greppability check on every new anchor. Zero recurrence of dangling-article / duplicated-article / AC-inversion / non-greppable-coinage defects. One mild grammatical redundancy at task-0249 line 107 ("comment at ... head-comment") was caught by architect P3 #1 and fixed in-thread before close.
- **AC#4 — cycle-125 heredoc-quoting**: GREEN (vacuous; no shell heredoc this cycle).
- **AC#5 — sweep ordering**: GREEN. Architect-named top-frequency offenders (partition_workers.rs:40 ×10, driver/main.rs:410/413/385 ×8, multi_worker_walker.rs:478/404 ×5) all completed.

### Review gate

Parallel read-only review:
- **qa-test-runner: GO**. just build clean; just clippy clean (-D warnings); just test 854/0/3 dev; just test-release 854/0/3 (dev/release parity); just e2e 108/92/0/16/0 (M5 baseline preserved). No P1/P2/P3 defects.
- **mped-architect: GO with P2 #1, P2 #2, P3 #1, P3 #2, P3 #3** — all addressed in-thread before close:
  - P2 #1 (narrative-tense strand at task-0271:26): RESOLVED by appending cycle-129 STATE-OF-WORLD ADDENDUM to task-0271 Description.
  - P2 #2 (honest-scope slippage + TASK-0313 needs filing): RESOLVED by filing TASK-0313 covering Phase B (acfg.rs:697 substantive-claim drift; driver/main.rs:399 narrative drift; ~256 long-tail).
  - P3 #1 (grammatical redundancy at task-0249:107): FIXED in-thread.
  - P3 #2 (feedback-sed-batch-tracker-md-substitution forward-carry — narrative-tense rule): MEMORY UPDATED with cycle-129 epilogue.
  - P3 #3 (feedback-silent-sibling-defect forward-carry — scope-wording-execution mismatch as recurring meta-failure): MEMORY UPDATED with cycle-129 entry.

### Gotchas + lessons forward-carried (for TASK-0313)

1. **Narrative-tense strands defy parenthetical annotation alone**. When migrating a citation embedded in prose carrying "current" / "today" / "now" markers, parenthetical annotation may leave the reader with a wrong-current-state conclusion. Either rewrite the tense marker to past-tense ("at file time") OR append a STATE-OF-WORLD ADDENDUM block. This is the cycle-129 task-0271 case + the [[feedback-implementer-disclosure-mechanism-wrong]] defense. Folded into [[feedback-sed-batch-tracker-md-substitution]] cycle-129 epilogue.

2. **Scope-wording-execution mismatch is recurring meta-failure**. The chain TASK-0311 → 0312 → 0313 demonstrates: each cycle's narrow scope spawns the next-tier sibling. Pathology is not narrow scope itself but AC#1 wording exceeding execution. Hygiene rule: at orchestrator review-pass closure, BEFORE marking parent Done, file the next-tier follow-up if AC#1 wording exceeds execution. Folded into [[feedback-silent-sibling-defect]] cycle-129 entry. Recurrence count now: 7 cycles (93, 95, 97-98, 116, 127, 129).

3. **acfg.rs:697 substantive-claim drift discovered, deferred**. The 8 hits across {task-0039, task-0179} citing "acfg.rs:697 eval_const, panics rather than returning a clean LowerError" carry STALE on BOTH line (eval_const definition is at acfg.rs:1351; call sites at 1089/1096) AND CLAIM (the code at 1085-1101 explicitly returns BuildAcfgError::NonConstLoopBound with comment "Typed error, not a panic (TASK-0179)"). **TASK-0179 (still listed as To Do) may itself be partially fulfilled** by this fix. Deferred to TASK-0313 because the substantive-claim verification is a per-sub-limitation code-read (out[i-1] underflow guard; single-assignment base-case+loop split; differing-constant-indices) — its own bounded investigation.

4. **task-0271 cycle-129 STATE-OF-WORLD ADDENDUM precedent**. The cycle-129 task-0271 addendum is the first instance of using a discrete ADDENDUM block to defend against [[feedback-implementer-disclosure-mechanism-wrong]] in a Done task's Description. Pattern: append after the Description's section list (here: after AC list at line 43), name the load-bearing tense-strand sites + today's state explicitly, cross-link the relevant feedback-* memory. AC-rewrite-on-Done-task compliant.

5. **Orchestrator-led was the right call (cycle-127 + cycle-129 precedents agree)**. ~23 atomic markdown edits across 10 files with explicit cycle-126/127 lessons-to-apply is the exact shape where briefing a fresh implementer subagent carries higher risk (mechanical generalisation re-introducing the forbidden defects) than orchestrator-context risk. Mandatory parallel review gate still ran and confirmed.

### TASK-0313 forward-carried context (filed at sweep-close per cycle-129 [[feedback-silent-sibling-defect]] rule)

- AC#1: acfg.rs:697 substantive-claim verification (per-sub-limitation code-read of TASK-0179 against current acfg.rs / link / lower.rs; may trigger TASK-0179 closure).
- AC#2: driver/main.rs:399 + task-0280/0285 spot-check (charitable retention if already historically annotated).
- AC#3: task-0271 cycle-129 STATE-OF-WORLD ADDENDUM verification (re-read after gate close; tighten prominence if Description-only skim still permits implementer-disclosure-mechanism-wrong shape).
- AC#4: substitution discipline mandatory (cycle-126/127/129 lessons including narrative-tense rule).
- AC#5: AC#1 SCOPE WORDING must match execution — explicit scope-stop with named follow-up.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0312 LANDED cycle 129 (2026-05-25). Class-wide stale absolute-line citation sweep across backlog/tasks/ — Phase A (architect-named top-frequency offenders) + selected Phase B (multi_worker_walker.rs:478/404) completed via 23 atomic per-string Edits across 10 tracker md files. Symbolic anchors per cycle-122/127 convention, greppability verified, cycle-126 substitution discipline preserved (zero recurrence of dangling-article / duplicated-article / AC-inversion / non-greppable-coinage defects). Cycle-129 architect P2 findings addressed in-thread: P2 #1 narrative-tense strand at task-0271 resolved by STATE-OF-WORLD ADDENDUM defending against [[feedback-implementer-disclosure-mechanism-wrong]]; P2 #2 honest-scope slippage resolved by filing TASK-0313 (acfg.rs:697 substantive-claim drift + driver/main.rs:399 narrative drift + ~256 long-tail citations) AT SWEEP-CLOSE TIME per new [[feedback-silent-sibling-defect]] cycle-129 rule. Production code untouched; gate preserved (just test 854/0/3 dev + release; just e2e 108/92/0/16/0). Memory updated: [[feedback-sed-batch-tracker-md-substitution]] cycle-129 epilogue (narrative-tense rule); [[feedback-silent-sibling-defect]] cycle-129 entry (recurrence count 7; scope-wording-execution mismatch as recurring meta-failure). Honest scope: AC#1 wording exceeded execution on the corpus-wide tail; TASK-0313 carries the closure record.
<!-- SECTION:FINAL_SUMMARY:END -->
