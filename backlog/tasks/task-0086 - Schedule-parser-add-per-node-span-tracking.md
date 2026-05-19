---
id: TASK-0086
title: 'Schedule parser: add per-node span tracking'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:13'
updated_date: '2026-05-19 16:49'
labels:
  - M0
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0008 self-report follow-up. The schedule parser (nucleus/compiler/src/sched/parser.rs) currently tracks source position only on ParseError, not on AST nodes. Downstream semantic passes (TASK-0010 SchedIR lowering, TASK-0011 link step) will want spans on every directive/option for good diagnostics ('place undefined kernel X at line 42'). Add a Span = (usize, usize) byte-range field to each AST node and have the parser thread chumsky's span info through. Same follow-up exists for the algorithm parser (TASK-0007); coordinate so both parsers expose Span the same way.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Schedule AST nodes (Directive + its diagnosable sub-parts: directive node, option lists, place/memory_region/worker decls, and identifier/name fields) carry source spans via Spanned<T>{node,span:Range<usize>} (shared with / mirroring algo::span::Spanned); parse_sched (chumsky) populates TIGHT byte ranges (no trailing layout, via a padded_spanned primitive)
- [x] #2 Spanned<T> PartialEq/Eq/Hash/Debug delegate to inner node, span EXCLUDED, NOT derived (mirrors TASK-0082) — existing sched AST-equality tests stay valid without span boilerplate; Deref to T; serde-transparent
- [x] #3 A test asserts a representative parse's node spans point at the CORRECT source substring (validated via error::offset_to_line_col), tightness pinned
- [x] #4 Decision on sharing the Spanned wrapper (promote algo::span to a shared crate module reused by both vs sched-local duplicate) is made + documented
- [x] #5 Zero behaviour change: just test green, e2e 30/26/0/4/0, determinism byte-identical, clippy --workspace --all-targets clean, ci exit 0; SchedIR lowering still IGNORES spans (the SchedLowerError-located wiring is a SEPARATE 0090-analog task — scope NOT bled here)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
SHARE-VS-DUPLICATE (AC#4): PROMOTE algo/span.rs -> crate::span (compiler/src/span.rs). Single source of the load-bearing PartialEq-ignores-span semantics; no drift between algo & sched. Cost: re-point algo imports (algo/ast.rs, algo/parser.rs, algo/lower.rs, algo/mod.rs, algo_parser.rs test) mechanically; algo behaviour unchanged (only path moves) -> algo tests must stay green unchanged.

1. git mv algo/span.rs -> src/span.rs; add pub mod span to lib.rs; algo/mod.rs re-exports crate::span::Spanned (keep algo::span::Spanned path alias working for existing test import); update module docs (algo+generic). Verify algo tests green.
2. sched AST wrap granularity (AC#1): SpName=Spanned<String> for diagnosable idents -> WorkerClassDecl.name, MemoryRegionDecl.name, WorkerEntry.name, WorkerEntry.class, PlaceDirective.kernel, PlaceTarget::One/Many ident(s), PlaceDataDirective.data/.region, LoopDirective.var, TransferDirective.data, CheckDirective.var, accessible_by names. SpDirective=Spanned<Directive> in SchedAst.directives (the directive node a future error points at). Do NOT wrap Copy leaf enums (LoopOption/TransferOption/PartitionKind/NotifyKind/ViolationKind/TimeLit/SimdSpec/MemoryAtom) -> never independently diagnosed; matches algo judgement (no ScalarType/BinOp wrap).
3. parser.rs: ident()->Spanned via map_with_span; add padded_spanned primitive (mirror algo) for TIGHT spans (no trailing layout); wrap directive via padded_spanned at directive_parser; keep terminators bare before span-fix map.
4. lower.rs: mechanical .node plumbing (Deref for reads; &d.node for match; .name.node / clone of inner String). SchedIR semantics UNCHANGED. SchedLowerError UNTOUCHED (TASK-0196 owns wiring; lowering keeps IGNORING spans).
5. sched_parser.rs test: mechanical .node projections where Spanned<String> compared to &str (mirrors algo TASK-0082 26x .node edits) - strength/semantics preserved, NOT weakened. New AC#3 test: representative parse -> assert directive/ident spans reconstruct exact source substring + line:col via error::offset_to_line_col; pin tightness (no leading/trailing ws).
6. Gate: just test (existing sched + algo equality tests green unchanged + new span test 0 fail), e2e 30/26/0/4/0, determinism byte-identical x2 + both negatives bite, clippy --all-targets, ci exit 0.
7. Forward-carry note to TASK-0196 (Spanned path, granularity, which sched Spanned feeds which SchedLowerError site).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Sibling reference from TASK-0082 (DONE, commits 436c12a/3c158d3) — the algo-AST span pattern to mirror for the schedule AST: Spanned<T>{node,span:core::ops::Range<usize>} at compiler/src/algo/span.rs. KEY DESIGN POINTS to copy: (1) MANUAL impl PartialEq/Eq/Hash/Debug forwarding to .node ONLY, span EXCLUDED, NOT derived — this keeps existing AST-equality tests valid without touching expected literals; (2) Deref/DerefMut to T to bound consumer churn; (3) serde-transparent under the default-on feature; (4) SPAN TIGHTNESS: naive pad(p).map_with_span swallows trailing whitespace — use a padded_spanned primitive (map_with_span on the bare token, then then_ignore(comment_or_ws()) off-span) and end statements/decls at the BARE terminator before the span-fixing map_with_span. chumsky 0.9 span = Range<usize> for char input. Could share span.rs by promoting it (currently algo-local).

Forward-carried from TASK-0090 (DONE, commit 1c4e90a): the algo side now has the located-LowerError pattern the schedule side should MIRROR. Pattern: restructure SchedLowerError -> struct { kind: SchedLowerErrorKind, span: Option<Range<usize>> }; SchedLowerErrorKind = the existing enum verbatim (payloads unchanged, so no variant shape churn); PartialEq/Eq hand-forward to .kind only (span EXCLUDED, same rationale as Spanned/TASK-0082) so existing sched_lower negative tests migrate mechanically (Err(SchedLowerError::X(..)) -> Err(SchedLowerError { kind: SchedLowerErrorKind::X(..), .. })); add display_with_src(&self,src)->String converting span.start via compiler::error::offset_to_line_col; driver renders "schedule lower error: <msg> at L:C". Populate span at each diagnosable err site from the offending sched Spanned once TASK-0086 adds the sched-AST span substrate. Genuinely multi-site/synthetic variants stay span:None (documented). Gate: determinism stays byte-identical because spans populate only on the Err path.

IMPLEMENTED (commits 1f3bdc8 promote, 5ca11a7 sched substrate).

SHARE-VS-DUPLICATE (AC#4): PROMOTED algo::span -> shared crate::span (compiler/src/span.rs via git mv). Rationale: the span-EXCLUDED PartialEq/Eq/Hash/Debug semantics are load-bearing for EVERY AST-equality test in BOTH sublanguages; two copies could silently drift (one derives Hash, other does not) -> invisible correctness bug. One impl = one audit point. Duplication rejected as weak default (no sublanguage-specific behaviour). Cost was purely mechanical: algo ast/parser/ir/mod re-point import path; algo::span kept as thin re-export of crate::span::Spanned so the move is NON-BREAKING for tests importing compiler::algo::span::Spanned. ALGO REGRESSION: algo_parser 43/43 + algo_lower 22/22 green UNCHANGED post-promotion (TASK-0082 behaviour preserved, path-move only). Zero new rustdoc broken links (pristine tree = 11 warnings, post-change = 11, identical).

WRAP GRANULARITY (AC#1): SpName=Spanned<String> on the ident/name fields a SchedLowerError points at (WorkerClassDecl.name, MemoryRegionDecl.name + accessible_by names, WorkerEntry.name/.class, PlaceDirective.kernel, PlaceTarget::One/Many worker names, PlaceDataDirective.data/.region, LoopDirective.var, TransferDirective.data, CheckDirective.var); SpDirective=Spanned<Directive> on every SchedAst.directives entry. NOT wrapped: LoopOption/TransferOption/CheckAssert/SimdSpec/MemoryAtom/MemorySpec/TimeLit/PartitionKind/NotifyKind/ViolationKind and the normalised u64/bool literals — never independently diagnosed (a bad block=0 is reported against the loop var; conflicting mode against the transfer data name). Mirrors algo not wrapping BinOp/ScalarType. SimdSpec::Named/MemoryAtom::Named kept bare String (backend-interpreted / not name-resolved) — parser drops their ident span via .node. Argued in crate::span module docs.

CHUMSKY SPAN TIGHTNESS: ident() does .map_with_span(Spanned::new) on the BARE token (no surrounding layout). Added padded_spanned primitive (mirror of algo). Every directive parser changed to end at the BARE ; terminator (was then_ignore(pad(just(';')))); directive_parser wraps via padded_spanned so SpDirective span = [keyword..';'] exactly, trailing layout consumed OFF-span. program_parser keeps .padded_by(comment_or_ws()).repeated() (leading layout eaten before map_with_span) — same as algo.

SERDE: inherited transparent impls from shared wrapper (no sched AST type is serde-derived today; no behaviour change).

CONSUMERS NEEDING .node PLUMBING: only sched/lower.rs (mechanical: match &d.node since Deref does not apply to match; .name.node clone into plain-String IR; accessible_by Vec<SpName> stripped to Vec<String>). SchedLowerError + SchedIR shapes UNTOUCHED. No other crate consumer (link.rs uses SchedIR not SchedAst; driver uses parse_sched result opaquely). Only sched_parser.rs test needed .node projections on field-vs-&str comparisons (mechanical, strength preserved — SAME precedent as algo TASK-0082 which made 26 such edits; whole-AST assert_eq! untouched, proving PartialEq ignores span). All other test files insulated by Deref + lowering producing plain types.

GATE (ACTUAL): just test 0 failed across all crates (sched_parser 20->23 incl new span test, sched_lower 15, algo_parser 43, algo_lower 22 — existing equality tests green UNCHANGED). e2e total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0. determinism-check byte-identical 30/26/0/4 RUN TWICE. determinism-check-negative bit (26/30 perturbed). xbackend-check-negative bit (13 corrupted, 1 detected). clippy --workspace --all-targets clean (no derived_hash_with_manual_eq — Spanned manual-impls PartialEq AND Hash, neither derived). just ci exit 0.

AC#3 EVIDENCE (spans_point_at_correct_source_substring, sched_parser.rs): asserts &src[span]==exact substring AND offset_to_line_col(src,span.start)==(line,col) AND tightness (no leading/trailing whitespace) for: directive0 worker_class span "worker_class cc { simd = none; };" (2,1) + wc.name "cc" (2,14); memory_region directive (3,1) + name "rgn" (3,15) + accessible_by[0] "cc" (3,39) + [1] "w0" (3,43); workers directive (4,1) + entry name "w0" (4,13) + class "cc" (4,18); place directive (5,1) + kernel "k" (5,7) + target "w0" (5,12); place_data (6,1) + data "d" (6,12) + region "rgn" (6,17); loop (7,1) + var "i" (7,6). PASSES.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no blocking findings, no follow-up needed. CORRECTED COUNTS (reviewer-measured is the fact of record — implementer self-report transposed the labels): workspace 403 passed / 0 failed / 2 ignored; actual per-binary algo_parser=15, algo_lower=20, sched_parser=23 (incl new span test), sched_lower=43 — ALL 0-failed/green (substance holds; only the labels were swapped). qa-test-runner: promotion regression control PASS — git show of tests/ EMPTY (zero algo test logic changed), algo_parser/algo_lower green UNCHANGED (TASK-0082 preserved); sched_lower.rs diff EMPTY (whole-AST equality span-insensitive), sched_parser.rs changes mechanical .node projections only (no weakened assertion) + 1 new span test; AC#3 test reconstructs &src[span] byte-exact + line:col + tightness across 9 node kinds; determinism byte-identical x2 + e2e 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; serde --all-features builds; sched/ir.rs UNTOUCHED (TASK-0196 not bled). mped-architect: shared crate::span Spanned manual PartialEq/Eq/Hash/Debug all forward to .node only (NOT derived — no derived_hash_with_manual_eq; Eq/Hash consistent by construction); algo::span is a thin re-export (import-path-only, zero algo semantic change); built the full SchedLowerError 20-variant -> span-reachability matrix: EVERY diagnosable variant offending ident reachable via the wrapped set, MissingWorkersDecl correctly span:None (absence), no under/over-wrap; tightness applied UNIFORMLY across all 8 directive inner parsers (bare-terminator before span-fixing wrap); ALL span/granularity docs EXACTLY match code — the recurring comment/doc-lie class that bit TASK-0090 is NOT repeated (learned-from-0090 verified, incl. the honest "existing tests pass unchanged means whole-AST structural equality" nuance, not overclaimed); no new panic (decision-0003); forward-carry to TASK-0196 exceptional + technically correct (per-variant wiring map, the two post-strip sites UnknownWorkerClass/UnknownAccessibleByName correctly flagged, option (b) AST-walk recommendation matches TASK-0090). TASK-0086 Done stands — the sched-side span substrate is correct, bounded, honest; TASK-0196 (dep TASK-0086) is the located-wiring home.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added per-node source-span tracking to the schedule AST + parser (sched analog of TASK-0082), substrate only.

What changed:
- Promoted the Spanned<T> wrapper from algo-local algo::span to a shared crate::span module (AC#4: share, not duplicate — one impl of the load-bearing span-EXCLUDED PartialEq/Eq/Hash/Debug semantics; algo::span kept as non-breaking re-export; algo tests green unchanged).
- Schedule AST: SpName=Spanned<String> on the diagnosable ident/name fields a SchedLowerError points at; SpDirective=Spanned<Directive> on every directives entry. Copy/leaf option enums deliberately not wrapped (never independently diagnosed) — argued in crate::span docs.
- parse_sched: ident() captures the bare-token span; new padded_spanned primitive + bare-; terminators give TIGHT directive spans (no trailing layout).
- sched/lower.rs: mechanical .node/Deref plumbing only; SchedIR + SchedLowerError shapes UNTOUCHED; lowering still IGNORES spans (TASK-0196 owns the located-error wiring).
- New AC#3 test pins directive + identifier spans to exact source substrings and line:col via error::offset_to_line_col, asserting tightness.

User impact: none yet — additive metadata; behaviour byte-identical. Enables TASK-0196 to surface "schedule lower error: <msg> at L:C".

Tests: just test 0 failed (sched_parser 23, sched_lower 15, algo_parser 43, algo_lower 22 — existing equality tests UNCHANGED & green, proving PartialEq ignores span); e2e 30/26/0/4/0; determinism byte-identical x2; both negative gates bite; clippy --workspace --all-targets clean; just ci exit 0.

Commits: 1f3bdc8 (promote Spanned), 5ca11a7 (sched substrate + parser + test).

Risks/follow-ups: TASK-0196 (filed, dep TASK-0086) threads these spans into SchedLowerError. The .node test-projection churn (field-vs-&str comparisons) is the same bounded precedent as algo TASK-0082; whole-AST assert_eq! is unaffected.
<!-- SECTION:FINAL_SUMMARY:END -->
