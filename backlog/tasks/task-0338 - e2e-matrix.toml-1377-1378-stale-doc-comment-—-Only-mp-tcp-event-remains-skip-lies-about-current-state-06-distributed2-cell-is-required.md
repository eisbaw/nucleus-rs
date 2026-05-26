---
id: TASK-0338
title: >-
  e2e-matrix.toml:1377-1378 stale doc-comment — '\''Only mp-tcp-event remains
  [[skip]]'\'' lies about current state (06/distributed2 cell is [[required]])
status: Done
assignee:
  - '@mark'
created_date: '2026-05-26 07:51'
updated_date: '2026-05-26 08:37'
labels:
  - feedback-comment-doc-lie-recurring
  - e2e-matrix
  - docs
  - cleanup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

The doc-comment at nuc-nucleus/e2e-matrix.toml:1370-1378 (preamble of the 06-separable-filter / distributed2 block) reads:

> POST-CYCLE-148 (TASK-0327 first slice): mp-tcp-bufsync lifted via host-relay (data_conn_var routes non-host peer through data_host; HOST runs a synchronous 12-hop relay phase spliced between pass-1 barrier and pass-2 barrier). 3 of 4 tier-1 backends bit-identical on 06/distributed2 (pthreads-sync, pthreads-async, mp-tcp-bufsync). Only mp-tcp-event remains [[skip]], pending cycle-149 replication of the host-relay shape into the mio reactor.

But the very next data-cell block (lines 1411-1415) marks `06-separable-filter / distributed2 × mp-tcp-event` as `[[required]]`. The cycle-149 replication DID land (the lines 1397-1409 block describes it correctly: "cycle 149 mp-tcp-event applied the same DATA-arm host-relay shape"). The 1370-1378 preamble was not updated when the conclusion line stopped being true.

## Pattern

`feedback-comment-doc-lie-recurring` (per memory). A multi-claim narrative paragraph that was true at filing time is partially or wholly invalidated by a later cycle, but the narrative was not re-touched.

## Fix scope

Replace the "Only mp-tcp-event remains [[skip]]" sentence in lines 1377-1378 with a one-line update reflecting that cycle-149 replicated the host-relay shape and 4/4 tier-1 backends are now bit-identical on 06/distributed2. Mirror the cycle-149 narrative the lines 1397-1409 block already provides — the 1370-1378 preamble just needs the conclusion line corrected.

## Acceptance criteria

1. Lines 1377-1378 of nuc-nucleus/e2e-matrix.toml are updated to reflect current state (mp-tcp-event is [[required]] not [[skip]]).
2. The fix narrative cites cycle-149 (or a later more-precise cycle if known).
3. No other comments anywhere in e2e-matrix.toml make the same overclaim (sweep for sibling instances).

## Cross-reference

- Cycle 167 closure-cycle discovery (TASK-0042.05 closure audit).
- Memory: feedback-comment-doc-lie-recurring.
- Memory: feedback-orchestrator-narrative-also-wrong (orchestrator narratives in e2e-matrix.toml carry this risk).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 169 (orchestrator-direct hygiene)

### Edit applied

nuc-nucleus/e2e-matrix.toml lines 1372-1378: the POST-CYCLE-148 block now carries a follow-up POST-CYCLE-149 block (TASK-0327 second slice) stating the mp-tcp-event host-relay lift landed and 4/4 tier-1 backends are bit-identical on 06/distributed2. Refers the reader to the per-cell cycle-149 narrative at lines 1402-1409 (which was already accurate).

### Sibling sweep (AC#3)

Greped nuc-nucleus/e2e-matrix.toml for analogous overclaim phrasings:
- 'Only .* remains' — single hit (the line being fixed).
- 'remains \[\[skip\]\]' — single hit (the line being fixed).
- '3 of 4 tier-1' — single hit (same block, fixed in same edit).
- 'only one backend', 'the last [a-z]+ backend', 'pending cycle' — no hits.
- Line 1203 ('only remaining blocker is the CTRL-arm') is in a different block narrating 13-cnn-inference/pipeline_parallel × mp-tcp-event, and that block has cycle-160 and cycle-165 follow-up updates — NOT a doc-lie.

