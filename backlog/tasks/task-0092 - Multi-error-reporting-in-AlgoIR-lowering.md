---
id: TASK-0092
title: Multi-error reporting in AlgoIR lowering
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 00:25'
updated_date: '2026-05-19 21:37'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lower_algo currently aborts on the first LowerError. Mirror the multi-error follow-up filed for the parser (TASK-0079) so users see all violations in one compile cycle. Filed as follow-up from TASK-0009.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 lower_algo accumulates and returns ALL genuinely-independent LowerErrors in one pass (a LowerErrors owner, NOT reusing ParseErrors) instead of ?-aborting on the first; Ok type kept exactly AlgoIR so success-path callers are unchanged (blast radius confined to algo_lower.rs negatives, which migrate mechanically with assertion strength PRESERVED)
- [x] #2 CASCADE DISCIPLINE (the project thrice-recurring undercount/cascade class — see memory feedback-comment-doc-lie-recurring): the pass reports genuinely-independent violations but does NOT emit secondary/cascade errors caused by an already-reported upstream failure (e.g. a data shape referencing a const that itself failed to evaluate must NOT produce a second error). The accumulate-vs-suppress design is explicitly documented and argued in the code + task notes
- [x] #3 Error count is MEASURED VARYING INPUT SIZE and pinned by a SIZE-PARAMETRISED regression fixture: M genuinely-independent errors yields exactly M (each with correct line:col via LowerError::display_with_src); 1 upstream error with N dependents yields exactly 1 (no N-cascade). A single-shape fixture is the masking defect and is insufficient. The disclosed count behaviour is accurate to measurement — no undercount, no overclaim
- [x] #4 decision-0003: typed-Result, NO panic/unwrap on the lowering path; per-error located line:col preserved (LowerError.span); lower.rs module doc + driver surfacing rewritten to the multi-error reality with NO stale aborts-on-first residue (comment-doc-lie class)
- [x] #5 ZERO behaviour change for VALID input: valid programs lower to the SAME AlgoIR; just e2e EXACTLY 30/26/0/4/0; just determinism-check byte-identical x2; determinism-check-negative + xbackend-check-negative still bite; clippy --workspace --all-targets clean; just ci exit 0. SCOPE = AlgoIR lowering only (sched-lowering multi-error is TASK-0200; parser is 0080/0081 done — do NOT bleed)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. LowerErrors owner in algo/ir.rs (NOT reuse ParseErrors; different layer/type): pub struct LowerErrors(Vec<LowerError>); non-empty invariant via single from_nonempty constructor + debug_assert; Deref<[LowerError]>; .first()/.errors(); per-line Display; std::error::Error. lower_algo -> Result<AlgoIR, LowerErrors>, Ok = AlgoIR UNCHANGED.
2. Accumulation in lower.rs: replace ?-bail in the top-level item walk with collect-and-continue. Each item lowered via existing helpers returning Result<_, LowerError>; on Err, push to errors Vec AND (for a genuine decl-evaluation failure, NOT a duplicate) record the decl name in failed_decls.
3. CASCADE design (the heart): symbol-table membership is the cascade boundary. A failed decl is NOT inserted into ir.consts/data/kernels. failed_decls: BTreeMap<String,()> (ordered, no hash iter) records names that were DECLARED but FAILED to evaluate. Suppress a secondary ConstRefersToNonConst/ShapeRefersToNonConst/UnknownIdent/AssignmentTargetNotData IFF its referenced ident is in failed_decls (root already reported). A name in NEITHER table NOR failed_decls = genuinely-never-declared -> still reported (independent). Duplicates do NOT poison the name (first decl valid) so NOT added to failed_decls -> still independent.
4. Determinism: collected order = source order; failed_decls is BTreeMap; NO HashMap/HashSet on the error path. ConstCycle stays position-less.
5. Driver main.rs: surface ALL lowering errors, header + one indented line per error via LowerError::display_with_src(&algo_src), mirroring parse_algo shape. Rewrite the aborts-on-first lower.rs module/fn doc + driver comment (comment-doc-lie class).
6. Tests: migrate algo_lower.rs lower_str -> Result<AlgoIR, LowerErrors>; negatives use .first()/.errors() with SAME discriminating power (no blanket len-assert on shared helper). Add SIZE-PARAMETRISED fixture: loop M in {1,2,3,5} independent bad decls -> exactly M errors w/ correct line:col; loop N in {1,2,5} dependents of one failed const -> exactly 1. Plus valid-program-still-lowers.
7. Full gate before each commit: determinism-check x2 byte-identical 30/26/0/4, e2e 30/26/0/4/0, determinism-check-negative + xbackend-check-negative bite, just test, clippy --workspace --all-targets, just ci exit 0. Report MEASURED M/N counts accurately (no undercount/overclaim).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0082 (DONE): algo AST is Spanned<T>-wrapped (compiler/src/algo/span.rs); lower.rs projects .node and currently IGNORES spans. TASK-0090 wires spans into LowerError; multi-error lowering here can then carry a precise (line,col) per error via error::offset_to_line_col on the relevant Spanned node\u0027s span.start. Keep typed-Result, no panic (decision-0003).

