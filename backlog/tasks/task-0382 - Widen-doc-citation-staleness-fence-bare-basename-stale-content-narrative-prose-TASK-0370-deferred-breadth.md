---
id: TASK-0382
title: >-
  Widen doc-citation-staleness fence: bare-basename + stale-content +
  narrative-prose (TASK-0370 deferred breadth)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-31 02:20'
updated_date: '2026-05-31 04:06'
labels:
  - tooling
  - ci
  - doc-lie
  - robustness
  - cycle-220-followup
dependencies:
  - TASK-0370
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 TASK-0370 delivered check-doc-citation-staleness covering FULLY-QUALIFIED nucleus/*.rs:N citations (file-exists + line-in-range) over source/docs/READMEs/PRD/nuc-nucleus, excluding backlog/tasks. Three lie-shape/target classes were DEFERRED because they resist zero-FP mechanization in one cycle (empirically measured in TASK-0370):

(i) BARE-BASENAME citations (lib.rs:N, multi_worker.rs:N) — the BULK of source citations. Ambiguous resolution root (12+ lib.rs files) and cross-crate prose references (e.g. check_frame.rs "pre-extraction pthreads-sync at lib.rs:991") MISATTRIBUTE under a naive crate-relative resolver. Needs a crate-scoped, prose-aware resolver (only validate when the basename is unique within the citing crate AND the surrounding prose does not name another crate). Genuine same-crate stale citation example currently in-tree but uncaught: mp-tcp-event/tests/multi_worker_emit.rs:646 cites multi_worker.rs:854 (file now 296 LoC).

(ii) STALE-CONTENT detection — line still exists but the code at it moved (e.g. docs cited pthreads-sync/src/lib.rs:694..758 for single-worker check-emit; line 694 now holds render_reuse_marker_comment). Line-count check cannot see this; needs a content fingerprint / symbol-anchor convention.

(iii) PRESENT-TENSE NARRATIVE PROSE scanning of *.md and *.sched.nuc headers — the existing check-narrative-doc-lie pattern set FP-floods here (171 legitimate hits on backlog/tasks alone). Needs either a high-discipline curated target (not general prose) or a fundamentally different objective shape.

Also consider: should backlog/tasks citations be validated at all? Currently excluded as immutable filing-time-historical provenance (CLAUDE.md forbids hand-editing task md). A "historical-citation must carry a cycle/filing stamp" convention could make them auditable without rewriting history — design question, not obviously worth it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Bare-basename citations validated zero-FP via a crate-scoped prose-aware resolver (or a documented decision that this stays out of scope)
- [x] #2 Stale-content (line-exists-but-code-moved) detection OR a project convention (symbol-anchor mandate) that makes it moot
- [x] #3 Decision recorded on present-tense narrative-prose scanning of md/.sched.nuc (curated target or explicit out-of-scope)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
CYCLE-221 PLAN (implementer):
AC#1 (bare-basename): IMPLEMENT a sibling recipe check-doc-citation-staleness-bare. Crate-scoped, prose-aware, skip-favouring resolver over *.rs only: (a) crate root = nearest ancestor Cargo.toml; (b) resolve <base>.rs via find -name within crate, SKIP if !=1 match (ambiguous / partial-path-stripped); (c) cross-crate-prose guard scans citation line +WIN lines above for any OTHER crate name in BOTH dash and underscore forms, SKIP if found; (d) range-check N. WIN=3 (empirically: WIN=1 FPs check_frame.rs lib.rs cite; WIN=6 over-skips on common token e2e). Wire into ci after the FQ sibling. PROVE zero-FP on full tree + PROVE it bites (inject same-crate past-EOF cite).
AC#2 (stale-content): DECISION out-of-scope — line-count cannot see content drift; recommend cycle-138 symbol-anchor convention as mitigation. Record reasoning.
AC#3 (narrative md/.sched.nuc prose): DECISION explicit out-of-scope — existing pattern set FP-floods (171 hits on backlog/tasks per TASK-0370). Record.
File TASK-0384 for the SAFE deferred coverage gaps (partial-path-prefix-honouring resolver; bare-basename-as-prose-location no-:N variant).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-220 TASK-0383 split produced a fresh in-tree instance of the BARE-BASENAME-LOCATION lie this task's AC#1 targets, and it was caught ONLY by manual silent-sibling grep — the check-doc-citation-staleness fence is structurally blind to it. Concrete: splitting the BIN tests out of embedded-pattern/src/tests.rs into tests/bin_shape.rs left a comment in embedded-pattern/src/lib.rs reading 'pinned by bin_rejects_multi_worker_* in tests.rs' — now a LIE (the test lives in tests/bin_shape.rs). It carried no `:N` line number (just the bare basename `tests.rs` as a location claim in prose), so the fully-qualified fence never saw it. This is a NEW lie-shape variant for AC#1's scope: not 'bare basename + line number' but 'bare basename as a prose location claim with NO line number'. A crate-scoped resolver keyed on `<basename>.rs` tokens in `//` comments (validating that a named test/symbol still resides in the cited file) would have caught it. Generalises: ANY split-and-cite cycle (the TASK-0340 epic + this sibling) is a high-yield source of exactly this defect — worth a dedicated grep arm even before the full prose-aware resolver lands.

CYCLE-221 OUTCOME (implementer, all measurements empirical on the dirty working tree at commit 6aac40a).

AC#1 — IMPLEMENTED zero-FP. New sibling recipe just check-doc-citation-staleness-bare, wired into just ci immediately after the FQ sibling check-doc-citation-staleness. Crate-scoped prose-aware resolver over *.rs only; every rule biases to SKIP (over-skip is safe, under-skip risks an FP).

FP MEASUREMENTS (the data that drove the design):
- Total bare <base>.rs:N hits scanned in *.rs (with multiplicity): ~146. Distinct verdict classes at WIN=3: 111 OK (validated in-range), 12 SKIP:ambiguous (basename in >1 crate file, e.g. nucleus-compiler algo/ir.rs + sched/ir.rs), 11 SKIP:basename-not-in-crate (mostly partial-path forms whose prefix the basename-only resolver discards), 31 SKIP:cross-crate-prose.
- GENUINE stale catches on the current tree: ZERO. This is a REGRESSION-PREVENTION fence (same posture as check-mega-files on a clean tree). The one historically-known live stale bare cite (multi_worker_emit.rs cited multi_worker.rs:854) was already hand-fixed in commit 6aac40a; it now reads multi_worker.rs:174-186 against a 296-line file -> validates OK.
- The resolver BITES: injecting a same-crate past-EOF cite (wait.rs:99999 in backend-common collect.rs) makes the recipe exit 1 with a STALE line; removing it returns exit 0. Verified through the REAL just recipe, not just a scratch script.

LOAD-BEARING ZERO-FP RULES + WHY each is needed (each was an actual reproduced FP or near-FP):
1. cross-crate-prose guard scanning the citation line + WIN lines ABOVE. WIN=1 (same-line only) FALSE-POSITIVED check_frame.rs (backend-common) lib.rs:1010-1018: the words "pthreads-syncs pre-extraction comment" sit 3 lines above the cite on a wrapped /// line; a same-line check resolves lib.rs to backend-common/src/lib.rs (92 lines) and reports STALE for a cite that means pthreads-sync. WIN=3 fixes it.
2. crate-name matching in BOTH dash-form (pthreads-sync) AND module-path underscore-form (pthreads_sync). Two cites (multi_worker.rs:237 / :392 in pthreads-async) name pthreads_sync ONLY via the ::-path form pthreads_sync::multi_worker::Plan; dash-only matching MISSED them and they resolved in-range against pthreads-asyncs OWN multi_worker.rs by luck (wrong file, near-FP). Adding the underscore form reclassified both to SKIP.
3. WIN is deliberately SMALL (=3). WIN=6 OVER-skips a legitimate same-crate expr.rs cite in backend-common because a sentence 4 lines up says "the 7 shipped e2e gather cells" — e2e/nucleus/driver double as common domain words. The tie breaks toward SKIP, so over-skip costs only coverage, never a false alarm; but WIN=3 is the measured minimum that still avoids the rule-1 FP.

GOTCHA for the next person: the cross-crate-prose guard is a HEURISTIC, not a proof. It can still OVER-skip (a same-crate cite whose window incidentally contains another crate name) — that is BY DESIGN (zero-FP-favouring). It can in principle UNDER-skip if a future cite references another crate by NEITHER dash NOR underscore form NOR a nucleus/<crate>/ path within WIN lines (none exist today). If a future FP appears, widen the crate-name forms or WIN — do NOT relax to validate more.

AC#2 — DECISION: out of mechanized scope. Stale-CONTENT (line still exists, code at it moved) is invisible to any line-count check; detecting it needs a content fingerprint or a symbol-anchor convention. The project ALREADY has the mitigation: the cycle-138 prefer-a-stable-symbol-anchor-over-a-line-number rule (it is fix-preference #1 in BOTH citation recipes fix-messages). A cite anchored to a symbol name is immune to the very drift AC#2 names. Recommending/relying on that convention makes AC#2 moot WITHOUT shipping an FP-prone content checker. No code shipped for AC#2.

AC#3 — DECISION: explicit out-of-scope. Present-tense narrative-prose scanning of *.md / *.sched.nuc headers FP-FLOODS (TASK-0370 measured 171 legitimate hits on backlog/tasks alone with the existing check-narrative-doc-lie pattern set; the patterns capture true domain language not lies). A zero-FP prose scanner needs either a high-discipline curated single target (the existing check-narrative-doc-lie already serves that role for nuc-nucleus/e2e-matrix.toml) or a fundamentally different objective shape. The OBJECTIVE-citation approach (this task AC#1 + TASK-0370) is the zero-FP path for these locations; general prose scanning is not pursued. No code shipped for AC#3.

FOLLOW-UP filed: TASK-0382.01 (partial-path-prefix-honouring resolver + the no-:N bare-basename-as-location variant from the Implementation Notes). Both are SAFE coverage gaps (currently SKIP, never FP), purely additive.

VERIFICATION done: just check-doc-citation-staleness-bare OK (zero-FP) + bite-tested + cross-crate-guard-tested (a pthreads-sync-named past-EOF cite injected into a backend-common file is correctly SKIPPED, no false alarm). Sibling cheap fences green: check-doc-citation-staleness, check-narrative-doc-lie, check-mega-files, check-doc-links all OK. Change is justfile/shell ONLY (no .rs edits) so clippy/test/e2e/codegen are unaffected — I did NOT run the full heavy just ci (e2e/determinism); the read-only review gate will.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DELIVERED cycle-221 (commit dae8404).

AC#1 IMPLEMENTED zero-FP: new recipe check-doc-citation-staleness-bare wired into just ci after the FQ sibling. Crate-scoped, prose-aware, skip-favouring bare-basename resolver over *.rs. Proven zero-FP on the full tree (111 OK / 12 ambiguous-skip / 11 not-in-crate-skip / 31 cross-crate-prose-skip; ZERO false positives). Proven to BITE (injected same-crate past-EOF cite -> recipe exits 1 with STALE; restored -> exit 0) AND proven NOT to false-alarm on cross-crate cites (a pthreads-sync-named past-EOF cite in a backend-common file is correctly SKIPPED). Three skip rules are each an empirically-reproduced FP/near-FP fix (windowed cross-crate-prose guard at WIN=3; dash+underscore crate-name forms; small window to avoid over-skip on common tokens) — see Implementation Notes.

ZERO genuine catches on the current tree (the one known live stale bare cite was hand-fixed in 6aac40a). This is a REGRESSION-PREVENTION fence (same posture as check-mega-files): it defends FUTURE bare-basename citations, especially from the split-and-cite cycles (TASK-0340 epic + the TASK-0383 sibling) that recurrently produce exactly this lie shape.

AC#2 DECISION (out of mechanized scope): stale-CONTENT (line exists, code moved) is invisible to any line-count check; the cycle-138 prefer-a-stable-symbol-anchor convention (fix-preference #1 in both citation recipes) is the existing mitigation and makes it moot without an FP-prone content checker. No code shipped — decision recorded, which AC#2 wording ("OR a project convention that makes it moot") admits.

AC#3 DECISION (explicit out-of-scope): present-tense narrative-prose scanning of md/.sched.nuc FP-floods (171 legitimate hits on backlog/tasks; the patterns capture true domain language). The objective-citation approach (AC#1 + TASK-0370) is the zero-FP path for these locations; general prose scanning not pursued. Decision recorded, which AC#3 wording ("curated target or explicit out-of-scope") admits.

FOLLOW-UP: TASK-0382.01 (partial-path-prefix-honouring resolver + no-:N bare-basename-as-location variant) — both SAFE additive-coverage gaps, never FP.

GATE: change is justfile/shell ONLY (no .rs), so no codegen/clippy/test/e2e impact. Cheap sibling fences green: check-doc-citation-staleness, check-doc-citation-staleness-bare, check-narrative-doc-lie, check-mega-files, check-doc-links all OK. Heavy just ci (e2e/determinism) deliberately NOT run by implementer (no codegen change); read-only review gate runs full just ci to confirm the new arm is green end-to-end.
<!-- SECTION:FINAL_SUMMARY:END -->
