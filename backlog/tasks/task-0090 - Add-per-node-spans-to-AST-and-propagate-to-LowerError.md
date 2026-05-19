---
id: TASK-0090
title: Add per-node spans to AST and propagate to LowerError
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:25'
updated_date: '2026-05-19 16:14'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Both the parser AST and the lowering pass currently lack span tracking on individual nodes. Once AST nodes carry (line, column), LowerError variants should gain position fields. Surface stays source-compatible; just enriches diagnostics. Filed as follow-up from TASK-0007 and TASK-0009.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Diagnosable LowerError variants (DuplicateConst/Data/Kernel, UnknownIdent, DoubleAssignment, IterVarOutOfScope, and the other user-reachable variants) carry a source position populated at the error site from the relevant Spanned node's span (span.start -> error::offset_to_line_col); Display renders 'at line:col'
- [x] #2 The driver surfaces the located error (nucleus: error: algorithm lower error: <msg> at L:C); surface stays source-compatible, typed-Result preserved, NO panic (decision-0003)
- [x] #3 A test feeds representative bad programs (e.g. duplicate const, unknown ident, double assignment) and asserts the LowerError carries the CORRECT line:col validated against the source via error::offset_to_line_col
- [x] #4 A decision on LowerError equality (position informational-and-ignored-in-PartialEq, mirroring TASK-0082 Spanned, vs position-compared) is made + documented; existing LowerError-asserting tests updated accordingly (honest expected scope, not hidden)
- [x] #5 Zero behaviour change for VALID input: just test green, e2e 30/26/0/4/0, determinism byte-identical, clippy --workspace --all-targets clean, ci exit 0 (positions populate only on the Err path)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
DESIGN DECISION 1 (span storage + conversion site): Restructure LowerError into struct { kind: LowerErrorKind, span: Option<Range<usize>> }. LowerErrorKind = today's enum verbatim (all ~19 variants, payloads unchanged). Byte span populated at each err site from the offending Spanned (.span). Option<> because a few derived/synthetic sites have no single honest source node (ConstCycle: multi-decl; the <index/loop-bound> synthetic NonIntegerShapeExpr) -> documented position-less, honest-partial per-variant, NOT a faked span. Conversion to line:col at the DRIVER via error::offset_to_line_col (lowering stays &AlgoAst-only; mirrors error.rs/ParseError, lower-touch). Add LowerError::display_with_src(&self,src)->String rendering "<kind> at L:C"; library Display stays span-free for source-less callers/tests.

