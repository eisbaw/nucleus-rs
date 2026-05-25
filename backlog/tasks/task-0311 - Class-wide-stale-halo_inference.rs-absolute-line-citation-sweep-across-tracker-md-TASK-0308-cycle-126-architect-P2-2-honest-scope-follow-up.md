---
id: TASK-0311
title: >-
  Class-wide stale halo_inference.rs absolute-line citation sweep across tracker
  md (TASK-0308 cycle-126 architect P2 #2 honest-scope follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 05:55'
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
