---
id: TASK-0378
title: >-
  Pre-existing rustfmt drift in nucleus-compiler (lib.rs, transfer_inject.rs,
  sidecar.rs, algo_lower.rs, algo_parser.rs)
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-30 23:35'
updated_date: '2026-05-31 03:11'
labels:
  - fmt
  - hygiene
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered during TASK-0377 (cycle 218): cargo fmt --all -- --check reports formatting drift in several committed files that no recent task touched: nucleus-compiler/src/lib.rs (net_soundness re-export ordering), passes/transfer_inject.rs (~7 sites), src/sidecar.rs (2 sites), tests/algo_lower.rs (2 sites), tests/algo_parser.rs (1 site). These predate TASK-0377 (diff is vs HEAD; the files are unmodified by 0377). The everyday cheap gate (just build+clippy+test+test-release+e2e) does NOT include a fmt-check arm, so the drift went unnoticed; just ci (full gate) likely catches it via just fmt-check. Fix: run cargo fmt --all once on the tree, verify just ci stays green, commit as a formatting-only change. Low risk, mechanical.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 cargo fmt --all -- --check is clean on the whole nucleus workspace
- [ ] #2 just ci passes (fmt arm included)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
SEPARATE formatting-only commit AFTER the TASK-0383 split.
Cited file list (lib.rs, transfer_inject.rs, sidecar.rs, ...) is STALE per cycle-218 notes — re-verify current drift with `cargo fmt --all -- --check` and fix whatever is actually drifted (orchestrator brief reports the real drift is now in backend-common/tests/render_gather_negative.rs + render_guard_siblings.rs). Run `cargo fmt --all` (also formats the new split files). Tick AC#1 (fmt --check clean) truthfully; AC#2 is mis-stated (just ci has NO fmt arm) — record the correction, do not game it. Note overlap with TASK-0276.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Likely duplicate/overlap of the older TASK-0276 (Apply accumulated rustfmt drift, TASK-0256 follow-up). Both are the same recurring deferred fmt-cleanup condition. When picked up, dedupe against TASK-0276 — fix once, close both.

Cycle-218 orchestrator verification: the rustfmt DRIFT is CONFIRMED real — `just fmt-check` (= cd nucleus && cargo fmt --all -- --check) exits 1 with diffs in nucleus-compiler/src/lib.rs:63,70 and passes/transfer_inject.rs (2961,497,5180,5230,5391,5516,5576, ...). HOWEVER the filing claim that "just cis fmt arm catches it" is INCORRECT: `just ci` has NO fmt arm (its body runs check/clippy/test/test-release/check-*/e2e/determinism/xbackend/required-coverage — no fmt-check). Per justfile line ~44 fmt is DELIBERATELY dev-side informational only (TASK-0069 closure: clippy is the gate, not fmt). So this drift does NOT block `just ci`; it is only surfaced by the standalone `just fmt-check`. Net: real, pre-existing (files untouched by TASK-0377), non-gating, overlaps TASK-0276. Trivial fix is `just fmt` but it rewrites 5 files this cycle did not author -> keep as its own reviewed change. (Recurrence of the implementer-disclosure-mechanism-wrong pattern: correct symptom, wrong attributed gate.)
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle-220 (TASK-0383 sibling) — APPLIED. Ran `cargo fmt --all` in the nix dev shell; `cargo fmt --all -- --check` now exits 0 (AC#1 ticked).

CORRECTION on the brief's framing: the claim that the originally-cited file list (lib.rs / transfer_inject.rs / sidecar.rs / algo_lower.rs / algo_parser.rs) was STALE is itself only PARTLY right. Actual current drift at fix time spanned 7 files: nucleus-compiler/src/lib.rs (net_soundness re-export ordering, 2 sites), passes/transfer_inject.rs (7 sites), tests/algo_lower.rs (2), tests/algo_parser.rs (1) — i.e. 4 of the 5 originally-cited files were STILL drifted — PLUS two newer files the original filing predates: backend-common/tests/render_gather_negative.rs (1) and render_guard_siblings.rs (3). The ONE originally-cited file no longer drifted was src/sidecar.rs (its 2 sites vanished when TASK-0383 split it; the split's new child sidecar/cumulative_tests.rs picked up 1 fmt site instead, also fixed here). So: original list ~80% accurate, not stale; the 'real drift is only the two backend-common files' framing in the brief understated it.

AC#2 ('just ci passes (fmt arm included)') is MIS-STATED and was NOT ticked: `just ci` has NO fmt arm (verified — its body runs check/clippy/test/test-release/check-*/e2e/determinism/xbackend/required-coverage). Per justfile line ~44 fmt is deliberately dev-side informational only (TASK-0069 closure: clippy is the gate, not fmt). The drift was therefore NON-GATING for `just ci`; it is only surfaced by the standalone `just fmt-check`. Recording the correction rather than gaming the AC.

Committed as a formatting-only change separate from the TASK-0383 split so the structural-move diff stays clean. Overlaps the older TASK-0276 (same recurring deferred fmt-cleanup condition) — this fix discharges that condition for the current tree too; dedupe/close TASK-0276 against this.
<!-- SECTION:FINAL_SUMMARY:END -->