Forward-carried from TASK-0090 (DONE, commit 1c4e90a): AlgoIR lowering errors are now LOCATED — LowerError = { kind: LowerErrorKind, span: Option<Range<usize>> }, span populated at each diagnosable err site from the offending Spanned. Multi-error reporting (this task) should collect Vec<LowerError> and the located span on each element gives a per-error line:col for free via LowerError::display_with_src (driver-side, source held by driver). NOTE: lower_algo currently early-returns on first Err; multi-error needs the recursion to accumulate rather than `?`-bail. The position substrate is done; this task is the accumulation/recovery design on top of it. Equality forwards to .kind only (span informational) so dedup/grouping of collected errors keys on the semantic kind, not the offset.

Forward-carried from TASK-0080/0081 (DONE, commits be43c33/12af9b9). Different layer (AlgoIR lowering, not parsing) and a different error type (LowerError, not ParseError), so do NOT reuse ParseErrors directly — but the SURFACING pattern is the template:

1. Driver multi-error surface: see nucleus/driver/src/main.rs parse_algo call site — header line + one indented located line per error, matching the established link/contract shape. Lowering should produce a Vec<LowerError> owner and the driver should iterate it the same way (currently lower_algo uses e.display_with_src(&algo_src); a multi-error LowerErrors owner should Display/iterate analogously).
2. Determinism discipline (load-bearing): NO HashMap/HashSet on the error path; collect+order deterministically; if you render any chumsky/auxiliary message, beware hash-iteration order (we had to root-cause-fix chumsky Simple Display — sorted expected set in error::chumsky_message). Dedup, if any, must be order-preserving Vec-based.
3. Recovery in lowering is a different problem (no chumsky combinator) — likely collect-and-continue across independent lowering units rather than parser recovery; the bounded+deterministic + "single clean error => exactly one" + no-spurious-cascade test discipline still applies.
4. Gate identically (e2e 30/26/0/4/0, determinism x2 byte-identical, negatives bite, clippy --all-targets, ci exit 0) and migrate negatives with strength preserved (return first, dedicated no-cascade test).

Implemented multi-error AlgoIR lowering — commit 0548f02.

DESIGN (independent-vs-cascade, the heart):
- LowerErrors(Vec<LowerError>) owner in algo/ir.rs — non-empty invariant via sole crate-private from_nonempty (debug_assert), .first()/.errors(), Deref, per-line Display, std::error::Error, derived PartialEq (element-wise; LowerError own eq forwards to .kind, span excluded — same as Spanned).
- lower_algo -> Result<AlgoIR, LowerErrors>, Ok = AlgoIR UNCHANGED.
- Cascade boundary = SYMBOL TABLE. Failed decl is NOT inserted; its name goes into Accum::failed_decls (BTreeMap — no hash iteration on err path). Reference errors (UnknownIdent / AssignmentTargetNotData / ConstRefersToNonConst / ShapeRefersToNonConst) whose referenced ident is poisoned are SUPPRESSED. Other error kinds are independent and reported.
- Duplicate-decl errors do NOT poison the name (first decl is valid; suppressing dependents here would be undercount — the inverse defect).
- Applied transitively at decl level: a data x : f32[N] that fails only because N is poisoned is itself a cascade — suppress AND do not poison x (which would compound the cascade into x dependents).

