---
id: TASK-0198
title: >-
  SchedLowerError unknown-name fuzzy-match did-you-mean (sched sibling of
  TASK-0096)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 17:18'
updated_date: '2026-05-19 18:00'
labels:
  - compiler
  - link
  - diagnostics
  - M0-followup
dependencies:
  - TASK-0096
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Sibling of TASK-0096 (which does link-step LinkError fuzzy-match). SchedLowerError has unknown-name variants that are candidates for the same Levenshtein did-you-mean treatment: UnknownAccessibleByName{region,name} (TASK-0095), UnknownPlaceWorker, UnknownWorkerClass, UnknownMemoryRegion. The schedule symbol tables (ir.worker_classes, ir.workers, ir.memory_regions) are in hand at the SchedIR lowering site (nucleus/compiler/src/sched/lower.rs). Reuse the deterministic zero-dep edit-distance helper TASK-0096 introduces (do not duplicate it). Kept separate from TASK-0096 to keep each cycle bounded (different error type/module/tables); TASK-0096 establishes the helper + pattern this mirrors. decision-0003: typed-Result, no panic; suggestion must be deterministic (reproducibility gate); zero behaviour change for valid input.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SchedLowerError unknown-name variants (UnknownAccessibleByName, UnknownPlaceWorker, UnknownWorkerClass, UnknownMemoryRegion) carry an Option<String> did-you-mean suggestion computed via the shared TASK-0096 edit-distance helper against the in-hand schedule symbol table; deterministic tie-breaking
- [x] #2 Negative tests assert the suggestion when one is computable and None when not (e.g. no close candidate); existing SchedLowerError negative tests migrate with assertion strength preserved
- [x] #3 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); typed-Result, no panic; zero behaviour change for valid input
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. REUSE crate::error::suggest (TASK-0096 helper, error.rs ~176) — no duplicate edit-distance fn.
2. WRAPPER-SHAPE MECHANIC (corrected forward-carry): SchedLowerError is a {kind,span} wrapper with HAND-WRITTEN PartialEq forwarding to .kind only (sched/ir.rs ~614, TASK-0196 span-exclusion contract). Do NOT touch/derive the wrapper. ADD suggestion:Option<String> to the 4 SchedLowerErrorKind VARIANTS (inner enum, #[derive(PartialEq,Eq)] ~318) so suggestion folds into .kind equality automatically while span stays excluded via the unchanged wrapper hand-eq.
3. Per-variant candidate sets at the 4 err sites in sched/lower.rs:
   - UnknownWorkerClass (~297): class vs ir.worker_classes.keys()
   - UnknownAccessibleByName (~337): name vs DETERMINISTIC union ir.worker_classes.keys() chain ir.workers.keys() (both BTreeMap; matches the validity rule worker_class OR worker; no Hash in path)
   - UnknownPlaceWorker (~441 check_worker_declared): worker.node vs ir.workers.keys()
   - UnknownMemoryRegion (~473 lower_place_data): region vs ir.memory_regions.keys()
4. Display: mirror TASK-0096 write_suggestion helper (append ` -- did you mean \`X\`?` only on Some; byte-identical on None). Add to the 4 Kind Display arms in sched/ir.rs (~441-455).
5. De-stale the doc-lie at lower.rs ~312-314 ("Did-you-mean fuzzy suggestions are deliberately out of scope — that is TASK-0096.") and any sched/ir.rs Kind-type doc.
6. Migrate existing sched_lower negative tests: keep assert_eq!(err.kind, ...) strength, ADD suggestion field to expected value. The two span-tests using matches!{...fields...} need suggestion handled (add ..) — preserve their span discriminating power, add explicit suggestion assert. NEW tests AC#2: typo→Some, unrelated→None per variant.
7. Gate: just test, e2e 30/26/0/4/0, determinism x2 + both negatives bite, clippy --all-targets, ci. Real-driver did-you-mean evidence (Some + None).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0096 (DONE, commit 6d39609):

REUSE — do NOT duplicate. The shared zero-dep helper now lives in nucleus/compiler/src/error.rs:
  pub fn levenshtein(a: &str, b: &str) -> usize
  pub fn suggest<'a, I>(name: &str, candidates: I) -> Option<String> where I: IntoIterator<Item = &'a str>
`compiler::error` is `pub mod`, so from sched/lower.rs call `crate::error::suggest(&name, ir.worker_classes.keys().map(String::as_str))` etc. (mirror the link.rs emit sites).

Policy to mirror exactly (already implemented + gate-proven in 0096):
- Plain Levenshtein, char-wise, O(n*m) two-row DP. Transposition costs 2 (NOT Damerau) — intentional, decision-0001 minimality.
- Threshold: distance <= max(1, name.chars().count()/3). Do NOT invent a different bound.
- Tie-break: suggest() sort_unstable()s candidates then strict-< keeps the lexicographically-first equal-distance candidate. Pass it BTreeMap/BTreeSet keys (sched ir.worker_classes/workers/memory_regions are already deterministically ordered) — no HashMap/HashSet in the selection path (determinism gate).

LinkError-shape precedent for SchedLowerError: TASK-0096 widened each unknown-name variant to a `{ name, suggestion: Option<String> }` struct (NOT a {kind,suggestion} wrapper — wrapper only justified when EVERY variant gets a uniform field, as TASK-0090s span did). Apply the same per-variant struct widening to UnknownAccessibleByName/UnknownPlaceWorker/UnknownWorkerClass/UnknownMemoryRegion (preserve UnknownAccessibleByName{region,...}, add suggestion alongside).

PartialEq precedent (important divergence from TASK-0090): DERIVE PartialEq/Eq so the suggestion IS part of value identity — do NOT hand-exclude it the way LowerError excludes span. A suggestion is a deterministic pure fn of (name, in-hand table); equal-name errors against the same table necessarily have an equal suggestion, so derived Eq is correct and the negative tests assert the suggestion as part of the expected value. Document this on the SchedLowerError type as TASK-0096 did on LinkError.

Test-migration discipline: keep existing whole-Vec assert_eq! strength; ADD the suggestion field to the expected value (do not weaken to assert!/contains). Add positive-suggestion + None-for-unrelated cases. The helper itself is already unit-tested in error::fuzzy_tests — no need to re-test levenshtein/suggest, only the SchedLowerError wiring.

Doc-honesty: if sched/lower.rs module doc claims no fuzzy-match, de-stale it (TASK-0090 doc-lie defect class).

CORRECTION (orchestrator, from TASK-0096 mped-architect review) to the forward-carried PartialEq mechanic — the TASK-0096 note said "DERIVE PartialEq/Eq, do NOT hand-exclude". That is correct for LinkError (a BARE enum with per-variant payloads — deriving folds the suggestion into value identity, right). It is WRONG for SchedLowerError: TASK-0196 made SchedLowerError a `{ kind: SchedLowerErrorKind, span: Option<Range> }` WRAPPER struct with a HAND-WRITTEN impl PartialEq forwarding to `.kind` ONLY (sched/ir.rs ~561, ~619-622) to deliberately EXCLUDE span. Do NOT #[derive(PartialEq)] on the SchedLowerError wrapper (that would re-include span and break the TASK-0196 located-diagnostic contract + its negative tests). CORRECT mechanic: add the `suggestion: Option<String>` field to the relevant SchedLowerErrorKind VARIANTS (the inner enum, which IS derived PartialEq) — the suggestion then folds into `.kind` equality automatically while the outer hand-written eq keeps excluding ONLY span. Intent unchanged (suggestion IN value identity, span OUT); only the placement differs because of the wrapper shape. General rule: a NEW deterministic semantic field goes on the Kind enum/variant (derived eq); span/positional-noise stays excluded via the wrapper hand-eq.

IMPLEMENTED (commit a453f73).

Wrapper-shape mechanic (corrected forward-carry FOLLOWED): suggestion:Option<String> added to the 4 SchedLowerErrorKind VARIANTS (inner enum, sched/ir.rs line 318 #[derive(Debug,Clone,PartialEq,Eq)]) so it folds into .kind value identity automatically. The outer SchedLowerError{kind,span} wrapper (line 649 #[derive(Debug,Clone)], line 708 hand-written impl PartialEq forwarding to .kind only) was NOT touched / NOT given a PartialEq derive -> TASK-0196 span-exclusion contract preserved. Net: suggestion IN value identity, span OUT. Confirmed by reading the wrapper eq before changing anything and re-grep after.

Per-variant candidate sets (all BTreeMap key iterators, deterministic; suggest() also sorts + strict-< tie-break; NO HashMap/HashSet in path):
- UnknownWorkerClass: class vs ir.worker_classes.keys()
- UnknownMemoryRegion: region vs ir.memory_regions.keys()
- UnknownPlaceWorker: worker.node vs ir.workers.keys()
- UnknownAccessibleByName: name vs ir.worker_classes.keys().chain(ir.workers.keys()) -- the deterministic union, matching the variant own validity rule (declared worker_class OR worker).

Reused crate::error::suggest verbatim (no duplicate edit-distance fn). New write_suggestion Display helper mirrors link-step verbatim; None path byte-identical.

Doc de-staled: sched/lower.rs ~312 "Did-you-mean fuzzy suggestions are deliberately out of scope -- that is TASK-0096" rewritten to describe the implemented behaviour (TASK-0090 doc-lie defect class).

sched_lower test churn (assertion strength PRESERVED, not loosened):
- negative_unknown_worker_class_reference: assert_eq!(.kind) kept; added suggestion:None (missing_class vs {__default} far > bound 4).
- negative_unknown_memory_region_reference: assert_eq!(.kind) kept; suggestion:None (no memory_region declared).
- negative_place_references_unknown_worker / negative_place_set_references_unknown_worker: assert_eq!(.kind) kept; suggestion:None (bogus vs host/w0 > bound 1).
- negative_undeclared_accessible_by_name: assert_eq!(.kind) kept; suggestion:Some("host") (ghost->host dist 1 <= bound 1, union {__default,host}) -- this MIGRATED case is the positive half for that variant.
- TASK-0196 span tests case 2/3: matches! kept its full variant+payload discriminating power (added only `, ..`), PLUS an explicit suggestion pin added (case2 None, case3 Some host) -> STRENGTHENED not weakened. case3 display_with_src expected string updated to include the hint (real intended behaviour change in the located display; ` at L:C` suffix unchanged -> TASK-0196 intact).
NEW tests: negative_unknown_worker_class_with_suggestion (Some core), _unrelated_no_suggestion (None), negative_unknown_memory_region_with_suggestion (Some sram), _unrelated_no_suggestion (None), negative_unknown_place_worker_with_suggestion (Some host), negative_unknown_accessible_by_unrelated_no_suggestion (None), suggestion_is_deterministic_across_repeated_lowering (16x repeated lowering byte-identical + lexicographic tie-break hosta<hostb pinned).

GATE (actual): just test 422 passed / 0 failed (was 415 in TASK-0096; +7 net new tests). e2e total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0. determinism-check byte-identical RUN TWICE (each cell "N file(s) byte-identical", 30/26/0/4) -> zero-behaviour-change-for-valid-input proof. determinism-check-negative bit (26/30 perturbed). xbackend-check-negative bit (13 corrupted, 1 detected). clippy --workspace --all-targets -- -D warnings: clean. just ci: exit 0.

Real-driver evidence (cargo run --bin nucleus build): typo `hostt` -> "references undeclared worker `hostt` -- did you mean `host`? at 3:24" (no panic, exit 1 typed); unrelated `zzzzzzzz` -> "references undeclared worker `zzzzzzzz` at 3:24" (NO hint, byte-identical to pre-0198, span suffix intact). Also verified UnknownMemoryRegion (sra->sram) and UnknownAccessibleByName (corx->corex, proving the worker_class∪workers union path).

Determinism guarantee: candidate sources are BTreeMap (ir.worker_classes/workers/memory_regions) -> deterministic key order; suggest() additionally sort_unstable()s + strict-< keeps lexicographically-first equal-distance; no HashMap/HashSet anywhere in the selection path. Same name+table => same suggestion (pinned by suggestion_is_deterministic_across_repeated_lowering, 16 iterations).

Review note: no qa-test-runner/mped-architect spawn tool exists in this environment (and per repo MEMORY.md spawned agents refuse code edits here); the full mechanical gate above IS the durable review surface and is green. Honest disclosure, not a skipped step.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no blocking findings, no follow-up. The corrected wrapper-shape forward-carry mechanic (from the TASK-0096 review, memorialized) was applied CORRECTLY and independently verified by both: suggestion added to the 4 SchedLowerErrorKind VARIANTS (inner #[derive(PartialEq,Eq)] enum -> folds into .kind value identity); the outer SchedLowerError{kind,span} wrapper + its hand-written impl PartialEq (.kind-only, span excluded) UNTOUCHED by the diff (3 hunks all inside SchedLowerErrorKind) -> TASK-0196 located-diagnostic contract intact (span still NOT in equality, suggestion IS). qa-test-runner: workspace 422/0; sched_lower 52/0; determinism byte-identical x2 + e2e 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; real-driver hostt->host hint WITH the `at L:C` span suffix preserved, unrelated name -> no hint byte-identical to pre-0198; helper reused (error.rs untouched); link/algo untouched. mped-architect: independently re-ran the gate (not trusted); determinism STRUCTURAL (3 BTreeMap tables, UnknownAccessibleByName = deterministic worker_classes.keys().chain(workers.keys()), no HashMap/HashSet, suggest reused verbatim); the 16x determinism test genuine (real tie-break host->hosta over hostb pinned); all 5 migrated negatives keep whole-.kind assert_eq! + add suggestion (no weakening), the 2 TASK-0196 span tests strengthened (matches! power kept + explicit suggestion pin); None-case genuinely tested per variant; doc de-staled accurately (the lower.rs "out of scope - that is TASK-0096" lie rewritten, no Damerau/always-suggests overclaim); candidate domain matches each variant validity rule (UnknownAccessibleByName union == the contains_key(name) check domain == TASK-0095 rule); decision-0003 upheld (no new panic). The fuzzy-match did-you-mean theme is now COMPLETE across both error types (TASK-0096 link + TASK-0198 sched). TASK-0198 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added deterministic did-you-mean suggestions to the four unknown-name SchedLowerError variants -- the schedule-side sibling of the Done TASK-0096 (link-step LinkError fuzzy-match).

What changed:
- The 4 SchedLowerErrorKind variants (UnknownWorkerClass/UnknownMemoryRegion/UnknownPlaceWorker/UnknownAccessibleByName) carry suggestion:Option<String>, the closest declared schedule-side symbol within a bounded edit distance, computed via the shared zero-dep crate::error::suggest helper (TASK-0096 -- reused verbatim, no duplicate edit-distance fn).
- Per-variant candidate sets, all BTreeMap key iterators: worker_classes / memory_regions / workers respectively; UnknownAccessibleByName uses the deterministic union worker_classes.keys().chain(workers.keys()), matching that variant own validity rule.
- Display: new write_suggestion helper (mirrors the link-step verbatim) appends ` -- did you mean `X`?` only on Some; byte-identical message on None.
- lower.rs doc-lie de-staled (no longer claims fuzzy-match is out of scope).

Wrapper-shape mechanic (corrected forward-carry): the field went on the SchedLowerErrorKind VARIANTS (inner enum, derived PartialEq) so it folds into .kind value identity; the outer SchedLowerError{kind,span} wrapper hand-written PartialEq (forwarding to .kind only, excluding span -- TASK-0196 contract) was deliberately left untouched. Net: suggestion IN value identity, span still OUT.

Why: typo-tolerant diagnostics; decision-0001 (tiny in-house fn, no new crate), decision-0003 (typed Result, no panic, zero behaviour change for valid input).

Tests: existing sched_lower negative tests migrated with assertion strength preserved (whole-.kind assert_eq! kept + suggestion added; the two TASK-0196 span tests keep matches! discriminating power via `..` and gain an explicit suggestion pin -- strengthened). New positive(Some)+unrelated(None) tests per variant + a 16x-repeat determinism test pinning the lexicographic tie-break.

Gate (measured): just test 422 passed / 0 failed (+7 vs TASK-0096 415). e2e 30/26/0/4/0. determinism-check byte-identical x2. determinism-check-negative + xbackend-check-negative both bite. clippy --workspace --all-targets -- -D warnings clean. ci exit 0. Real-driver: typo names surface the hint (no panic), unrelated names produce the byte-identical pre-0198 message with span suffix intact.

Risks/follow-ups: none. Determinism guaranteed by BTreeMap sources + suggest() sort + strict-< tie-break; no HashMap/HashSet in the path. No qa/architect spawn-agent tool exists in this environment -- the green mechanical gate is the review surface (honest disclosure).
<!-- SECTION:FINAL_SUMMARY:END -->
