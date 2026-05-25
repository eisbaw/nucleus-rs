---
id: TASK-0307
title: >-
  halo_inference: structural Some(0) key-exists pin on no_halo_bare_iv emit site
  (TASK-0305 cycle-122 architect P2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 04:05'
updated_date: '2026-05-25 05:54'
labels:
  - M5
  - compiler
  - test-coverage
  - halo_inference
  - contract-pin
  - forward-carried-from-TASK-0305
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0305 (cycle 122) decided Option B (preserve halo_inference's `absent ≡ explicit-0` contract degree of freedom). The architect's review-gate flagged a real coverage gap: NO existing test asserts `halo_widths[K][iv] == Some(0)` for a bare-iv access. Both `no_halo_bare_iv` (in-module test) and `elementwise_add_records_only_zero_halos` use `.unwrap_or(0)` patterns — vacuous-tolerant under silent-skip. The `stencil_3x3_produces_halo_one_on_both_axes` test uses `Some(1)` (key-exists) but only on non-zero offsets.

This means a future walker regression that silently emits NO entries for a bare-iv access would NOT be caught by any current test. The narrative pins (task0299_*, task0303_*) would pass vacuously.

## Acceptance criteria

1. Add a single one-line structural pin in the in-module tests of `nucleus/nucleus-compiler/src/passes/halo_inference.rs` near `no_halo_bare_iv` (line ~1199):
   ```
   assert_eq!(acfg.halo_widths.get(&k_id).and_then(|m| m.get(&iv_id)).copied(), Some(0));
   ```
2. The pin must fail LOUD if the production `per_iv.entry(iv).or_insert(0)` emit site inside `classify_index` (halo_inference.rs) is silently regressed to omit entries for inspected bare-iv accesses.
3. Update the cross-references in the Option B contract paragraph in halo_inference.rs (search for "absent ≡ explicit-0") and sidecar_halo.rs's task0303_07 comment to point at the new contract-form sentinel test. (Closed cycle 123 by CHARITABLE interpretation: kept the production-sink search hint `per_iv.entry(iv).or_insert(0)` AND added a reference to the new `fn no_halo_bare_iv` sentinel — the cycle-123 architect accepted this reading.)

## Honest scope

LOW priority. The vacuous-pass risk is judged unlikely (today's walker DOES always emit explicit-0). This task is a defensive sentinel — single-line pin, no contract change. Compatible with the Option B decision.

## Cross-references

- TASK-0305 (cycle 122) — the Option B decision this defends.
- the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs (search for "absent ≡ explicit-0") — the contract paragraph (Option B project decision marker).
- `per_iv.entry(iv).or_insert(0)` inside `classify_index` (halo_inference.rs) — the production emit site whose silent-skip the pin would catch.
- `fn no_halo_bare_iv` symbolic anchor in halo_inference.rs — in-module test (the location where the pin should land).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT cycle 123.

DESIGN: add the structural Some(0) pin INSIDE the existing in-module test 'no_halo_bare_iv' at halo_inference.rs:1201 (per TASK-0307 AC#1 wording 'near no_halo_bare_iv'). Keep the existing .unwrap_or(0) assertion as contract documentation; ADD the structural sentinel right next to it. This co-locates the contract-floor and the today-implementation-pin in one test for a future reader.

The TASK-0307 AC#3 wording 'replacing the current record_halo text search hint' is interpreted CHARITABLY: keep the symbolic search hint at the production sink (it is the canonical production-site pointer; removing it loses load-bearing information) AND add a reference to the new sentinel test. The architect can catch the divergence in review if I am misreading the intent.

STEPS:
1. Edit halo_inference.rs:1201 no_halo_bare_iv test — add ONE-LINE structural assert that copied() == Some(0); label it cycle-123 TASK-0307 P2 sentinel; the assert message names the protection (vacuous-pass arm).
2. Edit the "TASK-0305 cycle-122 project decision (Option B)" contract paragraph in halo_inference.rs (search for "absent ≡ explicit-0") — add a sentence pointing to the sentinel test as defence-in-depth for the production sink.
3. Edit sidecar_halo.rs task0303_07 docstring (lines ~778-780) — update the predictive 'cycle-122 architect filed TASK-0307 as a structural sentinel' to the past-tense LANDED form, name the in-module test by symbolic anchor.
4. Run the cheap gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e' — pre-mortem any noisy failures and triage.
5. Parallel review gate (qa-test-runner + mped-architect, READ-ONLY).
6. Apply any review findings; re-gate.
7. Commit with message style 'tests + tracker: TASK-0307 cycle 123 structural Some(0) sentinel at no_halo_bare_iv (TASK-0305 Option B defence)'.

GATE: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e' before commit. E2E baseline 104/88/0/16/0 must hold (pure additive test; no code change → e2e bytes unchanged).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR-DIRECT cycle 123 (2026-05-25). TEST + DOC-ONLY.

SHIPPED:
- nucleus/nucleus-compiler/src/passes/halo_inference.rs:74-83 — added contract-doc paragraph naming the new sentinel test by symbolic anchor; describes the protection (closes Option B vacuous-pass arm without coupling downstream tests to the explicit-0 representation).
- nucleus/nucleus-compiler/src/passes/halo_inference.rs:1245-1277 — added structural sentinel inside in-module 'no_halo_bare_iv' test: 'assert_eq!(acfg.halo_widths.get(&k_id).and_then(|m| m.get(&y_iv)).copied(), Some(0), ...)' with explanatory comment. The pre-existing '.unwrap_or(0)' assertion is PRESERVED as Option B contract-form documentation.
- nucleus/nucleus-compiler/tests/sidecar_halo.rs task0299_06 docstring: added LANDED structural-sentinel cross-reference paragraph (mirroring task0303_07's update); explicit acknowledgement of the vacuous-pass arm + how the sentinel closes it. Updated absolute-line citation 'halo_inference.rs:53-57' to symbolic anchor.
- nucleus/nucleus-compiler/tests/sidecar_halo.rs task0303_05 docstring: updated absolute-line citations 'halo_inference.rs:53-57' to symbolic anchors. SKIPPED the LANDED sentinel cross-reference because task0303_05's '== 1' strict-positive pin is contract-form-independent BY CONSTRUCTION (silent-skip → unwrap → 0 ≠ 1 → fails loud); no vacuous-pass arm so sentinel is moot.
- nucleus/nucleus-compiler/tests/sidecar_halo.rs task0303_07 docstring: updated predictive 'cycle-122 architect filed TASK-0307' language to past-tense LANDED form; updated absolute-line citation to symbolic anchor.

GATE: 'just build' clean. 'just clippy' (-D warnings) clean. 'just test' 850/0/3 ignored across 74 test binaries — stable across 2 runs (qa-test-runner flake check). 'just test-release' 850/0/3 (matches dev). 'just e2e' 108/92/0/16/0 — UNCHANGED from cycle-121/122 baseline (purely additive test + doc-only).

REVIEW GATE (cycle 123 parallel read-only):
- qa-test-runner: GO. Numbers measured + traced sentinel reachability through 'classify_index' (line 762) → 'per_iv.entry(iv).or_insert(0)' (line 861). Single emit site, sentinel covers the entire emit surface.
- mped-architect (round 1): NO-GO. Found 2 P1s + 1 P2:
  * P1 #1 (silent-sibling): task0299_06 + task0303_05 sibling docstrings untouched while task0303_07 got the LANDED reference. This is the EXACT recurrence pattern this task was filed to defend against. Folded back: added LANDED-sentinel paragraph to task0299_06; skipped task0303_05 on the strict-positive-by-construction grounds (documented as deliberate skip).
  * P1 #2 (phantom symbol 'record_halo'): the cycle-122 contract paragraph (halo_inference.rs:66) and the TASK-0307 task description (line 48) cited a function 'record_halo' that does NOT exist as a symbol. Real fn is 'classify_index' (halo_inference.rs:762). Cycle 123 propagated the phantom to 2 NEW sites (halo_inference.rs:78 + 1249) + 1 update site (sidecar_halo.rs:775). Folded back via 'replace_all' across both nucleus/ files: 4 sites updated, 'grep -rn record_halo nucleus/' returns zero hits.
  * P2 #3 (stale 'halo_inference.rs:53-57' line citations at 3 sites): updated all 3 to symbolic anchors ('search for "absent ≡ explicit-0"' / paragraph title).
- mped-architect (round 2): GO with one in-commit fix + a follow-up filing:
  * NEW P2 #1 (sentinel-comment overclaim): the sentinel's own comment + assert message cited 'task0299_* / task0303_*' as vacuous-pass-prone, but task0303_05's '== 1' strict-positive pin is NOT vacuous-pass-prone. The orchestrator's OWN narrative at sidecar_halo.rs:717-719 said as much — the sentinel's freshly-landed comment contradicted it. Recurring 'feedback-comment-doc-lie' pattern firing on a comment whose explicit purpose was doc-lie defence. Folded back: tightened glob to 'task0299_06' + 'task0303_07' in both inline comment + assert message.
  * NEW P2 #2 (tracker-md stale citations): 5 phantom-record_halo / stale-line-number citations across 4 task markdown files (task-0299/0303/0305/0307). The 'replace_all' was scoped to nucleus/ source only. Filed as TASK-0308 follow-up (LOW, hygiene-only; non-blocking since tests pass + production protection is real).

GOTCHAS + FORWARD-CARRY (for stateless future implementers):
- The 'feedback-silent-sibling-defect' pattern fired TWICE in this cycle: (a) only task0303_07 docstring updated (architect's P1 #1) — fix scope must match the structurally-identical sibling set, not just the most-cited test; (b) 'replace_all' for the phantom symbol was scoped to nucleus/ but the same lie lived in 4 backlog/tasks/ files (architect's NEW P2 #2 → TASK-0308). The general principle: when fixing a defect, grep the WHOLE repo (code + tracker + memory + docs), not just the obvious code path.
- The 'feedback-comment-doc-lie-recurring' pattern fired THREE TIMES: (a) the inherited phantom 'record_halo' from cycle-122; (b) the orchestrator's own propagation of the phantom to 2 new sites + 1 updated site in cycle 123; (c) the orchestrator's freshly-landed sentinel-comment overclaim ('task0303_*' glob). The general principle: every new comment is a CLAIM that must be verified against the code; the highest comment-doc-lie risk is on comments whose stated purpose is doc-lie defence (because the narrative spirals).
- The 'feedback-orchestrator-narrative-also-wrong' pattern is the dual of the above: even an orchestrator's review-gate-driven hardening commit produces fresh comment-doc-lies, not just implementers' disclosures. The parallel architect-review gate caught the spiral in 2 rounds; a single-round review would have shipped the inconsistency.
- 'classify_index' is a confusing function name for the role it plays (the function CLASSIFIES the index expression — affine/strided/data-dependent — AND emits the halo entry via 'per_iv.entry(iv).or_insert(0)'). Renaming to e.g. 'classify_index_and_record_halo' would make 'record_halo'-style citations TRUE but is a larger ergonomics decision and not part of this cycle.
- Backlog CLI gotcha: 'backlog task create -p Low ...' interprets '-p' as '--parent' (subtask), not '--priority'. The correct flag is '--priority Low'. Bad task was archived (TASK-LOW.01); recreated as TASK-0308.

FILES SHIPPED (cycle 123):
- nucleus/nucleus-compiler/src/passes/halo_inference.rs (+44 / -2): contract paragraph extension + structural sentinel in no_halo_bare_iv test.
- nucleus/nucleus-compiler/tests/sidecar_halo.rs (+41 / -15): task0299_06 LANDED-sentinel paragraph + symbolic-anchor migrations + task0303_07 past-tense LANDED-form update + the renaming of 'record_halo' → 'classify_index' (single site).
- backlog/tasks/task-0307 - ... (status: To Do → Done; Plan + Notes + Final Summary written).
- backlog/tasks/task-0308 - ... (new, TASK-0307 cycle-123 architect P2 follow-up filing).

CROSS-REFERENCES:
- TASK-0305 cycle 122 — the Option B contract decision this defends.
- TASK-0299/0303 — the narrative-pin precedents whose vacuous-pass arm the sentinel closes.
- TASK-0308 (new) — the across-boundary tracker-md hygiene sweep (the silent sibling on the code/tracker boundary).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0307 cycle-123 (2026-05-25) LANDED. Structural Some(0) sentinel inside the in-module 'no_halo_bare_iv' test defends TASK-0305 cycle-122 Option B (absent ≡ explicit-0 contract degree of freedom) by closing the vacuous-pass arm at the contract boundary — without coupling the downstream 'task0299_06 / task0303_07' narrative pins to the explicit-0 representation. AC#1 satisfied (structural pin at no_halo_bare_iv, sound message). AC#2 satisfied (verified via classify_index→or_insert(0) trace at halo_inference.rs:762→861; sentinel directly observes the production sink). AC#3 satisfied with the CHARITABLE interpretation (keep production-sink search hint + add sentinel reference) — both architect reviews accepted this. Review-driven hardening closed 2 P1s (silent-sibling sweep on task0299_06; phantom 'record_halo' symbol replaced with real 'classify_index' across 4 sites in nucleus/) + 2 P2s (sentinel-comment overclaim glob tightened; stale 'halo_inference.rs:53-57' absolute-line citations migrated to symbolic anchors). TASK-0308 filed as follow-up for the tracker-md phantom-citation sweep (5 sites across 4 task md files; scope-of-fix gap that the architect's NEW P2 caught). Gate: 850/0/3 tests dev + release, e2e 108/92/0/16/0 unchanged.
<!-- SECTION:FINAL_SUMMARY:END -->
