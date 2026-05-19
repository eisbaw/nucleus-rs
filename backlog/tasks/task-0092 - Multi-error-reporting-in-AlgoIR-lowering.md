---
id: TASK-0092
title: Multi-error reporting in AlgoIR lowering
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 00:25'
updated_date: '2026-05-19 22:47'
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
TRANSITIVE-POISON FIX (5th-recurrence remediation, fresh session):

1. ONE-LINE CODE FIX. In lower.rs Accum::record_decl_failure, case-1 branch (currently 'if self.is_cascade_of_failed_decl(&e) { return; }'): BEFORE the early return, insert 'self.failed_decls.insert(name.to_string(), ());'. Sound because a cascade-decl by definition has no independent meaning — its upstream root is already poisoned; every downstream reference to it is by definition a transitive cascade of the same root.

2. DOCSTRING REWRITE at lower.rs:~189-218 (case-1 doc paragraph). Replace the 'KNOWN DEFECT' block with the corrected truth: cascade-decls ARE transitively poisoned at decl level; soundness via no-independent-meaning argument. Remove the disavowed 'Poisoning x here would compound the cascade' rationale entirely; remove the TASK-0092-known-defect label.

3. NEW DEPTH>1 PARAMETRISED FIXTURE in tests/algo_lower.rs (alongside the existing depth=1 trio). Shape:
   - For K in {1,2,3,5} cascade-decls (each a 'data dk_i : f32[N];' where the root 'const N : usize = 1/0;' is the poisoned upstream)
   - For L in {1,2,3} statements per cascade-decl referencing it (e.g. 'dk_i[0] = 1.0;' assignment statements that would emit AssignmentTargetNotData/UnknownIdent without the fix)
   - Assert errors().len() == 1 (the root ConstDivByZero{N}) for every (K,L).
   K and L each iterate >=3 values → genuine parametric coverage, not a single-shape masking fixture.

4. GATE x7 inside nix develop (must measure, not parrot): just test ≥447/0/2; cargo clippy --workspace --all-targets clean; just e2e EXACTLY 30/26/0/4/0; just determinism-check twice byte-identical; just determinism-check-negative bites 26/0; just xbackend-check-negative bites 13/1; just ci exit 0.

5. REAL-DRIVER CROSS-CHECK with a depth>1 input: 'const BAD: i32 = 1/0; const X: i32 = BAD + 1; data y : f32[X]; y[0] = 1.0; for i in 0..X { y[i] = y[i] + 1.0; }' — must emit EXACTLY 1 error (the BAD div-by-zero), capture verbatim stdout for the disclosure.

6. TRACKER UPDATE: append cycle-3 outcome paragraph to TASK-0092 notes; rewrite FORWARD-CARRY to TASK-0200 to reflect transferable corrected design; mark AC#3 ticked ONLY if K>=3 and L>=3 are both genuinely iterated AND every (K,L) measures exactly 1. Forward-carry note to TASK-0200 notes via --append-notes.

NO scope creep. NO touching multi-error infra, EffectStmt, depth=1 fixtures, ParseErrors/sched-lower. NO panics/unwraps on the lowering path.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0082 (DONE): algo AST is Spanned<T>-wrapped (compiler/src/algo/span.rs); lower.rs projects .node and currently IGNORES spans. TASK-0090 wires spans into LowerError; multi-error lowering here can then carry a precise (line,col) per error via error::offset_to_line_col on the relevant Spanned node's span.start. Keep typed-Result, no panic (decision-0003).

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
- DEPTH=1 ONLY (CORRECTED 2026-05-20 from prior doc-lie "applied transitively"): poisoning is at the immediate-failure boundary. A decl that fails to evaluate poisons its own name. A decl that fails ONLY because its referenced upstream is poisoned does NOT itself get poisoned — this is the known transitive overcount defect now documented in lower.rs:195-218 and tracked under this task. The previous "applied transitively / do not poison x" rationale was disavowed (see ORCHESTRATOR re-gate paragraph below and code-level docstring correction in commit dcdc302). AC#3 is correspondingly UNTICKED until the one-line transitive fix + depth>1 parametrised fixture land.