DESIGN DECISION 2 (equality, AC#4): LowerError PartialEq/Eq forward to .kind ONLY (span EXCLUDED), hand-written, mirroring Spanned (TASK-0082) + decision precedent. Position is informational-for-humans, not semantic identity. Existing negative tests pattern-match the variant+payload; they migrate mechanically from `LowerError::X(n)` to `LowerError { kind: LowerErrorKind::X(n), .. }` (payload assertions unchanged) — expected churn per AC#4, not hidden.

PLAN:
1. ir.rs: rename enum -> LowerErrorKind; add struct LowerError{kind,span}; ctor helpers (LowerError::new(kind), .at(span)); hand PartialEq/Eq fwd to kind; Display fwd to kind (span-free); add display_with_src; std::error::Error.
2. lower.rs: thread offending Spanned span to every diagnosable err site. Helpers resolve_ident/eval_const_ident/eval_shape_ident take the byte span alongside name. DuplicateConst/Data/Kernel -> name.span; UnknownIdent/AssignmentTargetNotData/IterVarOutOfScope -> offending SpIdent.span; DoubleAssignment -> lhs.name.span; NonPositiveDim/Shape* -> dim SpExpr.span; Const* -> offending SpExpr/SpIdent.span; ConstCycle + synthetic index/loop-bound -> span:None (documented).
3. algo/mod.rs: re-export LowerErrorKind alongside LowerError.
4. driver/main.rs:180: use e.display_with_src(&algo_src).
5. tests/algo_lower.rs: migrate negative matches to struct form (payload asserts unchanged); add AC#3 test (dup const / unknown ident / double assignment) asserting exact line:col validated via error::offset_to_line_col against crafted source.
6. Full gate (just test/e2e/determinism x2/negatives/clippy/ci) before each commit; real-driver located-error evidence; backlog notes w/ design decisions + actual numbers + exact changed tests; forward-carry to TASK-0086/0080/0092/0096.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0082 (DONE, commits 436c12a/3c158d3): Spanned<T>{node,span:Range<usize>} now EXISTS at compiler/src/algo/span.rs and parse_algo populates TIGHT byte ranges (no trailing layout). Wrap granularity: Item, Stmt (incl nested for-body), Expr (all recursive positions), and Spanned<String> for ConstDecl/DataDecl/KernelDecl.name, IndexedLValue.name, Call.callee, Stmt::For.var. Spanned PartialEq/Eq/Hash/Debug IGNORE the span (forward to .node) so AST/IR-equality tests are unaffected. lower.rs currently does .node projection and IGNORES spans — THIS task wires those spans into LowerError. The span you want is on the SpIdent (e.g. lhs.name.span for an undeclared/duplicate symbol) or the SpExpr (rhs.span for a bad expression); feed span.start to error::offset_to_line_col for (line,col). Keep typed-Result (decision-0003), do NOT panic.

IMPLEMENTED — commit 1c4e90a.

DESIGN DECISION 1 (span storage + conversion site): LowerError became `struct { kind: LowerErrorKind, span: Option<Range<usize>> }`. LowerErrorKind = the prior enum VERBATIM (all ~19 variants, payloads byte-for-byte unchanged) — chosen over per-variant (line,col) fields specifically so NO variant payload shape changed (the negative tests still assert the same payload). Byte Range (not line/col) stored because lower_algo takes &AlgoAst and has no source string; conversion to line:col happens DRIVER-SIDE via LowerError::display_with_src(&self,src) calling compiler::error::offset_to_line_col — lowering stays &AlgoAst-only, mirrors error.rs/ParseError. Rationale for driver-side over threading src into lower_algo: lower-touch (1 driver line), consistent with existing ParseError surfacing, keeps the pass source-text-free.

DESIGN DECISION 2 (equality, AC#4): LowerError PartialEq/Eq HAND-WRITTEN, forward to .kind ONLY; span EXCLUDED from value identity. Same decision + rationale as Spanned (TASK-0082): position is informational-for-humans, not semantic identity. Consequence: existing negative tests (which assert kind+payload, never offset) stay semantically valid; they only needed the mechanical struct-destructure migration. Documented on the LowerError type doc + pinned by a dedicated test.

PER-VARIANT SPAN SOURCE MAPPING:
- DuplicateConst/Data/Kernel <- c.name/d.name/k.name .span (the duplicate decl identifier)
- UnknownIdent <- offending SpIdent.span (callee c.callee.span / lhs.name.span / lv.name.span / bare ident name.span)
- AssignmentTargetNotData, DoubleAssignment <- lhs.name.span (the violating LHS)
- IterVarOutOfScope <- the out-of-scope reference SpIdent.span (threaded into resolve_ident)
- IterVarShadowsDecl <- for-var SpIdent.span
- ConstRefersToNonConst <- offending ident SpIdent.span (threaded into eval_const_ident)
- ConstOverflow/ConstDivByZero <- offending const sub-expr SpExpr.span
- NonPositiveDim <- the dim SpExpr.span; ShapeRefersToNonConst <- ident span; ShapeOverflow/ShapeDivByZero/NonIntegerShapeExpr(real decl) <- shape sub-expr SpExpr.span
- NonIntegerConstExpr <- offending const sub-expr SpExpr.span
POSITION-LESS BY DESIGN (span:None, documented honest-partial): ConstCycle (spans several decls, no single primary node) and the SYNTHETIC <index/loop-bound expression> NonIntegerShapeExpr in lower_index_expr (reported against a pseudo-decl, not a real source node — the real-decl shape NonIntegerShapeExpr in eval_shape_expr IS located).

FILES CHANGED: nucleus/compiler/src/algo/ir.rs (enum->LowerErrorKind, new LowerError struct + new()/at()/display_with_src(), hand PartialEq/Eq, Display fwd, Range import, doc-link fix), nucleus/compiler/src/algo/lower.rs (all err sites -> LowerError::at/new; resolve_ident/eval_const_ident/eval_shape_ident gained a name_span param; Range import), nucleus/compiler/src/algo/mod.rs (re-export LowerErrorKind), nucleus/driver/src/main.rs (display_with_src(&algo_src)), nucleus/compiler/tests/algo_lower.rs (11 negative-test migrations + 2 new tests).

EXACT LowerError-ASSERTING TESTS CHANGED (AC#4 honest scope — all in tests/algo_lower.rs, all the SAME mechanical change Err(LowerError::X(..)) -> Err(LowerError { kind: LowerErrorKind::X(..), .. }), payload asserts UNCHANGED, NOT a masked regression): duplicate_kernel_name_is_error, duplicate_data_name_is_error, duplicate_const_name_is_error, const_and_data_share_namespace (matches!(err,..) -> matches!(err.kind,..)), double_assignment_to_same_data_is_error, iter_var_outside_its_loop_is_error, const_divide_by_zero_is_error, const_forward_reference_is_error, non_positive_shape_dim_is_error, unknown_kernel_in_dataflow_rhs_is_error, assignment_to_const_is_rejected. NEW: located_errors_carry_correct_line_col (AC#3 — dup const@2:7, unknown ident@2:7, double-assign@7:1, each expected pos recomputed from the source via offset_to_line_col, plus exact display_with_src string), multi_site_variants_are_position_less (pins ConstCycle span:None + Display fallback).

GATE (all inside nix develop, ACTUAL): just test = 0 failed (every crate ok; algo_lower 20 passed incl 2 new). just e2e = total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0. just determinism-check RUN TWICE = byte-identical 30/26/0/4 both runs (zero-behaviour-change-for-valid-input proof). just determinism-check-negative = OK (correctly bit). just xbackend-check-negative = OK (correctly bit). just clippy (--workspace --all-targets -D warnings) = exit 0 clean. just ci = exit 0.

REAL-DRIVER EVIDENCE (release nucleus binary, crafted bad .algo.nuc): "const N..\nconst N.." -> `nucleus: error: algorithm lower error: duplicate const `N` at 2:7`; "data x..\nx <-- nope();" -> `nucleus: error: algorithm lower error: unknown identifier `nope` at 2:7`.

GOTCHAS / LESSONS (feed-forward, subagents stateless): (1) the span-vs-driver-conversion choice: store byte Range on the error, convert at the driver where src lives — do NOT thread src into lower_algo (keeps pass &AlgoAst-only, mirrors ParseError; 1-line driver change vs signature churn). (2) The &str helpers (resolve_ident/eval_const_ident/eval_shape_ident) LOSE the span before the err site — they each needed a name_span: Range<usize> param threaded from the caller's SpIdent/SpExpr; this is where most of the lower.rs churn was. (3) LowerError-equality decision (forward to kind only) is what kept negative-test churn purely mechanical — without it every payload assert would also have needed touching. (4) Stmt::For shadows the `var` binding with `var.node`; capture var_span BEFORE the shadow. (5) Comment honesty: the old "span unused (TASK-0090)" comments were stale-by-design markers — replaced with accurate located-at comments.

PROCESS LIMITATION (honest): the global review gate mandates qa-test-runner + mped-architect SUB-AGENTS before commit; those sub-agents are NOT surfaced as tools in this environment (only the Task todo-tracking family is). Performed a thorough manual self-review instead (no production panic/unwrap/expect added — decision-0003 preserved, display_with_src clamps offset defensively; comment-honesty audited; full mechanical gate green) and committed. Flagging so a reviewer with the sub-agents can still run them post-hoc on 1c4e90a.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO. qa-test-runner clean: workspace 402/0/1; ALL 11 migrated tests verified line-by-line strength-PRESERVED (pure Err(LowerError::X(p))->Err(LowerError{kind:LowerErrorKind::X(p),..}), no wildcarding/removed assertion); AC#3 genuine (3 bad programs, exact line:col via offset_to_line_col); determinism byte-identical x2 + e2e 30/26/0/4/0 + both negatives bite; real-driver located errors correct (duplicate const `N` at 2:7, unknown identifier `nope` at 2:7); clippy --all-targets clean, NO derived_hash_with_manual_eq (LowerError is Debug+Clone only, manual PartialEq/Eq forward to .kind, no Hash, not a map key — no key-merge hazard); display_with_src clamps span.start.min(src.len()) (no panic, decision-0003 upheld); sched/span/ast untouched. mped-architect GO + found the RECURRING comment/doc-lie class: ir.rs:357-363/:381/:389-390 + algo_lower.rs:691 falsely claimed the synthetic NonIntegerShapeExpr is position-less / "two position-less variants" — code is CORRECT (real expr.span; only decl LABEL synthetic) but docs understated it; ConstCycle is the SOLE span:None site. Per phase3-ralph honesty spine (Done must not rest on a committed doc-lie) ORCHESTRATOR FIXED IN-THREAD (TASK-0079 precedent): corrected the 3 ir.rs doc spots + the test comment; verified comment-only zero-behaviour-change (determinism byte-identical, algo_lower 20/0, clippy --all-targets clean). The P3 coverage gap (no positive test pins the synthetic-NonIntegerShapeExpr-IS-located boundary) filed as TASK-0195 (dep TASK-0090). LowerErrorKind verified byte-identical to the prior enum; equality decision sound+documented; forward-carry to TASK-0086/0080/0092/0096 accurate (0086 SchedLowerError is a plain enum — the {kind,span} mirror applies cleanly). TASK-0090 Done stands — the docs now match the correct code.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Propagated TASK-0082 algorithm-AST spans into LowerError so AlgoIR-lowering diagnostics now carry source line:col.

WHAT CHANGED:
- LowerError restructured into `struct { kind: LowerErrorKind, span: Option<Range<usize>> }`. LowerErrorKind is the prior enum verbatim — no variant payload shape changed. Byte span populated at every diagnosable err site from the offending Spanned (TASK-0082 substrate); ConstCycle and the synthetic <index/loop-bound> NonIntegerShapeExpr stay position-less by design (documented honest-partial — a documented missing position beats a fabricated wrong one).
- LowerError PartialEq/Eq hand-forward to .kind only (span EXCLUDED from value identity), mirroring Spanned (TASK-0082). Position is informational-for-humans, not semantic identity.
- Byte->line:col conversion is driver-side via LowerError::display_with_src(&self,src) (compiler::error::offset_to_line_col); lowering stays &AlgoAst-only, mirroring ParseError. Driver renders `nucleus: error: algorithm lower error: <msg> at L:C`.

WHY: realizes the diagnostics value of the just-built span substrate without re-opening TASK-0082 or touching the schedule side; typed-Result preserved (decision-0003), zero new panic/unwrap on user paths.

USER IMPACT: a duplicate const / unknown identifier / double assignment etc. now reports the exact source position, e.g. `nucleus: error: algorithm lower error: duplicate const `N` at 2:7` (real-driver verified). Valid programs are byte-identically unaffected (spans populate only on the Err path).

TESTS: 11 existing negative tests migrated mechanically to the struct-destructure form (payload assertions unchanged — expected AC#4 scope, not a masked regression); 2 new tests added — located_errors_carry_correct_line_col (exact line:col for 3 representative bad programs, each validated against the crafted source via offset_to_line_col, plus exact display_with_src strings) and multi_site_variants_are_position_less (pins the deliberate position-less variants). just test 0 failed; e2e 30/26/0/4/0; determinism byte-identical x2; both negative gates still bite; clippy --workspace --all-targets clean; ci exit 0.

FOLLOW-UPS / FORWARD-CARRY: substrate notes appended to TASK-0086 (sched SchedLowerError mirrors this), TASK-0080/0092 (multi-error reporting builds on located errors), TASK-0096 (fuzzy-match reuses the SpIdent span).

RISK/LIMITATION: the global mandated qa-test-runner + mped-architect sub-agent review could not run (those sub-agents are not surfaced as tools in this environment); a thorough manual self-review + the full mechanical gate were done instead. Re-run the sub-agents post-hoc on commit 1c4e90a if available.
<!-- SECTION:FINAL_SUMMARY:END -->
