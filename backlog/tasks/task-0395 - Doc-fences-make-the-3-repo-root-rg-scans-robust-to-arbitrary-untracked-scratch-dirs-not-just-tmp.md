---
id: TASK-0395
title: >-
  Doc fences: make the 3 repo-root rg scans robust to arbitrary untracked
  scratch dirs (not just tmp/)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 01:10'
updated_date: '2026-06-01 21:07'
labels:
  - tooling
  - ci
  - doc-lie
  - robustness
dependencies:
  - TASK-0394
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-232 architect P3 #4 (review adbf154) follow-up to TASK-0394. TASK-0394 added tmp/ to .gitignore so the three repo-root rg-scanning fences (check-doc-citation-staleness, check-doc-citation-staleness-bare, check-doc-test-name-staleness) skip the conventional scratch dir. But a DIFFERENTLY-named untracked scratch dir (scratch/, foo-out/, an ad-hoc emit dir) would reintroduce the footgun: a stale citation in untracked scratch reds just ci. Robust fix options: (1) scan only git-tracked paths (e.g. feed the fences git ls-files output instead of a bare . root), or (2) restrict each fence to the known crate roots (nucleus/, driver/, etc.) like check-mega-files already does. TRADE-OFF to weigh: option (1) would let a stale citation in an untracked-but-intended-to-be-committed file ESCAPE the fence until git add -- so tracked-only scanning trades the scratch-FP-risk for a pre-commit-coverage-gap. LOW priority; the tmp/ ignore (TASK-0394) covers the conventional case. Only pursue if a non-tmp scratch dir actually trips a fence in practice.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE (commits 97dc3a5 + 89025a4 fold-back). Converted all 5 rg `.` scans across the 4 doc-citation fences (check-doc-citation-staleness / -bare / check-doc-test-name-staleness / check-doc-cell-path-staleness) to `git ls-files -z -- <pathspec> | xargs -0 -r rg ...` so they scan only intentional tracked content — deterministic regardless of untracked, non-gitignored scratch dirs (the footgun TASK-0394 only half-closed for tmp/). Chose option 1 (git ls-files) over option 2 (explicit crate roots): preserves full coverage of tracked .md/.toml/.nuc docs that option 2 would have to re-enumerate. NOTE: the task said "3" scans; there were actually 5 (the cycle-233 cell-path fence post-dates the filing, and check-doc-test-name has 2 scans) — all hardened for silent-sibling completeness.

VERIFIED (3-arm bite test, orchestrator in-thread): (1) all 4 fences pass clean; (2) an untracked non-gitignored scratch dir with stale citations of all 4 types is IGNORED (raw `rg .` control confirmed bait present, so the OLD fences would have tripped — robustness goal met); (3) all 4 still BITE on a tracked stale citation (git add -N also suffices to bring a file in scope).

REVIEW: mped-architect read-only GO. Silent-sibling sweep CLEAN (no other repo-root rg/grep/find scan in the justfile has the footgun; check-mega-files/textual-replace/include-str-coverage/narrative-doc-lie/doc-links all use explicit roots or controlled generated dirs). Architect P2 (fail-OPEN if git ls-files yields nothing) FOLDED BACK in 89025a4: a `git rev-parse --is-inside-work-tree` guard on all 4 fences fails loud instead of scanning nothing. Architect P3 (check-include-str-coverage bashism vs POSIX claim) filed as a separate follow-up.

GOTCHAS (forward-carried): (a) --with-filename / --no-filename are LOAD-BEARING with xargs — a single-file final batch drops the `file:` prefix the parsers split on; set them explicitly. (b) The bare fence is prose-aware and SKIPS a citation whose 3-line preceding window names another crate — so a combined multi-bait bite test mis-reads as no-bite; test the bare fence in ISOLATION. (c) TRADE-OFF accepted: a brand-new fully-untracked file s citations are unchecked until git add (fine for a pre-commit fence; git add -N suffices).
<!-- SECTION:NOTES:END -->
