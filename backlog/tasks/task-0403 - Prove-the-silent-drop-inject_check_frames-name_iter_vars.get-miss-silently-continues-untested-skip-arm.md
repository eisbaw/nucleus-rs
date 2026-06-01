---
id: TASK-0403
title: >-
  Prove-the-silent-drop: inject_check_frames name_iter_vars.get miss silently
  continues (untested skip arm)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 05:53'
updated_date: '2026-06-01 12:24'
labels:
  - hardening
  - testing
  - prove-the-silent-drop
  - silent-sibling
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 TASK-0402 architect-review P3c follow-out. Distinct CATEGORY from the UnknownLoopVar typed-error family TASK-0402/0400 completed: inject_check_frames.rs:~97 handles a name_iter_vars.get(name) MISS by silently continue-ing (a check directive whose name resolves to an algorithm loop but produced no IterVar -- e.g. a loop the compiler eliminated -- is skipped, the assertion having no loop to bind to). The link step is the documented gate that rejects genuinely-unknown names, so this skip is BELIEVED correct, but the skip arm has NO test (neither a positive that a real eliminated-loop check is dropped, nor a pin that the drop is intentional-not-a-defect).

SCOPE: add a prove-the-silent-drop test -- construct a checks map with a directive whose name is absent from name_iter_vars (eliminated/non-resolving loop) and assert inject_check_frames produces NO check frame for it (and does not panic / does not misbind). Mirror the white-box (LinkedIR, ACFG) poison style of TASK-0402 if a real eliminated-loop fixture is not reachable from .nuc source.

This is a SILENT-DROP guard (returns/continues), NOT a typed-error variant, so it is correctly outside the prove-the-check-bites error-enum audit. Lower value than a typed guard (a wrong silent drop loses a check assertion quietly) -- but exactly the silent-sibling class the project tracks. LOW; purely additive coverage.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0410/0411 (cycle-237): the just-ci gate does NOT build docs, so any change touching a doc-linked symbol (removing/narrowing a pub item, removing an error variant referenced by [`...`]) must run cargo doc --workspace --no-deps before/after and diff the generated-N-warning sum (baseline 10). For bite/sibling-sweep tasks that ADD tests this is usually moot, but if the work removes or renames a symbol carrying an intra-doc-link, add the cargo-doc diff to the gate.

ORCHESTRATOR-IMPLEMENTED IN-THREAD (cycle-241; single test, per do-not-spawn-subagent-for-trivial-task). HONEST FINDING: the CORE silent-drop arm (name_iter_vars.get MISS at inject_check_frames.rs ~line 97) was ALREADY covered by the pre-existing inline test unknown_check_name_silently_dropped (lone unknown-name check -> no frame, no panic) -- TASK-0403 was filed cycle-236 without noticing this. So this cycle added ONLY the genuinely-uncovered increment, NOT a duplicate: eliminated_name_check_dropped_per_entry_valid_sibling_still_injects -- a MIXED checks map {ghost (absent from names), n (valid)} with BTreeMap ordering visiting ghost FIRST, proving the arm is a PER-ENTRY continue (not loop-break) and a valid sibling still injects (10ms, loop_var n, no misbind). BITE verified empirically by continue->break mutation (test FAILS, then reverted) -- existing single-check tests do NOT bite this. REVIEW GATE: qa GO (build clean, clippy fresh exit 0, test 1238 dev / 1237 release -- new test passes BOTH profiles, e2e 385/328/0/57/0 x2; one P3 process note: a transient touch-then-test incremental-build race, not reproducible) + architect GO (independently re-ran the mutation confirming bite; confirmed already-covered judgment correct + new test genuinely distinct not perfunctory). ARCHITECT P3 (FOLDED BACK @f1a9160): traced that the arm is STRUCTURALLY UNREACHABLE for a link-accepted check (name_iter_vars = collect_iter_var_names(all algo loops); link gate rejects any check var not in that set via UnknownLoop; no elimination pass between) -- so the eliminated-loop framing in BOTH the pre-existing production comment AND the new test comment was a speculative doc-lie. Corrected both to defense-in-depth framing. prove-the-check-bites silent-drop residual now CLOSED.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0403 DONE (cycle-241, orchestrator-implemented in-thread). Added eliminated_name_check_dropped_per_entry_valid_sibling_still_injects to inject_check_frames mod tests, pinning that the name_iter_vars.get MISS arm is a per-entry continue (mixed eliminated+valid checks map, BTreeMap-ordered so the dropped entry iterates first; bite-verified by continue->break mutation). Core drop arm was already covered by unknown_check_name_silently_dropped; this adds the per-entry-continue composition increment only (no duplicate). qa GO + architect GO. Fold-back f1a9160 corrected the eliminated-loop doc-lie (arm is structurally unreachable defense-in-depth, link-gated) in both production + test comments. Commits 248c84c + f1a9160. e2e 385/328/0/57/0; test 1238/1237.
<!-- SECTION:FINAL_SUMMARY:END -->
