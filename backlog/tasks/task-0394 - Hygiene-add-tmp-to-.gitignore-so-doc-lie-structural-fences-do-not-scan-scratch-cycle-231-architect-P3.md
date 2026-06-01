---
id: TASK-0394
title: >-
  Hygiene: add tmp/ to .gitignore so doc-lie/structural fences do not scan
  scratch (cycle-231 architect P3)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 00:53'
updated_date: '2026-06-01 01:10'
labels:
  - tooling
  - ci
  - hygiene
  - gitignore
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-231 architect-review P3 (a89db02). tmp/ is untracked but NOT gitignored, so ripgrep scans it -- meaning ALL the check-* fences that scan '.' with only -g '!target/**' (check-doc-citation-staleness / -bare / check-doc-test-name-staleness / check-narrative-doc-lie etc.) also scan tmp/. A scratch .rs dropped in tmp/ can RED 'just ci' for the developer (architect empirically reproduced: tmp/fence_test/inject.rs broke the new fence; qa reproduced the bite via tmp/qa_bite.rs). Pre-existing shared footgun across the whole fence family, NOT introduced by any one fence. Fix: add 'tmp/' to .gitignore (the project's scratch dir per CLAUDE.md cruft/scratch conventions). Verify no fence or recipe relies on tmp/ being scannable first. Low risk, removes the footgun for the whole family at once.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-231 orchestrator note — TWO implementation options, pick in the cycle:
(A) Add tmp/ to .gitignore. One line, fixes the whole fence family at once. DOWNSIDE: also hides tmp/ from git status (user-facing behavior change) — confirm the user is fine not seeing scratch in status.
(B) Add -g (!tmp/** ) to each fence recipe that scans . (check-doc-citation-staleness / -bare / check-doc-test-name-staleness / check-narrative-doc-lie scans e2e-matrix.toml only so N/A). Keeps git status behavior unchanged; scopes the fix to the fences. DOWNSIDE: touches several recipe bodies; a NEW fence added later could forget the exclusion (silent-sibling risk) — mitigate by a shared variable or a lead comment.
RECOMMENDATION: (B) is lower-blast-radius (no git-status behavior change) but (A) is more durable + DRY. Lean (A) if the user does not rely on seeing tmp/ in status; else (B). Either way verify all check-* fences still green after.

Cycle-232 (orchestrator in-thread; trivial .gitignore change). DECISION: option A (add tmp/ to .gitignore), resolved on evidence:
- Root .gitignore ALREADY ignores local scratch + per-user state (target/, backlog/email-preferences.json, .claude/scheduled_tasks.lock) with explanatory comments -- ignoring tmp/ is fully consistent with that convention.
- NO recipe references the repo ./tmp/: the only tmp/ mention in justfile is the SYSTEM /tmp/ absolute path in port-stress-check. So nothing relies on ./tmp/ being scannable or tracked.
- Fixes the WHOLE fence family at once (DRY); rg respects .gitignore so all check-* fences that scan . stop seeing tmp/ scratch (~241 generated .rs files today).
GOTCHA introduced (forward): after this, rg no longer scans tmp/, so a bite-test for any of these fences CANNOT use a tmp/ path (it will silently pass). Bite-tests must inject into a scannable (tracked-crate) path then rm. Re-proving the fence still bites via a non-tmp path is part of this cycle.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-232. Added tmp/ to .gitignore (option A) so the three repo-ROOT rg-scanning doc fences (check-doc-citation-staleness, -bare, check-doc-test-name-staleness) skip local scratch. Closes the architect cycle-231 P3 footgun: un-ignored scratch .rs in tmp/ (241 generated files today) would be scanned and could RED just ci for whoever has a stale file there.

REVIEWED GOx2 (qa-test-runner + mped-architect, read-only, parallel). qa: tmp/ ignore proven via stash A/B (the new line is the sole cause); all 6 tree-scanning fences green; fence STILL BITES via a scannable path (nucleus/backend-common/zz_qa_bite.rs -> FAIL exit 1); a stale ref placed in tmp/ is now SKIPPED (the gotcha, confirmed). architect: option A is the durable + DRY + silent-sibling-proof choice over B (B would touch 3 sibling recipes + invite a forgot-the-glob silent-sibling); no recipe/CI/test depends on ./tmp/ (.github/workflows/ci.yml runs on fresh checkout, never reads ./tmp/).

FOOTGUN IS LATENT, NOT ACTIVE (architect P3 #2, verified): forcing rg over tmp/ returns zero hits today (generated emit has no doc-citation prose / no task-named test fns). The fix is PREVENTIVE.

REVIEW-DRIVEN FIX applied in-thread (architect P3 #1 -- a doc-lie in my OWN .gitignore comment, the recurrence this fence family exists to catch): the comment had falsely listed check-mega-files as gitignore-affected; it scans explicit nucleus/.../src roots and never saw tmp/. Narrowed the comment to the three actual repo-root fences.

FORWARD GOTCHA (recorded in the .gitignore comment + TASK-0395): rg no longer scans tmp/, so a fence bite-test must inject into a tracked-crate path, not tmp/.

e2e NOT re-run this cycle (cargo ignores .gitignore, so build/test/e2e are byte-identical to the 385/328/0/57/0 baseline established twice earlier this session; qa-test-runner independently agreed e2e is unaffected). FOLLOW-UP filed: TASK-0395 (make the 3 root-scanning fences robust to arbitrary non-tmp scratch dirs).
<!-- SECTION:FINAL_SUMMARY:END -->