MEASURED COUNTS (varying input size — the AC#3 disclosure at depth=1):
- M independent bad decls -> EXACTLY M errors. Tested M in {1,2,3,5,8} in parametrised fixture, each error located at its own decl div-by-zero expression position (validated against src.match_indices, not a guessed constant). Driver-binary cross-check: M in {1,2,5} -> 1, 2, 5 lines emitted with correct distinct line:col (1:22, 2:22, ...).
- 1 failed const with N depth=1 dependents -> EXACTLY 1 error (no N-cascade). Tested N in {1,2,5,8} in parametrised fixture. Driver-binary cross-check: N in {1,2,5} -> 1, 1, 1.
- Combined (the depth=1 discriminating case): 1 failed const + N dependents (all suppressed) + M independents -> EXACTLY 1+M. Tested (N,M) in {1,2,5} x {1,2,3}. Catches both undercount (suppressing independents) and overcount (emitting cascade) — both bite at depth=1.
- DEPTH>1 NOT MEASURED IN FIXTURE — this is the masking-defect class that bit TASK-0080/0081/0087 too. The mped-architect PROBE 5 (1 poisoned const + 1 data using it + 2 statements using the data) measured 3 vs expected 1; PROBE 14 (poisoned const + data + for-loop using data) measured 2 vs expected 1. The transitive overcount remains a known defect (see ORCHESTRATOR re-gate paragraph).

BLAST RADIUS: only algo_lower.rs negatives changed (9 match-blocks via .map_err(|e| e.first().clone()), 6 expect_err sites via .first().clone() — discriminating arms unchanged, no blanket len() asserts; assertion strength preserved). Ok=AlgoIR preserved so the ~8 other lower_algo callers (transfer_inject, block_transform, acfg_to_petri, boundedness, deadlock, transfer_inject_hoist, contract, sync_inject, link, petri_to_events, acfg) compile untouched (their .expect/.unwrap works for any E: Debug).

DOC HONESTY (comment-doc-lie class): lower.rs module-doc and lower_algo fn-doc rewritten to multi-error reality — no stale aborts-on-first residue. Driver comment likewise updated. Commit dcdc302 subsequently corrected the case-1 record_decl_failure docstring at lower.rs:195-218 to accurately disclose the depth=1-only limitation (was previously claiming "applied transitively / do not poison x" — the disavowed rationale).

GATE (all inside nix develop):
- determinism-check x2: byte-identical 30/26/0/4 (valid programs lower to SAME AlgoIR — byte-identical codegen, the load-bearing zero-behaviour-change proof).
- e2e: 30/26/0/4 required-fail 0.
- determinism-check-negative: 0/26/4 (NUC_NONDET injection bites 26/30).
- xbackend-check-negative: 13 corrupted / 1 detected (required-fail 1).
- workspace cargo test: 447 passed / 0 failed / 2 ignored (440 baseline + 7 from TASK-0089).
- clippy --workspace --all-targets -- -D warnings: clean.
- just ci: exit 0.

FORWARD-CARRY to TASK-0200 (sched-lowering multi-error): the cascade-design template here transfers WITH CAVEAT — symbol-table-membership as the boundary, suppression keyed on the ident referenced by the four reference-error variant kinds (or sched-side equivalents), duplicates NOT poisoning. Depth=1 only at present (NOT transitive — the transitive overcount is a known defect tracked under this task, do NOT propagate the broken design to sched-lowering). TASK-0200 should pick up the corrected design once the one-line fix here lands. Different layer / different error type, same independence-vs-cascade discipline.

ORCHESTRATOR re-gate (cycle 1): NO-GO STANDS (reviewers split; mped-architect independently-measured falsifying evidence decisive over QA depth-1-only re-derivation, exactly as on TASK-0087). The recurring cascade/undercount-leak class RECURRED A FIFTH TIME across multiple tasks (TASK-0080/0081 "one"->two; TASK-0087 first "ONE/no for-body"->false; TASK-0087 correction "+2 bounded/not-a-cascade"->actually n+2 linear cascade; THIS cycle: transitive overcount leak). Precise defect: lower.rs ~205-221 Accum::record_decl_failure case-1 suppresses the cascade error AND does NOT poison the cascade-decl name. mped-architect PROBE 5 (1 poisoned const + 1 data using it + 2 statements using the data) measured 3 errors vs claimed 1; PROBE 14 (poisoned const + data + for-loop using data) measured 2 vs claimed 1. Root: when a decl fails only because it refers to a poisoned upstream, the decl name is left out of failed_decls -> downstream statements referencing it emit UnknownIdent("x") which is suppressible-as-cascade-by-design BUT does not match because x is not in failed_decls -> overcount leak. The docstring lower.rs:189-196 ("Applied transitively at decl level ... do not poison x") was a DOC-LIE: it claimed transitivity but the implementation drops the transitive poison; mped-architect proved the docstring justification was wrong (a name that NEVER successfully declared has no independent meaning; every reference IS a cascade by definition). PRECISE REMEDIATION (mped-architect-specified, fully scoped — for a FRESH session, not another cycle in this exhausted context): (1) one-line fix in case-1 of record_decl_failure: insert name into failed_decls before return; (2) add a transitive-depth parametrised fixture iterating K cascade-decls x L statements per cascade-decl, asserting exactly 1 error (not 1 + K*L); (3) correct the docstring to the corrected truth (cascade-decls ARE poisoned transitively; sound because cascade-decls have no independent dependents); (4) re-run the full gate; (5) update TASK-0200 forward-carry. Reliability-signal stop applies (5 recurrences across multiple tasks IS the textbook pattern phase3-ralph names). TASK-0092 In Progress; the multi-error feature is real and working for depth=1 (the common case); the transitive overcount is a precise localized known defect with a one-line fix tracked here.

ORCHESTRATOR re-gate (cycle 2, 2026-05-20 fresh session, post-TASK-0089): qa-test-runner GO (all 7 gate numbers match implementer claims: test 447/0/2, clippy clean, e2e 30/26/0/4/0, det-check x2 byte-identical 30/26/0/4, det-check-negative 26 perturbed/0 pass, xbackend-check-negative 13 corrupted/1 detected, just ci exit 0); mped-architect GO-with-caveats (TASK-0092 tracker still contained the disavowed "applied transitively at decl level" sentences at three locations — DESIGN bullet, TASK-0200 forward-carry, Final Summary — AND AC#3 still ticked despite the measured n+2 overcount). The cascade-class doc-lie had migrated from code (corrected by dcdc302) into tracker (this layer). Tracker cleanup applied IN-THREAD by orchestrator (this revision): DESIGN bullet rewritten to depth=1-only with explicit disavowal of the prior transitive claim; FORWARD-CARRY to TASK-0200 corrected to NOT propagate the broken design; AC#3 UNTICKED; Final Summary correspondingly rewritten; MEASURED COUNTS section explicitly delineated as depth=1 measurements and DEPTH>1 NOT MEASURED IN FIXTURE called out. Two new follow-up tasks filed (multi-error span line:col under-assertion + Effect-to-poisoned-kernel cascade-short-circuit test that depends on the case-1 fix). NO-GO STANDS pending the precise one-line transitive fix + the depth>1 parametrised fixture + the case-1 docstring update at lower.rs:189-218 to the corrected truth.

ORCHESTRATOR re-gate (cycle 3, 2026-05-20 fresh implementer session): TRANSITIVE-POISON FIX LANDED, commit 79c654d.

CODE FIX (one line, fully scoped): in Accum::record_decl_failure case-1 branch (the cascade-decl path), before the early return, insert self.failed_decls.insert(name.to_string(), ()). Cascade-decls have no independent meaning — the upstream root is already poisoned; every downstream reference is by definition a transitive cascade of that same root. Soundness: see the rewritten case-1 docstring at lower.rs:~196-220 (the KNOWN DEFECT block is GONE; replaced by an explicit soundness argument).

DOCSTRING REWRITE: case-1 paragraph at lower.rs:189-220 now states 'AND transitively poison this declaration's own name'; the disavowed 'Poisoning x here would compound the cascade' rationale is removed; the KNOWN DEFECT label is removed; the soundness argument is the no-independent-meaning truth. The lower_algo counting-contract paragraph at lower.rs:109-120 is correspondingly upgraded to include the transitive-depth case (1 root + K cascade-decls × L statements -> exactly 1 for any K, L).

NEW PARAMETRISED FIXTURE: tests/algo_lower.rs:986-1037 transitive_cascade_collapses_for_any_k_l — K in {1,2,3,5} cascade-data-decls × L in {1,2,3} dump-statements per cascade-decl. Each (K,L) asserts errors().len() == 1 and the surviving error is ConstDivByZero{in_const: N}. Genuinely parametric in BOTH dimensions (>=3 distinct values each); single-shape masking-defect avoided.

MEASURED COUNTS (depth>1, this cycle):
- The new parametrised fixture: every (K,L) in {1,2,3,5}x{1,2,3} measures exactly 1 error. 12 combinations total. PASS.
- Real-driver (cargo run --bin nucleus -- build --algo /tmp/depth2_probe.algo.nuc ...):
    const BAD : usize = 1 / 0;
    const X : usize = BAD + 1;
    data y : f32[X];
    kernel dump : (f32[X]) -> () effectful;
    dump(y);
    for i : 0 .. X { dump(y); }
  emits:
    nucleus: error: algorithm lower error(s) in /tmp/depth2_probe.algo.nuc (1):
      - divide-by-zero in const `BAD` at 1:21
  EXACTLY 1 error (pre-fix would have been 4-5).
- Depth-3 chain probe (BAD -> X -> Y -> 3 data decls -> 4 statements): also exactly 1 error.
- Independence preserved: 1 cascade root + 2 independent bad consts measures exactly 3 errors (root + 2 indep), confirming the fix does NOT over-suppress independents.

EXISTING DEPTH=1 FIXTURES (no regression): m_independent_bad_decls_yield_exactly_m_errors, one_failed_const_with_n_dependents_yields_exactly_one_error, cascade_suppressed_while_independents_still_surface, valid_program_still_lowers_under_multi_error — all pass (case-1 fires only on the cascade branch; duplicate-name and genuine-independent branches are untouched).

GATE x7 (all inside nix develop, measured this cycle):
- just test: 448 passed / 0 failed / 2 ignored (+1 from new transitive test, 447 baseline).
- cargo clippy --workspace --all-targets -- -D warnings: clean exit 0.
- just e2e: 30 / 26 / 0 / 4 / required-fail 0.
- just determinism-check (x2): both 30/26/0/4, byte-identical (valid programs lower to same AlgoIR).
- just determinism-check-negative: bites, NUC_NONDET_PERTURBED_CELLS=26 / 0 pass.
- just xbackend-check-negative: bites, NUC_XBACKEND_CORRUPTED_APPLIED=13 / NUC_XBACKEND_CORRUPTED_DETECTED=1.
- just ci: exit 0.

AC#3 RE-TICKED: K iterates {1,2,3,5}, L iterates {1,2,3}; 12 (K,L) combinations measured exactly 1 error each; the depth>1 case AND the depth=1 case AND the independence dimension AND the duplicate dimension are all separately exercised; real-driver cross-check confirms depth>1 = 1. The masking-defect class — single-shape fixture or single-dimension parametrisation — is genuinely avoided.

TASK-0203 (Effect-to-poisoned-kernel cascade-short-circuit test) is now UNBLOCKED — the transitive-poison fix is the prerequisite this cycle resolves.

FORWARD-CARRY to TASK-0200 (POST-FIX, cycle 3): the cascade-design template here now transfers CLEANLY — including the transitive dimension. Sched-lowering multi-error should pick up:
- LowerErrors / SchedErrors owner pattern (Vec<E>, non-empty invariant, per-line Display).
- Symbol-table-membership as the cascade boundary; reference errors keyed by referenced ident.
- Duplicates do NOT poison (first decl valid).
- **Cascade-decls ARE transitively poisoned at decl level** — insert the failing name into the poisoned set BOTH when the failure is genuine-independent AND when it is itself a cascade (case-1). This is the corrected design; do NOT carry the prior depth=1-only design forward. The soundness argument is no-independent-meaning: a cascade-decl has no value/shape/sig, so every reference to it is by definition a transitive cascade of the same upstream root.
- Parametrise the regression fixture in BOTH dimensions — single-shape (single K or single L) is the masking-defect class. Iterate >=3 distinct values per dimension.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Multi-error AlgoIR lowering: lower_algo accumulates every genuinely-independent LowerError in one pass (LowerErrors owner) instead of ?-bailing on the first, and does NOT emit cascade errors (direct or TRANSITIVE references to a declaration that itself failed). The cycle-3 transitive-poison fix (commit 79c654d) corrects the cascade-class undercount/overcount defect at depth>1 — cascade-decls are now transitively poisoned at decl level, so the depth>1 case collapses to the root just like the depth=1 case.

Why and what:
- Users seeing semantically broken algorithm sources get every violation in one compile cycle, including arbitrarily-deep transitive cascades collapsing to their single root failure (no n+2 / 1+K*L overcount).
- Ok type is unchanged AlgoIR — the ~8 success-path callers compile untouched; blast radius confined to algo_lower.rs negatives.

Independent-vs-cascade discipline (AC#2 / AC#3, now transitive):
The symbol table IS the cascade boundary. A declaration that fails to evaluate is not inserted into ir.consts/data/kernels and its name is recorded in Accum::failed_decls. Reference errors (UnknownIdent, AssignmentTargetNotData, ConstRefersToNonConst, ShapeRefersToNonConst) whose referenced identifier is poisoned are suppressed. Duplicate-decl errors do NOT poison (first decl valid). **Transitively** (cycle-3 fix at case-1 of Accum::record_decl_failure): a declaration that fails ONLY because it references an already-poisoned upstream is ITSELF inserted into failed_decls — cascade-decls have no independent meaning, every downstream reference is by definition a transitive cascade of the same upstream root. The case-1 docstring at lower.rs:189-220 is rewritten to the corrected truth with the no-independent-meaning soundness argument; the KNOWN DEFECT block is removed.

Measured counts (AC#3, post-fix):
- M independent bad decls -> EXACTLY M errors. Parametric M in {1,2,3,5,8}.
- 1 failed const with N depth=1 dependents -> EXACTLY 1 (no N-cascade). Parametric N in {1,2,5,8}.
- Combined: 1 root + N suppressed + M independents -> EXACTLY 1+M. (N,M) in {1,2,5}x{1,2,3}.
- TRANSITIVE (NEW, the cycle-3 closer): 1 root + K cascade-decls × L statements per cascade-decl -> EXACTLY 1 for every (K,L) in {1,2,3,5}x{1,2,3}. Fixture transitive_cascade_collapses_for_any_k_l. Real-driver depth>1 and depth-3 cross-checks both emit exactly 1 error.

Driver surface unchanged:
  nucleus: error: algorithm lower error(s) in <path> (<count>):
    - <kind> at <line>:<col>
    - ...
matching the parse_algo/link/contract shape.

Doc honesty (AC#4): module/lower_algo/case-1 docstrings reflect the multi-error + transitive-poison reality with the soundness argument explicit; no stale aborts-on-first or KNOWN-DEFECT residue. decision-0003 preserved (typed Result, no panics on the lowering path).

Tests:
- algo_lower.rs negatives migrated (.first().clone()); strength preserved.
- Parametric cascade fixtures at depth=1 (3 functions, M/N/combined) AND at depth>1 (transitive_cascade_collapses_for_any_k_l, K×L parametric).
- valid_program_still_lowers_under_multi_error guards AC#5 at unit level.

Gate (all inside nix develop, cycle-3 measurements):
- just test: 448 passed / 0 failed / 2 ignored (447 baseline + 1 new transitive test).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 30 / 26 / 0 / 4 / required-fail 0.
- just determinism-check (x2): 30/26/0/4 byte-identical.
- just determinism-check-negative: bites (26 perturbed / 0 pass).
- just xbackend-check-negative: bites (13 corrupted / 1 detected required).
- just ci: exit 0.

Scope: AlgoIR lowering. Sched-lowering multi-error remains TASK-0200 — the cascade-design template now transfers cleanly including the transitive dimension (forward-carry note appended to TASK-0200). TASK-0203 (Effect-to-poisoned-kernel cascade-short-circuit test) is now UNBLOCKED.

Commits: 0548f02 (multi-error infra) + dcdc302 (depth=1-only honesty docstring) + 79c654d (cycle-3 transitive-poison fix + K×L parametric fixture). All five ACs ticked. Status: ready-for-Done pending orchestrator review.
<!-- SECTION:FINAL_SUMMARY:END -->
