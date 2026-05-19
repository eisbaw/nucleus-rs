---
id: TASK-0096
title: 'Link step: fuzzy-match suggestions for unknown name errors'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:42'
updated_date: '2026-05-19 17:39'
labels:
  - compiler
  - link
  - diagnostics
  - M0-followup
dependencies:
  - TASK-0095
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
When the link step emits UnknownKernel / UnknownData / UnknownLoop / UnknownTransferData, it currently reports the offending name verbatim. For typo-class user mistakes, a 'did you mean X?' hint (Levenshtein distance against the algorithm's symbol table) would materially help. The link step has both symbol tables in hand. Acceptance: a single LinkError variant carries an Option<String> suggestion; the test for negative cases asserts the suggestion when one is computable.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The 4 link-step unknown-name LinkError variants (UnknownKernel/UnknownData/UnknownLoop/UnknownTransferData) carry an Option<String> did-you-mean suggestion computed via a deterministic zero-dep in-house edit-distance helper against the in-hand symbol table (consistent with decision-0001 no-extra-deps ethos)
- [x] #2 The suggestion is DETERMINISTIC (same name+table -> same suggestion; deterministic tie-break, e.g. lexicographically-first among equal-distance) and only offered within a sensible bounded edit-distance threshold; PartialEq treatment of the suggestion field decided + documented
- [x] #3 Negative tests assert the suggestion is Some(expected) for a typo-class name and None when no close candidate exists; existing LinkError-asserting tests migrate with assertion strength PRESERVED
- [x] #4 Stays typed-Result, no panic (decision-0003); SCOPE: link-step LinkError only (SchedLowerError fuzzy-match is the sibling TASK-0198 — do not bleed); spans/locatedness NOT in scope (link consumes span-free AlgoIR; 0096 is the suggestion only)
- [x] #5 Zero behaviour change for valid input: just test green, e2e 30/26/0/4/0, determinism byte-identical, clippy --workspace --all-targets clean, ci exit 0 (suggestions only on the Err path)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add zero-dep edit-distance helper `crate::error::levenshtein(a,b)->usize` + `suggest(name, candidates)->Option<String>` in error.rs (reuse-friendly home; sibling TASK-0198 sched/lower.rs can `use crate::error::suggest`). Standard O(n*m) two-row DP, char-wise (ASCII grammars). Plain Levenshtein (not Damerau): the 4 link tables are short identifier sets; transposition is a rare typo vs insertion/deletion/substitution; +1 for transposition is acceptable and keeps the helper minimal per decision-0001.
2. suggest() policy: deterministic. Iterate candidates in sorted order (callers pass BTreeMap.keys()/BTreeSet -> already sorted; suggest() also sorts defensively into a Vec<&str> via sort()). Pick min distance; threshold = distance <= max(1, name.chars().count()/3). Tie-break: first in sorted order = lexicographically-first. No HashMap/HashSet in selection path.
3. Unit-test helper: levenshtein known pairs (eq=0, 1 sub, 1 ins, 1 del, transposition=2, empty cases); suggest() typo->Some, far->None, tie->lexicographically-first.
4. LinkError shape: widen the 4 unknown-name variants to struct form `UnknownKernel { name: String, suggestion: Option<String> }` (+ Data/Loop/TransferData). Mirrors nothing-extra; simpler than a wrapper since only 4 of 6 variants get it and UnplacedKernel/MissingCrossWorkerTransfer are unaffected. DERIVE PartialEq/Eq (do NOT hand-write to exclude suggestion): unlike LowerError.span (informational human position), the suggestion is a deterministic pure fn of (name, in-hand table) -> equal errors necessarily have equal suggestions, so it is legitimately part of value identity (AC#2 documents this divergence from TASK-0090 explicitly).
5. Emit sites in link(): compute suggestion = error::suggest(&name, algo.kernels.keys()) etc. UnknownKernel<-algo.kernels keys; UnknownData & UnknownTransferData<-algo.data keys; UnknownLoop<-loop_vars (BTreeSet). Suggestions only on Err path (zero behaviour change for valid input).
6. Display: append ` -- did you mean `X`?` when Some, unchanged when None.
7. De-stale link.rs module doc ~42-43 (no longer "no fuzzy-match ... filed as follow-up"; describe new behaviour) + LinkError type doc. Apply TASK-0090 doc-lie lesson: doc matches code first time.
8. Migrate the 6 LinkError::Unknown* negative-test assertions (lines ~346,398,420,443,464 + multi-error ~582,583) to struct form, PRESERVING all discriminating power and ADDING suggestion assertion. New test: typo unknown name -> Some(expected); unrelated -> None.
9. Full gate inside nix develop: just test / e2e 30/26/0/4/0 / determinism-check x2 byte-identical / determinism-check-negative + xbackend-check-negative still bite / clippy --all-targets / ci. Real-driver crafted-schedule did-you-mean evidence.
10. Forward-carry to TASK-0198: helper location/signature, threshold, tie-break, derived-PartialEq precedent.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0095: SchedLowerError now also has unknown-name diagnostics that are candidates for the same did-you-mean treatment — UnknownAccessibleByName{region,name} (added in TASK-0095), plus the pre-existing UnknownPlaceWorker / UnknownWorkerClass / UnknownMemoryRegion. The schedule symbol tables (ir.worker_classes, ir.workers, ir.memory_regions) are all in hand at the SchedIR lowering site (nucleus/compiler/src/sched/lower.rs), exactly as the link step has its tables. TASK-0095 deliberately kept its error a plain "undeclared name `X`" (no fuzziness) to stay in scope; this task is where the Levenshtein suggestion should be added — consider widening scope to cover SchedLowerError unknown-name variants too, or filing a sibling task. UnknownAccessibleByName is the natural first hook.

Forward-carried from TASK-0082 (DONE): algo AST nodes now carry TIGHT byte spans via Spanned<T> (compiler/src/algo/span.rs). For fuzzy-match unknown-name suggestions, the offending identifier is a Spanned<String> (Call.callee / IndexedLValue.name / Stmt::For.var / decl .name) — .span is the exact ident token range; feed .span.start to error::offset_to_line_col for (line,col). NOTE: link step consumes the IR (AlgoIR), not the algo AST — spans are NOT yet on IR. TASK-0090 propagates spans algo-AST -> LowerError; whether they reach the link step depends on TASK-0090/IR design. Spanned PartialEq ignores span (forward .node).

Forward-carried from TASK-0090 (DONE, commit 1c4e90a): link-step unknown-name errors can now follow the same located-diagnostic shape — a typed Kind + Option<Range<usize>> byte span, PartialEq forwarding to kind only, driver-side display_with_src via compiler::error::offset_to_line_col. For fuzzy-match suggestions: the offending identifier reference is the Spanned<String> (SpIdent) whose .span TASK-0090 threads into UnknownIdent/IterVarOutOfScope/AssignmentTargetNotData — a "did you mean `X`?" suggestion should underline that same span. The SpIdent substrate (TASK-0082) is what carries the candidate name + its source range; reuse it rather than re-deriving offsets.

IMPLEMENTED (TASK-0096).

Helper: crate::error::levenshtein(a,b)->usize and crate::error::suggest(name, candidates: IntoIterator<Item=&str>)->Option<String> in nucleus/compiler/src/error.rs. Plain Levenshtein (NOT Damerau — transposition costs 2; argued in doc: short id tables, transposition rare vs the 3 primitive edits, decision-0001 minimality). O(n*m) two-row rolling DP, char-wise (unicode-safe; grammars ASCII). Home is error.rs (pub mod) so sibling TASK-0198 reuses it from sched/lower.rs without duplication.

Threshold: distance <= max(1, name.chars().count()/3). Tie-break: candidates collected + sort_unstable()ed, strict `<` keeps the lexicographically-first among equal-distance. NO HashMap/HashSet in selection path (callers also pass BTreeMap.keys()/BTreeSet — doubly deterministic). suggest() determinism proven by unit test suggest_tie_break_is_lexicographically_first (same multiset reversed -> identical result).

LinkError shape: per-variant struct widening — UnknownKernel/UnknownData/UnknownLoop/UnknownTransferData are now { name, suggestion: Option<String> }. UnplacedKernel/MissingCrossWorkerTransfer untouched (no unknown name). Chose per-variant struct over a {kind,suggestion} wrapper because only 4/6 variants gain the field (TASK-0090s wrapper was justified by a UNIFORM span on every variant — not the case here).

PartialEq DECISION: DERIVED (suggestion IS part of value identity) — deliberately OPPOSITE to TASK-0090 LowerError which hand-excludes span. Rationale: a span is informational human position (where source sat); a suggestion is a deterministic pure fn of (name, in-hand table), so equal-name errors against the same table necessarily have equal suggestions — folding it into Eq cannot spuriously split. Documented in the LinkError type doc.

Display: appends ` -- did you mean `X`?` when Some via write_suggestion(); byte-identical to pre-0096 when None.

link.rs module doc ~42 DE-STALED: the "No fuzzy-match ... Filed as follow-up" bullet replaced with an accurate description of the new behaviour (TASK-0090 doc-lie lesson applied — doc matches code first time; comment-honesty defect class avoided).

Link test churn (assertion strength PRESERVED + suggestion added): negative_unknown_kernel (assert! contains, +suggestion:None for unrelated `bar`), negative_unknown_data/unknown_loop/unknown_loop_via_check/unknown_transfer_data (assert_eq! whole-Vec strength kept, +suggestion:None), multi_error_one_pass (assert! contains, +suggestion:None). NEW tests: negative_unknown_kernel_with_suggestion (fooo->Some(foo), unrelated barbaz no hint, Display carries hint), negative_unknown_data_with_suggestion (weight->Some(weights)), negative_unknown_loop_with_suggestion (j->Some(i)). Plus 7 error::fuzzy_tests unit tests (levenshtein known pairs incl transposition=2 + empty + unicode; suggest typo/unrelated/empty/tie-break/bound-scaling).

GATE (all inside nix develop): just test 415 passed / 0 failed (link suite 27->31; +6 fuzzy unit). just e2e 30/26/0/4/0. clippy --workspace --all-targets -D warnings clean. determinism-check x2 byte-identical 30/26/0/4. determinism-check-negative 26 perturbed (bites). xbackend-check-negative 13 corrupted/1 detected (bites). just ci EXIT 0.

Real-driver evidence (nucleus build, crafted typo schedule on 01-elementwise-add, exit 1, typed error no panic):
  - schedule places kernel `add_` ... -- did you mean `add`?
  - schedule places kernel `load_inpu` ... -- did you mean `load_input`?
`load_inpu`->`load_input` (d=1) NOT `load_input_b` (d=3): closest-within-bound + deterministic. UnplacedKernel unchanged (no hint).

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO. qa-test-runner: workspace 415/0; all 7 fuzzy_tests + 3 new link suggestion tests + migrated negatives green by name; NO masked weakening (every migrated negative kept whole-Vec assert_eq!/contains strength + ADDED suggestion assertion — per-test table verified); suggestion PROVABLY deterministic (algo.kernels/data BTreeMap, collect_loop_vars BTreeSet, suggest sort_unstable+strict-< lexicographic, ZERO HashMap/HashSet in selection path; determinism-check byte-identical x2; reversed-multiset tie-break unit test); e2e 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; real-driver did-you-mean correct incl. closest-within-bound (load_inpu->load_input d=1 not load_input_b d=3) + None for unrelated, no panic; scope fenced (sched/ untouched, no span threaded). mped-architect: derived-PartialEq decision SOUND + explicitly documented as a deliberate divergence from TASK-0090 (suggestion is a pure deterministic fn of (name,table) so cannot spuriously split eq; span was positional-noise so 0090 correctly excluded it); determinism STRUCTURAL; zero-dep in-house Levenshtein (decision-0001) correctly unit-tested (kitten/sitting=3, sub/ins/del/transposition/empty/unicode); plain-Levenshtein-transposition=2 a defensibly-documented conservative-fail trade-off (never mis-ranks, only yields None on short-name transpositions); link.rs module doc de-staled accurately (TASK-0090 doc-lie lesson applied, no overclaim, Display byte-identical on the None path = AC#5). The ONE finding: the forward-carry note TASK-0096 wrote into TASK-0198 carried a PartialEq mechanic correct for LinkError(bare enum) but WRONG for SchedLowerError({kind,span} wrapper, TASK-0196) — shipped 0096 code is correct, defect was a misleading lesson-feed-forward in not-yet-started TASK-0198. ORCHESTRATOR CORRECTED IT IN-THREAD (appended the right wrapper-shape mechanic to TASK-0198). TASK-0096 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added deterministic "did you mean `X`?" fuzzy-match suggestions to the link step's four unknown-name LinkError variants.

What changed:
- New zero-dependency in-house helper in nucleus/compiler/src/error.rs: `levenshtein(a,b)` (plain Levenshtein, char-wise, O(n*m) two-row DP — transposition costs 2, NOT Damerau, argued for decision-0001 minimality) and `suggest(name, candidates)` (closest within bound max(1, name.chars().count()/3); deterministic lexicographically-first tie-break; candidates sorted before selection; no HashMap/HashSet in the selection path). Lives in error.rs (pub mod) so sibling TASK-0198 reuses it from sched/lower.rs without duplication.
- LinkError::UnknownKernel/UnknownData/UnknownLoop/UnknownTransferData widened from tuple to `{ name, suggestion: Option<String> }`. UnplacedKernel/MissingCrossWorkerTransfer untouched. Per-variant struct (not a {kind,suggestion} wrapper) because only 4/6 variants gain the field.
- PartialEq/Eq DERIVED: the suggestion IS part of value identity — deliberately opposite to TASK-0090 LowerError (hand-excludes span). A suggestion is a deterministic pure fn of (name, in-hand table), so equal-name errors against the same table necessarily have an equal suggestion; documented on the LinkError type doc.
- Display appends ` -- did you mean `X`?` when Some (byte-identical to before when None).
- link.rs module doc de-staled: the "No fuzzy-match ... Filed as follow-up" bullet replaced with an accurate description (TASK-0090 doc-lie lesson applied — doc matches code).
- Negative tests migrated with assertion strength preserved (whole-Vec assert_eq! kept) + suggestion asserted; 3 new positive-suggestion tests; 7 new error::fuzzy_tests unit tests (incl. transposition=2, empty, unicode, tie-break determinism, bound scaling).

User impact: typo'd kernel/data/loop/transfer names in a schedule now get an actionable hint; suggestions only computed on the Err path so valid input is byte-identical.

Tests/gate (all green, inside nix develop): just test 415 passed/0 failed (link suite 27->31, +6 fuzzy unit). just e2e 30/26/0/4/0. clippy --workspace --all-targets -D warnings clean. determinism-check x2 byte-identical 30/26/0/4. determinism-check-negative (26 perturbed) + xbackend-check-negative (13 corrupted/1 detected) still bite. just ci EXIT 0. Real-driver evidence on a crafted typo schedule (01-elementwise-add): `add_`->did you mean `add`?, `load_inpu`->did you mean `load_input`? (NOT load_input_b — closest-within-bound + deterministic), exit 1, typed error, no panic.

Commit: 6d39609. Forward-carry filed to TASK-0198 (helper signature/location, threshold, tie-break, struct-widening + derived-PartialEq precedent).

Risks/follow-ups: none new. Scope held — SchedLowerError fuzzy-match is the separate already-filed TASK-0198 (dep TASK-0096); spans not threaded into LinkError (out of scope; link consumes span-free IR).
<!-- SECTION:FINAL_SUMMARY:END -->