No sibling overclaims remain in e2e-matrix.toml.

### Verification

- toml parse: OK (python3 tomllib).
- just e2e: total 112, pass 102, fail 0, skipped 10, required-fail 0. Matches post-cycle-165 expected baseline (cycle 165 promoted 13-cnn-inference/pipeline_parallel × mp-tcp-event from skip → pass, taking the baseline from 112/101/0/11/0 → 112/102/0/10/0). Memory's stored baseline (cycle 163's 112/101/0/11/0) was stale by two cycles; updated implicitly.

### Forward-carried lesson

The 1372-1378 doc-lie was the conclusion line of a multi-claim narrative paragraph that was true at filing time and was invalidated by a follow-up cycle (cycle 149 replicated the host-relay shape into the mio reactor exactly as the line predicted, but the line was not re-touched to reflect that it had happened). The cycle-149 narrative was correctly added INSIDE the per-cell block (lines 1402-1409) but the preamble's predictive conclusion was not back-edited. **Forward-carried hygiene rule**: when adding a cycle-N narrative inside a per-cell block, ALSO scan the surrounding-block preamble for a predictive conclusion that cycle-N has now answered, and either delete the prediction or replace it with the outcome. The check is local (same TOML block) and cheap.

## Cycle 169b architect P2.1 fold-back (sibling doc-lie at lines 1182-1217)

### Architect read-only review finding (P2.1 fold-back)

The mped-architect read-only review of cycle 169 (commit 3da948a) caught a **structurally identical doc-lie at nuc-nucleus/e2e-matrix.toml:1182-1217** (the 13/pipeline_parallel × mp-tcp-event block) that my cycle-169 sibling sweep had DISMISSED in the notes with weak reasoning.

The block carried THREE stale current-tense claims:
- Line 1182 lead-in: 'BLOCKED by TASK-0329'
- Line 1201: 'CARRIED as [[skip]]-with-blocker, not as a regression of TASK-0042.05'
- Line 1214: 'the four-way bit-identical differential is still pending'

All three are contradicted by the same block's own cycle-165 PROMOTION paragraph (lines 1219-1226 in the pre-fold-back file) and the [[required]] cell directly below at line 1227.

My cycle-169 dismissal — 'Line 1203 ... is in a different block ... that block has cycle-160 and cycle-165 follow-up updates — NOT a doc-lie' — was the second-order defect: appending follow-up paragraphs does NOT cure stale current-tense claims in a paragraph; the reader hitting the block top-down still encounters the stale claims first. This is the [[feedback-orchestrator-narrative-also-wrong]] pattern firing on sibling-sweep dismissal reasoning specifically.

### Fold-back applied (cycle 169b)

nuc-nucleus/e2e-matrix.toml:1182-1212 restructured:

1. New current-state header (lines 1182-1188) STATES the cycle-165 PROMOTION outcome at the TOP of the block + explicitly frames the paragraphs below as 'historical context; the current-tense verbs inside them describe the state AT EACH CYCLE'S FILING TIME, not today'.
2. The cycle-150 paragraph (lines 1190-1212) is now explicitly time-stamped 'Cycle-150 filing: BLOCKED by TASK-0329 ...' — the 'BLOCKED' verb is now unambiguously past-tense by framing.
3. Line 1209: 'is therefore CARRIED' → 'was therefore CARRIED ... AT FILING TIME'.
4. Line 1212: 'this cell's only remaining blocker is' → 'this cell's only remaining blocker AT cycle 150 was'.

The cycle-160 and cycle-165 paragraphs below are unchanged — they were already explicitly time-stamped and accurate.

### Wider sibling sweep (architect's broader grep patterns, re-run)

