---
id: TASK-0198
title: >-
  SchedLowerError unknown-name fuzzy-match did-you-mean (sched sibling of
  TASK-0096)
status: To Do
assignee: []
created_date: '2026-05-19 17:18'
updated_date: '2026-05-19 17:39'
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
- [ ] #1 SchedLowerError unknown-name variants (UnknownAccessibleByName, UnknownPlaceWorker, UnknownWorkerClass, UnknownMemoryRegion) carry an Option<String> did-you-mean suggestion computed via the shared TASK-0096 edit-distance helper against the in-hand schedule symbol table; deterministic tie-breaking
- [ ] #2 Negative tests assert the suggestion when one is computable and None when not (e.g. no close candidate); existing SchedLowerError negative tests migrate with assertion strength preserved
- [ ] #3 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); typed-Result, no panic; zero behaviour change for valid input
<!-- AC:END -->

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
<!-- SECTION:NOTES:END -->
