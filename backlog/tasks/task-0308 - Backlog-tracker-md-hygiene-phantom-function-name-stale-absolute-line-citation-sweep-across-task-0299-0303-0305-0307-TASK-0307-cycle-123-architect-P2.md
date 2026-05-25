---
id: TASK-0308
title: >-
  Backlog tracker md hygiene: phantom function-name + stale absolute-line
  citation sweep across task-0299/0303/0305/0307 (TASK-0307 cycle-123 architect
  P2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 04:32'
updated_date: '2026-05-25 05:55'
labels:
  - M5
  - backlog-hygiene
  - comment-doc-lie
  - forward-carried-from-TASK-0307
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0307 cycle-123 (the structural sentinel that closed the Option B vacuous-pass arm) ALSO closed the cycle-122 phantom-function-name citation + stale absolute-line citation pattern across the SOURCE / TEST files in `nucleus/`. The orchestrator's `replace_all` was scoped to `nucleus/` only.

The cycle-123 architect review-gate flagged the same stale-citation pattern in the TRACKER markdown files — the orchestrator's durable narrative memory. The replace_all DID NOT touch them.

## What is stale (as of cycle 123, pre-fix)

12-13 stale citation sites across 5 task markdown files (task-0299, task-0303, task-0305, task-0307, task-0308 itself):

- The phantom-function-name pattern in cross-references + text-search hints.
- Stale absolute-line citations into the `halo_inference.rs` contract paragraph (the cycle-119 line range), the production emit-site line (`per_iv.entry(iv).or_insert(0)` line citation), and the `no_halo_bare_iv` test line.

(Exact line numbers may shift as the tracker files evolve. The defect class is the citation rot, not the specific lines.)

## Acceptance criteria

1. `grep -rn` for the phantom-function-name pattern across `backlog/tasks/` returns hits ONLY inside intentional historical-lesson-preservation records (task-0307 notes/final-summary that document the cycle-122 mistake by name as a recurrence-pattern marker). Cleanup did NOT touch those (lesson trail).
2. `grep -rn` for the cycle-119 absolute-line range citation (the contract paragraph) returns the same: hits only inside historical-lesson-preservation records.
3. `grep -rn` for the halo-entry-sink absolute-line citation returns the same.
4. `grep -rn` for the `no_halo_bare_iv` test absolute-line citation returns the same.
5. Each migrated citation uses the SYMBOLIC search-hint convention (cycle-122 lesson): symbolic anchors like `per_iv.entry(iv).or_insert(0)` text search hint, paragraph-title search hints (`search for "absent ≡ explicit-0"`), or `fn no_halo_bare_iv` symbolic anchor.
6. All edits applied via `backlog task edit` (CLAUDE.md project hygiene rule: never hand-edit task markdown files; route through the CLI).

## Honest scope

LOW priority. Tracker docs are the orchestrator's narrative memory; the citation rot will re-surface on the next backlog-driven cycle as the same recurrence-pattern (the next implementer brief that loads these tasks via `backlog task view` will inherit them). Pure hygiene; no production-code or test impact.

## Charitable AC interpretation (cycle 126)

The original AC#1-4 specified ZERO grep hits, but historical-lesson-preservation lines in task-0307 notes/final-summary literally record the cycle-122 mistake by name as a recurrence-pattern marker (the `feedback-comment-doc-lie-recurring` lesson trail). Strictly scrubbing those would erase the lesson punch. Cycle-126 followed the cycle-122 precedent (TASK-0307 AC#3 charitable interpretation that the cycle-123 architect accepted): scrub LIVE/CURRENT citations (in descriptions, ACs, cross-references, implementation plans) and preserve HISTORICAL records that name the mistake by literal symbol. Documented in cycle-126's final summary.

## Cross-references

- TASK-0307 cycle 123 architect review-gate NEW P2 #2 + NEW P2 #3 — the cycle that filed this follow-up.
- Memory: `feedback-silent-sibling-defect` — work pinning one visible arm while structurally identical sibling silently skips the fix. This task is the across-code-tracker-boundary instance of that pattern.
- Memory: `feedback-comment-doc-lie-recurring` — the phantom-function-name citation is exactly the symbol-that-doesn't-exist comment-doc-lie pattern.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-126 architect review-gate (read-only) returned NO-GO with 3 P1 + 1 P2 substitution-defect findings + 1 P2 #2 class-wide silent-sibling-defect finding. Fold-back applied in-thread (same commit cycle):

P1 #1 task-0305 SHIPPED-block broken file-path header (the substitution turned `nucleus/nucleus-compiler/src/passes/halo_inference.rs:53-71 — ...` into a string with a dangling directory prefix off a noun phrase). Fixed: rewritten as `nucleus/nucleus-compiler/src/passes/halo_inference.rs (Option B contract paragraph; search for "absent ≡ explicit-0") — ...`.

P1 #2 task-0307 AC#3 semantic inversion (substitution made the AC say "replace the current correct anchor", opposite of cycle-123 charitable interpretation). Fixed: rewrote AC#3 to past-tense disclosure recording what was charitably accepted, not imperative-future for a Done task.

P1 #3 task-0307 AC#2 duplicated article "the production the halo-entry sink". Fixed: rewrote as `the production per_iv.entry(iv).or_insert(0) emit site inside classify_index`.

P2 #1 the descriptive phrase "halo-entry sink" coined during cycle-126 was NON-GREPPABLE (grep -rn "halo-entry sink" nucleus/ returns zero) — exactly the new citation-rot vector the cycle was supposed to defend against. Fixed: every "halo-entry sink" coinage replaced with the greppable production form (per_iv.entry(iv).or_insert(0) — which grep -rn "per_iv.entry(iv).or_insert(0)" nucleus/ returns hits for).

P2 #2 class-wide silent-sibling-defect: AC#1-4 enumerated 4 literal patterns but the defect CLASS is broader (other absolute-line citations into halo_inference.rs at task-0260:132, task-0263:85, task-0275:27, task-0309:48). In-file same-class sibling task-0305:34 + :73 (halo_inference.rs:1184) fixed in cycle-126 fold-back as part of the editing pass already touching task-0305. Cross-file siblings filed as TASK-0311 (LOW; same charitable AC interpretation applies; the cycle-126 P1 + P2 #1 substitution-defect lessons explicitly carried as cycle-311 AC#3 to avoid the meta-recurrence).

Recurring-defect-catalog disclosure (cycle-126 architect findings explicitly invoke the lesson at task-0307:99): "the highest comment-doc-lie risk is on comments whose stated purpose is doc-lie defence". Cycle 126 fired this lesson FOUR TIMES (P1 #1-3 + P2 #1). The fold-back applies the lesson reflexively: every fix was verified by re-grepping the affected files for the new defect-pattern keywords (dangling article, halo-entry sink, broken file-path prefix) returning zero, and by greppability-verifying each new symbolic anchor against nucleus/.

Honest disclosure: the original cycle-126 commit message claimed "AC#1-4 satisfied per the charitable interpretation"; the architect P2 #2 finding establishes that the CLASS-LEVEL claim was overstated. The fold-back narrows the cycle-126 claim to "AC#1-4 literal patterns satisfied per the charitable interpretation; class-wide closure scoped out to TASK-0311". The amended commit (fold-back commit) discloses this.

Gate (re-run after fold-back): just check + clippy + test dev/release 854/0/3 + e2e 108/92/0/16/0 unchanged (no code touched).

Forward-carried lesson for TASK-0311: when scrubbing same-class citations, run each substitution as a small atomic edit and re-grep the surrounding context for: dangling articles, broken file-path prefixes, non-greppable descriptive coinages, AC semantic inversions. The cycle-126 sed-batch substitution caused all 4 P1/P2 #1 findings; a smaller-step approach would have caught each defect at its insertion site.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 126 LANDED with NO-GO → fold-back. Cycle commit e2f62cc + cycle fold-back commit (TBD). 4 architect findings folded back in-thread (P1 #1-3 + P2 #1 — substitution-defects this cycle introduced + 1 in-file P2 #2 sibling); cross-file P2 #2 class-wide siblings filed as TASK-0311 (LOW) with the cycle-126 substitution-defect lessons explicitly carried as cycle-311 AC#3.
<!-- SECTION:FINAL_SUMMARY:END -->