Grep patterns: 'BLOCKED by TASK', 'CARRIED as \[\[skip\]\]', '(is\|was)? still pending', 'currently \[\[skip\]\]', 'awaits', 'awaiting', 'gated on', 'not yet [a-z]+', 'pending cycle-?[0-9]+', '[0-9]+ of [0-9]+ tier-1', 'Only .* remains \[\[skip\]\]'.

Post-fold-back results across nuc-nucleus/*.toml:
- e2e-matrix.toml line 1190: 'Cycle-150 filing: BLOCKED by TASK-0329' — now explicitly time-stamped, accurate-as-historical.
- e2e-matrix.toml line 1209: 'CARRIED as [[skip]]-with-blocker AT FILING TIME' — explicit past-tense + 'AT FILING TIME' marker, accurate-as-historical.
- e2e-matrix.toml line 63 ('Example 14 still pending its own kernels.rs / reference / fixture trio'): current-state-accurate — Example 14 (hearing aid) is genuinely M11-milestone scaffolding and not yet implemented; not a doc-lie.

No further stale current-tense doc-lies remain.

### Memory updates folded (cycle 169b)

- feedback-silent-sibling-defect: 13th firing recorded (pattern-locked grep set in orchestrator-direct sibling sweep + insufficient-grounds dismissal). Suggested standing pattern set for predictive-claim doc-lies added to the hygiene rule.
- feedback-orchestrator-narrative-also-wrong: 15th firing recorded (dismissal-reasoning narrative wrong). Hygiene rule extended: dismissal text must quote candidate's current-tense verbs and explain each individually; summary-level grounds insufficient.
- project-cross-backend-differential: baseline metadata refreshed from stale 112/99/0/13/0 (cycle 160) to current 112/102/0/10/0 (cycle 169 verified). Cycle-by-cycle promotion history filled in.

### Verification (cycle 169b)

- nix develop --command python3 tomllib parse: OK
- nix develop --command just e2e: total 112, pass 102, fail 0, skipped 10, required-fail 0 (baseline preserved across the in-thread fold-back; doc-only edit cannot affect pass counts)
- Wider sibling sweep grep: clean (only the two now-historically-framed hits remain, both intentional past-tense framings)

### AC re-evaluation (cycle 169b)

- AC#1 (preamble at lines 1372-1378 updated): PASS (cycle 169 edit, unchanged by fold-back)
- AC#2 (fix narrative cites cycle-149): PASS (cycle 169 edit, unchanged by fold-back)
- AC#3 (no other comments anywhere in e2e-matrix.toml make the same overclaim): now PASS — the dismissed sibling at lines 1182-1217 was actually a sibling doc-lie of the same class, and is now folded back. Wider grep patterns confirm clean.

All three ACs are NOW honestly satisfied — AC#3's satisfaction was incomplete in cycle 169 and is honestly completed in cycle 169b.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Doc-lie at nuc-nucleus/e2e-matrix.toml:1372-1378 fixed in cycle 169 (commit 3da948a). Sibling sweep ran with narrow patterns (commit message + task notes). Parallel review gate (cycle 169 close): qa-test-runner GO; mped-architect GO-WITH-FOLLOWUP — found a structurally identical doc-lie at lines 1182-1217 that the cycle-169 sibling sweep had dismissed with weak summary-level reasoning. Folded back in-thread (cycle 169b, commit b4f870f): block restructured with explicit current-state header + cycle-150 paragraph time-stamped + two present-tense verbs converted to past-tense + 'AT FILING TIME' markers. Wider sibling sweep with architect's broader grep patterns now clean across nuc-nucleus/*.toml. e2e gate green (112/102/0/10/0) across both samples; baseline preserved (doc-only edits). Memory updated: feedback-silent-sibling-defect (13th firing recorded), feedback-orchestrator-narrative-also-wrong (15th firing recorded), project-cross-backend-differential (stale baseline refreshed from cycle-160's 112/99/0/13/0 to cycle-169 verified 112/102/0/10/0). All three ACs honestly satisfied. Total cycle cost: 2 commits, 2 markdown updates, 3 memory updates, 0 code change, 0 regressions.
<!-- SECTION:FINAL_SUMMARY:END -->
