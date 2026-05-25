---
id: TASK-0311
title: >-
  Class-wide stale halo_inference.rs absolute-line citation sweep across tracker
  md (TASK-0308 cycle-126 architect P2 #2 honest-scope follow-up)
status: Done
assignee:
  - mark
created_date: '2026-05-25 05:55'
updated_date: '2026-05-25 06:26'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0308 cycle-126 narrowly scrubbed 4 ENUMERATED patterns from backlog/tasks/:
- phantom function-name citation
- `halo_inference.rs:53-57` (contract paragraph absolute-line)
- `halo_inference.rs:848` (halo-entry sink absolute-line)
- `halo_inference.rs:1199` (no_halo_bare_iv test absolute-line)

The cycle-126 architect P2 #2 review-gate finding flagged that the underlying defect CLASS is broader — "stale absolute-line citations into halo_inference.rs anywhere in backlog/tasks/" — and that scoping AC#1-4 to four literal patterns left structurally-identical sibling citations untouched. Filing as the honest follow-up so cycle-126's claim to close the defect-class is not over-reach.

## Surveyed same-class sibling sites (cycle 126 grep)

- `backlog/tasks/task-0260 - ...md:132` — `halo_inference.rs:682` LIVE description.
- `backlog/tasks/task-0263 - ...md:85` — `halo_inference.rs:361-367` + `:412` (plan/notes; cycle-89 verification record — borderline historical, may stay).
- `backlog/tasks/task-0275 - ...md:27` — `halo_inference.rs:361` (description).
- `backlog/tasks/task-0309 - ...md:48` — `halo_inference.rs:89-129` (cross-references; task is To Do, citation is LIVE).

In-file siblings cycle-126 fixed during the fold-back (so this task does NOT need to redo them): task-0305 lines 34, 73, 94 (`halo_inference.rs:1184` description + plan + the broken SHIPPED file-path header).

## Acceptance criteria

1. `grep -rn 'halo_inference\.rs:[0-9]' backlog/tasks/` returns hits ONLY inside intentional historical-lesson-preservation records (same charitable-interpretation rule as cycle 126).
2. Each migrated citation uses the cycle-122 symbolic-anchor convention.
3. The cycle-126 lessons (P1 + P2 #1) MUST be applied during the substitution:
   - Single-string substitution per file (avoid multiple sites per `sed`).
   - Test-read the surrounding context to ensure the substitution does NOT (a) dangle a directory prefix off a noun phrase (P1 #1), (b) leave duplicated articles (P1 #3), (c) invert semantics on a LIVE AC (P1 #2), or (d) introduce a NON-GREPPABLE descriptive coinage (P2 #1).
   - Use the production-greppable anchors: `per_iv.entry(iv).or_insert(0)`, `fn no_halo_bare_iv`, `absent ≡ explicit-0`, `apply_halo_inference`, etc. — verified by `grep -rn '<anchor>' nucleus/` returning ≥1 hit.
4. Each substitution is run through the cycle-125 heredoc-quoting discipline (`cat <<'EOF'` not `cat <<"EOF"`) to avoid backtick command substitution.

## Honest scope

LOW priority. Same defect class as TASK-0308 cycle-126; same charitable AC interpretation applies. The class-wide closure was OUT OF SCOPE for cycle 126 because cycle-126 narrowly tracked the 4 patterns the cycle-123 architect enumerated, and widening scope mid-cycle to a class-wide sweep would have introduced its own substitution defects (as evidenced by the cycle-126 NO-GO that filed this).

## Cross-references

- TASK-0308 cycle-126 architect P2 #2 review-gate finding.
- TASK-0308 cycle-126 architect P1 #1, P1 #2, P1 #3, P2 #1 — the substitution-defect lessons to apply.
- Memory: `feedback-silent-sibling-defect` — the recurrence pattern at the meta level (closing visible sites while same-class siblings silently skip).
- Memory: `feedback-comment-doc-lie-recurring` — the underlying defect class.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0308 cycle-126 fold-back: durable memory entries persisted at feedback-sed-batch-tracker-md-substitution.md (the 4-defect class introduced by sed-batch substitution) and feedback-ac-rewrite-on-done-task.md (the related caveat on rewriting LIVE ACs of Done tasks). The next implementer briefing on TASK-0311 should read both before proposing the substitution shape. The cycle-126 lessons in concrete form:

1. Atomic per-string substitution + surrounding-context re-grep AFTER each. Never sed-batch.
2. Every NEW symbolic anchor must be greppability-verified by grep -rn "<anchor>" nucleus/ returning ≥1 hit. Editorial descriptive coinages (e.g. "halo-entry sink") are forbidden; use production literals (per_iv.entry(iv).or_insert(0), fn no_halo_bare_iv, "absent ≡ explicit-0" with backtick delimiters matching production).
3. For LIVE ACs on Done tasks, prefer separate "Cycle-NNN clarification addendum" block. In-place rewrite only when the AC was ambiguous at filing AND the de-facto interpretation is well-documented. Always cite the earlier cycle whose interpretation is being recorded.
4. Heredoc quoting discipline (cycle-125 lesson): use cat <<'EOF' (single-quoted) not double-quoted; backticks otherwise get command-substituted to empty strings.

Sites TASK-0311 specifically targets (as of cycle 126):
- task-0260:132 — halo_inference.rs:682 (F-P1 finding record in description)
- task-0263:85 — halo_inference.rs:361-367 + :412 (cycle-89 verification record; borderline historical, may stay)
- task-0275:27 — halo_inference.rs:361 (description)
- task-0309:48 — halo_inference.rs:89-129 (cross-references on a To Do task — LIVE citation)

## Cycle-127 implementation plan (orchestrator-led, 2026-05-25)

Orchestrator-led rather than implementer-briefed because the work is 4 atomic markdown edits across 4 tracker md files with explicit cycle-126 lessons-to-apply — sending it to a fresh subagent risks re-introducing the very sed-batch / dangling-article defects the lessons forbid. The mandatory parallel review gate (qa-test-runner + mped-architect read-only) still runs.

### Surveyed sites (cycle 127 grep, 2026-05-25)

`grep -rn 'halo_inference\.rs:[0-9]' backlog/tasks/` after the cycle-126 fold-back returns 4 LIVE citations + 13 historical-record hits (the latter qualify for charitable retention per the cycle-126 rule):

LIVE migration targets (these 4):
- task-0260:132 (Implementation Notes) — `halo_inference.rs:682` + STALE variant name `HaloInferenceError::UnknownIterVarInScope` (renamed to `UnknownLoopVar` in cycle 95, commit f8a3267, but task-0260's record was missed by that cycle's sweep at 1c5c221)
- task-0263:85 (Implementation Notes / cycle-89 verification) — `halo_inference.rs:361-367` + `(line 412)`
- task-0275:27 (Description) — `halo_inference.rs:361`
- task-0309:48 (Cross-references; To Do task; LIVE) — `halo_inference.rs:89-129`

Historical-lesson-preservation hits (retain per cycle-126 charitable rule):
- task-0307 (5 hits) — recorded past-cycle implementation notes
- task-0308 (1 hit) — recorded substitution-defect string
- task-0311 (7 hits) — this task's OWN description enumerating the sites to fix

### Greppability-verified symbolic anchors (production literals)

- `fn apply_halo_inference` — halo_inference.rs:411 (strict-A entry point)
- `fn apply_halo_inference_advisory` — halo_inference.rs:436
- `fn apply_halo_inference_partition_aware` — halo_inference.rs:471 (driver path; partition-policy-aware-B)
- `fn infer_halo_widths` — halo_inference.rs:529 (strict-variant short-circuit driver)
- `HaloInferenceError::UnknownLoopVar` — the typed-error variant filed cycle 81 (originally `UnknownIterVarInScope`, renamed cycle 95)
- `## Strict vs advisory vs partition-policy-aware entry points` — halo_inference.rs:100, module-doc section heading; single grep hit; durable anchor for the 3-entry-point contract paragraph

### Substitution discipline (cycle-126 lessons enforced)

1. Per-string atomic Edit (not sed-batch).
2. After each Edit: re-grep `halo_inference.rs:[0-9]` in the touched file to verify the substitution landed AND did not propagate.
3. After all 4: full repo-wide `grep -rn 'halo_inference\.rs:[0-9]' backlog/tasks/` — expect the 13 historical hits only.
4. Surrounding-context re-read of each substitution to defend against P1 #1 (dangling article), P1 #3 (duplicated article); preserve the short-vs-full-path convention of each original.
5. P1 #2 (AC semantic inversion) does not apply — none of the 4 sites are inside ACs.

### Out of scope (deliberate)

- task-0263:85 was flagged in TASK-0311 description as "borderline historical, may stay" — proceeding to migrate it anyway because the cycle-89 verification record sits in the LIVE Implementation Notes of an In-Progress task; an active engineer might follow the citation today, so the symbolic anchor is strictly better than the line number.
- The 13 historical-record hits are NOT migrated; charitable rule (cycle-126).

## Final summary (cycle 127, 2026-05-25)

TASK-0311 LANDED. Class-wide stale `halo_inference.rs:[0-9]` absolute-line citation sweep across `backlog/tasks/` completed via 4 atomic per-string Edits across 4 tracker md files (task-0260, task-0263, task-0275, task-0309) + 1 paired secondary Edit on task-0260 (CYCLE-95 UPDATE block revision) to resolve a duplicated rename-disclosure caught via surrounding-context re-read.

### Per-AC verdict

- **AC#1 — class-wide closure**: GREEN. Post-migration grep shows zero hits in the 4 LIVE target sites. Remaining hits are confined to {task-0305 (1), task-0307 (10), task-0308 (2), task-0311 (21 self-references)} — all qualify under the cycle-126 charitable historical-record retention rule. Confirmed by both qa-test-runner and mped-architect.
- **AC#2 — cycle-122 symbolic-anchor convention**: GREEN. All 4 migrated citations use `(search for X)` or `(module-doc section "..."; search for that heading)` form with greppable production literals. Anchors used: `fn apply_halo_inference`, `fn apply_halo_inference_advisory`, `fn apply_halo_inference_partition_aware`, `fn infer_halo_widths`, `HaloInferenceError::UnknownLoopVar`, `## Strict vs advisory vs partition-policy-aware entry points`. Each verified ≥1 hit in `nucleus/`.
- **AC#3 — cycle-126 substitution-defect lessons applied**: GREEN. No dangling article, no duplicated article, no AC inversion (none of the 4 sites are inside ACs), no non-greppable coinage. One reflexive cycle-126 P1 #3 defect was caught DURING the cycle (task-0260 line 132 vs the pre-existing CYCLE-95 UPDATE at line 152 carrying redundant rename-disclosure) and resolved by paired secondary Edit.
- **AC#4 — cycle-125 heredoc-quoting discipline**: GREEN (vacuously). No actual shell heredoc was committed this cycle; the only `cat <<"EOF"` string in the diff is inside this task's description as a LESSON-DESCRIPTION of the discipline, not a heredoc usage.

### Review gate

Parallel read-only review:
- qa-test-runner: GO. `just test` 854/0/3; `just test-release` 854/0/3 (dev/release parity preserved); `just e2e` 108/92/0/16/0 (M5 baseline preserved as expected for tracker-md-only change).
- mped-architect: GO with one P2 sibling-defect follow-up (the same defect class fires on other files cited by tracker md — partition_workers.rs:40 ×10, driver/main.rs:410+413 ×7, ~365 distinct `.rs:NNN` citations corpus-wide). Filed as TASK-0312.

### Gotchas + lessons forward-carried

1. **Reflexive cycle-126 P1 #3 firing on its own fix site**. The cycle-126 lesson explicitly warns about substitution-induced defects; when the cycle's purpose is scrubbing stale citations, the substitution-induced defect (duplicated rename-disclosure between line 132 and line 152 of task-0260) IS itself the comment-doc-lie class the cycle is supposed to be scrubbing. The discipline (surrounding-context re-read after each Edit) caught it mid-cycle. Folded into `feedback-sed-batch-tracker-md-substitution` cycle-127 epilogue.

2. **The CYCLE-95 UPDATE block in task-0260 was an in-place mutation** of a historical record, rather than an appended cycle-127 ADDENDUM. The cycle-127 qa-test-runner flagged this as P3-informational (mention-only) per the `feedback-ac-rewrite-on-done-task` neighbour rule. The block is process-meta, not an AC, and it cross-references both states accurately; not a literal violation. If a third instance of in-place-revising a historical UPDATE block recurs, promote to a hard P2.

3. **Orchestrator-led for tracker-md hygiene was the correct call**. 4 atomic markdown edits across 4 files with explicit lessons-to-apply is a shape where briefing a fresh implementer subagent carries higher risk (mechanical generalisation re-introducing the very defects the lessons forbid) than the orchestrator-context risk. Mandatory parallel review gate still ran and confirmed.

4. **Forward-carried to TASK-0312**: the architect P2 finding — the same defect class fires on OTHER production files cited by tracker md, ~365 distinct citations corpus-wide. The recurring class is `.rs:NNN` line citations in tracker md, file-agnostic; TASK-0311 was just the halo_inference.rs slice. TASK-0312 carries the broader scope at LOW priority.

5. **Forward-carried to TASK-0312 specifically (high-frequency offenders to start with)**: partition_workers.rs:40 ×10 (across task-0249, task-0258, task-0259); driver/main.rs:410 ×4 + :413 ×3 (across task-0265, task-0271, task-0280; task-0274:69 already self-corrected). These are the empirically-verified-stale starting set.

### Files changed (tracker-md hygiene only; production code untouched)

- `backlog/tasks/task-0260 - ... .md`: lines 132 + 152 (F-P1 finding-record migration + paired CYCLE-95 UPDATE block revision)
- `backlog/tasks/task-0263 - ... .md`: line 85 (cycle-89 code-path-verified citation migration)
- `backlog/tasks/task-0275 - ... .md`: line 27 (description citation migration)
- `backlog/tasks/task-0309 - ... .md`: line 48 (cross-references citation migration)
- `backlog/tasks/task-0311 - ... .md`: status In Progress → Done + plan/final-summary appends
- `backlog/tasks/task-0312 - ... .md`: NEW (follow-up filed)
- `~/.claude/projects/-home-mpedersen-topics-mark-thesis/memory/feedback-sed-batch-tracker-md-substitution.md`: cycle-127 epilogue
- `~/.claude/projects/-home-mpedersen-topics-mark-thesis/memory/feedback-silent-sibling-defect.md`: cycle-127 entry on meta-level silent-sibling shape
<!-- SECTION:NOTES:END -->