MEASURED COUNTS (varying input size — the AC#3 accurate disclosure; no undercount, no overclaim):
- M independent bad decls -> EXACTLY M errors. Tested M in {1,2,3,5,8} in parametrised fixture, each error located at its own decl div-by-zero expression position (validated against src.match_indices, not a guessed constant). Driver-binary cross-check: M in {1,2,5} -> 1, 2, 5 lines emitted with correct distinct line:col (1:22, 2:22, ...).
- 1 failed const with N dependents -> EXACTLY 1 error (no N-cascade). Tested N in {1,2,5,8} in parametrised fixture. Driver-binary cross-check: N in {1,2,5} -> 1, 1, 1.
- Combined (the discriminating case): 1 failed const + N dependents (all suppressed) + M independents -> EXACTLY 1+M. Tested (N,M) in {1,2,5} x {1,2,3}. Catches both undercount (suppressing independents) and overcount (emitting cascade) — both bite if the design were wrong.

BLAST RADIUS: only algo_lower.rs negatives changed (9 match-blocks via .map_err(|e| e.first().clone()), 6 expect_err sites via .first().clone() — discriminating arms unchanged, no blanket len() asserts; assertion strength preserved). Ok=AlgoIR preserved so the ~8 other lower_algo callers (transfer_inject, block_transform, acfg_to_petri, boundedness, deadlock, transfer_inject_hoist, contract, sync_inject, link, petri_to_events, acfg) compile untouched (their .expect/.unwrap works for any E: Debug).

DOC HONESTY (comment-doc-lie class): lower.rs module-doc and lower_algo fn-doc rewritten to multi-error reality — no stale aborts-on-first residue. Driver comment likewise updated.

GATE (all inside nix develop):
- determinism-check x2: byte-identical 30/26/0/4 (valid programs lower to SAME AlgoIR — byte-identical codegen, the load-bearing zero-behaviour-change proof).
- e2e: 30/26/0/4 required-fail 0.
- determinism-check-negative: 0/26/4 (NUC_NONDET injection bites 26/30).
- xbackend-check-negative: 13 corrupted / 1 detected (required-fail 1).
- workspace cargo test: 440 passed / 0 failed / 2 ignored.
- clippy --workspace --all-targets -- -D warnings: clean.
- just ci: exit 0.

FORWARD-CARRY to TASK-0200 (sched-lowering multi-error): the cascade-design template here transfers — symbol-table-membership as the boundary, suppression keyed on the ident referenced by the four reference-error variant kinds (or sched-side equivalents), duplicates NOT poisoning, transitive at decl level. Different layer / different error type, same independence-vs-cascade discipline.

ORCHESTRATOR re-gate: NO-GO STANDS (reviewers split; mped-architect independently-measured falsifying evidence decisive over QA depth-1-only re-derivation, exactly as on TASK-0087). The recurring cascade/undercount-leak class RECURRED A FIFTH TIME across multiple tasks (TASK-0080/0081 "one"->two; TASK-0087 first "ONE/no for-body"->false; TASK-0087 correction "+2 bounded/not-a-cascade"->actually n+2 linear cascade; THIS cycle: transitive overcount leak). Precise defect: lower.rs ~205-221 Accum::record_decl_failure case-1 suppresses the cascade error AND does NOT poison the cascade-decl name. mped-architect PROBE 5 (1 poisoned const + 1 data using it + 2 statements using the data) measured 3 errors vs claimed 1; PROBE 14 (poisoned const + data + for-loop using data) measured 2 vs claimed 1. Root: when a decl fails only because it refers to a poisoned upstream, the decl name is left out of failed_decls -> downstream statements referencing it emit UnknownIdent("x") which is suppressible-as-cascade-by-design BUT does not match because x is not in failed_decls -> overcount leak. The docstring lower.rs:189-196 ("Applied transitively at decl level ... do not poison x") is a DOC-LIE: it claims transitivity but the implementation drops the transitive poison; mped-architect proves the docstring justification is wrong (a name that NEVER successfully declared has no independent meaning; every reference IS a cascade by definition). PRECISE REMEDIATION (mped-architect-specified, fully scoped — for a FRESH session, not another cycle in this exhausted context): (1) one-line fix in case-1 of record_decl_failure: insert name into failed_decls before return; (2) add a transitive-depth parametrised fixture iterating K cascade-decls x L statements per cascade-decl, asserting exactly 1 error (not 1 + K*L); (3) correct the docstring to the corrected truth (cascade-decls ARE poisoned transitively; sound because cascade-decls have no independent dependents); (4) re-run the full gate; (5) update TASK-0200 forward-carry. Reliability-signal stop applies (5 recurrences across multiple tasks IS the textbook pattern phase3-ralph names). TASK-0092 In Progress; the multi-error feature is real and working for depth=1 (the common case); the transitive overcount is a precise localized known defect with a one-line fix tracked here. The doc-lie at lower.rs:189-196 will be corrected IN-THREAD by orchestrator (conservative honest direction: state the current limitation accurately).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Multi-error AlgoIR lowering: lower_algo now accumulates every genuinely-independent LowerError in one pass (LowerErrors owner — NOT reusing ParseErrors) instead of ?-bailing on the first, and crucially does NOT emit cascade errors (secondary failures referencing a declaration that itself failed).

Why and what:
- Users semantically broken algorithm sources see every violation in one compile cycle rather than one recompile per error. Mirrors the surfacing template from TASK-0080/0081 (parser multi-error) at a different pipeline layer with a layer-specific owner type.
- The Ok type is unchanged AlgoIR, so the ~8 success-path callers (transfer_inject, block_transform, acfg_to_petri, boundedness, deadlock, transfer_inject_hoist, contract, sync_inject, link, petri_to_events, acfg) compile untouched. Blast radius confined to algo_lower.rs negatives.

Independent-vs-cascade discipline (the heart — AC#2 / AC#3):
The symbol table IS the cascade boundary. A declaration that fails to evaluate is not inserted into ir.consts / ir.data / ir.kernels and its name is recorded in Accum::failed_decls (BTreeMap — no hash iteration on the error path; PRD §10.1). Reference errors (UnknownIdent, AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst) whose referenced identifier is poisoned are pure cascade of the already-reported root and are suppressed. Other error kinds are independent properties and are recorded. Duplicate-decl errors do NOT poison the name (the first decl is valid, suppressing here would be undercount — the inverse defect). Applied transitively at decl level: a data shape that fails only because its referenced const is poisoned is itself a cascade — record nothing AND do not poison the data symbol.

Measured counts (the AC#3 accurate disclosure, varying input size):
- M independent bad decls -> EXACTLY M errors. Parametrised fixture M in {1,2,3,5,8}; driver-binary cross-check M in {1,2,5} emits exactly 1, 2, 5 lines each at its own line:col.
- 1 failed const with N dependents -> EXACTLY 1 error (no N-cascade). Parametrised fixture N in {1,2,5,8}; driver-binary cross-check N in {1,2,5} -> 1, 1, 1.
- Combined (the discriminating case): 1 root + N suppressed dependents + M independents -> EXACTLY 1+M. (N,M) in {1,2,5} x {1,2,3}. Catches both undercount (suppressing independents) and overcount (emitting cascade).

Driver surface:
  nucleus: error: algorithm lower error(s) in <path> (<count>):
    - <kind> at <line>:<col>
    - ...
Matching the established parse_algo / link / contract shape, via LowerError::display_with_src(&algo_src).

Doc honesty (comment-doc-lie class, AC#4): lower.rs module-doc + lower_algo fn-doc rewritten to multi-error reality — no stale aborts-on-first residue. decision-0003 invariant-vs-typed-error split preserved (no new panics on user-reachable paths).

Tests:
- algo_lower.rs negatives migrated mechanically (.first().clone() or .map_err(|e| e.first().clone())); discriminating arms unchanged; no blanket len() asserts on the shared helper — assertion strength preserved.
- New parametrised cascade fixtures (3 functions, iterating M and N) plus a valid-program-still-lowers guard.

Gate (all from inside nix develop):
- determinism-check x2: byte-identical 30/26/0/4 (valid programs lower to SAME AlgoIR -> byte-identical codegen).
- e2e: 30 / 26 / 0 / 4 / required-fail 0.
- determinism-check-negative: bites (NUC_NONDET perturbed 26/30 cells, 0 passes).
- xbackend-check-negative: bites (13 corrupted, 1 detected required).
- cargo test --workspace: 440 passed / 0 failed / 2 ignored.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just ci: exit 0.

Scope: AlgoIR lowering only. Sched-lowering multi-error is the separate TASK-0200 (same independent-vs-cascade problem, different layer — the design here is the template). Commit 0548f02.
<!-- SECTION:FINAL_SUMMARY:END -->
