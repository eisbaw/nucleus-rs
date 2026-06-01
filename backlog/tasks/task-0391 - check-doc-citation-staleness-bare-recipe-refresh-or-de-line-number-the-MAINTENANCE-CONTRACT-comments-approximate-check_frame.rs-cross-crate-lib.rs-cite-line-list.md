---
id: TASK-0391
title: >-
  check-doc-citation-staleness-bare recipe: refresh (or de-line-number) the
  MAINTENANCE-CONTRACT comment's approximate check_frame.rs cross-crate lib.rs
  cite line list
status: Done
assignee:
  - '@mark'
created_date: '2026-05-31 22:16'
updated_date: '2026-06-01 00:12'
labels:
  - tech-debt
  - doc-drift
  - justfile
  - citation-fence
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3.2 on TASK-0382.01 (1fae634). The check-doc-citation-staleness-bare recipe MAINTENANCE-CONTRACT comment block enumerates approximate line numbers (cited as ~133/148/149/179/194/200/201) of the cross-crate bare lib.rs:N citations in nucleus/backend-common/src/check_frame.rs that the WIN=3 prose guard must keep skipping. Those line numbers have DRIFTED from the actual check_frame.rs lines (architect measured ~146/162/193/208). PRE-EXISTING (not introduced by TASK-0382.01), hedged with ~, and it lives in a justfile comment that NO fence scans, so it is a soft doc-drift not a gate failure. Fix: either refresh the line list against current check_frame.rs, OR (more durable) de-line-number it — describe the cites by their stable provenance (pre-extraction historical lib.rs pointers naming pthreads-sync/mp-tcp-bufsync within WIN lines) without specific line numbers, since an approximate line list in a comment itself rots. LOW.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-231 (orchestrator in-thread; trivial comment-only justfile fix, no subagent spawn per phase3 "do not spawn for trivial tasks").

GROUND-TRUTH (verified): the MAINTENANCE-CONTRACT comment hardcoded line list `~133/148/149/179/194/200/201` is both STALE and INCOMPLETE. Actual cross-crate bare lib.rs:N cites in nucleus/backend-common/src/check_frame.rs live at 146, 162-163, 193, 208, 214-215 (grep -nE verified). All point at pthreads-sync / mp-tcp-bufsync pre-extraction lib.rs sites (TASK-0052.04 provenance), never backend-common own 92-line lib.rs.

FIX (architect-recommended durable option): de-line-number the comment entirely. Replaced the rotting enumerated list with a stable provenance description + a grep recipe to locate them, plus an explicit note that an approximate line list in a comment is itself the line-rot this fence exists to catch (self-consistent with the recipe FAIL message).

SILENT-SIBLING SWEEP (feedback-silent-sibling-defect): swept whole justfile for other rotting line-number LISTS in comments. None found — the line-612 list was unique. Other :N matches (465/468/549-551/588/592) are illustrative citation-FORMAT examples whose .rs cite strings are self-validated by the fence on the .rs side; line 485 ~42 is a hedged count not a location list.

VERIFIED: just --list parses OK; just check-doc-citation-staleness-bare still OK (edit was entirely in the comment block above the recipe).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-231 (commit f800f8c). De-line-numbered the check-doc-citation-staleness-bare MAINTENANCE-CONTRACT comment per the preferred durable option (architect P3.2 on TASK-0382.01).

WHAT LANDED: replaced the stale+incomplete hardcoded cite line-list (~133/148/149/179/194/200/201; ZERO correct entries) with a stable provenance description (TASK-0052.04 pre-extraction pthreads-sync / mp-tcp-bufsync lib.rs pointers, NOT backend-common own 92-line lib.rs) + a grep locator (grep -nE lib.rs:[0-9] over check_frame.rs). Recipe BODY byte-unchanged; the load-bearing WIN-line maintainer guidance preserved; P3.1 prose-flow re-wrap polished in-thread.

VERIFIED (independently, both reviewers): actual cross-crate cites live at 146, 162-163, 193, 208, 214-215; backend-common src/lib.rs = 92 lines (the load-bearing FP-shortness). Reviewed GOx2 (qa-test-runner full gate + mped-architect, both re-verified ground-truth from scratch). Gate: build clean, clippy 0/0, test 1206p/0f/3i, test-release 1205p/0f/3i (TASK-0291 1-test divergence expected), e2e 385/328/0/57/0 twice (non-flaky), recipe still prints OK.

SILENT-SIBLING SWEEP (feedback-silent-sibling-defect): swept the whole justfile for other rotting line-number LISTS in comments. None found -- the line-611/612 list was the unique harmful instance. Other :N matches (465/468/549-551/588/592/1306) are illustrative citation-FORMAT examples self-validated by the fence on the .rs side, an example-NUMBER list, or a hedged ~42 count (485) -- none assert source-line locations.

GOTCHA / LESSON (forward): a comment that itself enumerates source line numbers is the SAME line-rot the citation fences exist to catch; prefer stable provenance + a grep locator over an enumerated list. P3.2 (architect advisory, NOT actioned, no follow-up filed): the grep locator is intentionally broader than cross-crate-only -- today every lib.rs:N in check_frame.rs happens to be cross-crate, but a future same-crate lib.rs:N cite would over-surface; acceptable for a locator aid (not a guard).

No residual gaps. The sibling gap (bare-basename-as-location prose symbol/test residence checking) remains TASK-0382.02.
<!-- SECTION:FINAL_SUMMARY:END -->
