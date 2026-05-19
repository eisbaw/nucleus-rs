---
id: TASK-0080
title: 'Algorithm parser: multi-error reporting'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:03'
updated_date: '2026-05-19 19:59'
labels:
  - compiler
  - language
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surface more than one parse error in a single pass. chumsky's Simple<char> already collects multiple alternatives at the same position; what we need is to recover from one syntactic failure and continue parsing the rest of the input, then bundle the collected errors into a Vec<ParseError> on return. Touches src/algo/parser.rs (recovery combinators) and the ParseError signature (probably becomes Vec<ParseError> or a struct that owns Vec).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 parse_algo surfaces MORE THAN ONE parse error in a single pass (recovery from TASK-0081 + bundle into the new signature); a multi-error fixture asserts >=2 distinct errors with CORRECT per-error line:col (via the existing error::offset_to_line_col / ParseError span infra — locatedness preserved)
- [x] #2 The ParseError->multi signature change (Vec<ParseError> or a struct owning Vec) is threaded through ALL callers: the driver (algo parse-error surfacing), every algo_parser.rs/algo_lower.rs test that calls parse_algo — existing single-error assertions migrate MECHANICALLY with assertion strength PRESERVED (no loosened/wildcarded/removed assertion); a single-error input still reports exactly that one error
- [x] #3 ZERO behaviour change for VALID input: valid programs parse to the SAME AST; just e2e EXACTLY 30/26/0/4/0; just determinism-check byte-identical x2; just determinism-check-negative + xbackend-check-negative still bite; clippy --workspace --all-targets clean; just ci exit 0
- [x] #4 decision-0003: typed-Result, NO panic/unwrap on parse paths; recovery is bounded/deterministic; SCOPE = algorithm parser only (TASK-0087 sched parser + TASK-0092 lowering are separate sibling tasks — do NOT bleed)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Error type: add `pub struct ParseErrors(pub Vec<ParseError>)` in error.rs; Deref<[ParseError]>, .first(), Display = one ParseError per line. Keep `map_first_chumsky_error` UNCHANGED (sched/parser.rs:56 shares it — out of scope TASK-0087). Add `map_all_chumsky_errors(src, Vec<Simple>) -> ParseErrors` with deterministic dedup+order.
2. parse_algo -> Result<AlgoAst, ParseErrors>; Ok type stays AlgoAst (blast radius = only algo_parser.rs negatives; ~9 non-parser test files + driver success-path unchanged). Use parser.parse_recovery(src).
3. Recovery (TASK-0081): recover_with(skip_then_retry_until([sync tokens])) anchored at stmt/item boundary (;, kernel/data/const/for starts) — bounded, deterministic, no infinite retry.
4. map_all: map EVERY Simple<char> -> ParseError via offset_to_line_col; dedup by (line,col,kind,message) preserving first-seen positional order (no HashSet — Vec dedup, deterministic).
5. Driver main.rs:174: iterate ParseErrors, one `nucleus: error:` line each.
6. Tests: migrate expect_err -> first ParseError (strength preserved); add multi-error fixture (>=2 distinct errs, correct per-err line:col), recovery test, no-spurious-cascade single-error test, bounded pathological-input test.
7. Full gate: determinism x2 byte-identical, e2e 30/26/0/4/0, negatives bite, clippy --all-targets, ci. Update parser.rs + error.rs module docs (no stale "first error only"/"bails" residue — doc-lie defect class).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0082 (DONE): Spanned<T> substrate at compiler/src/algo/span.rs; parse_algo populates tight byte ranges; PartialEq ignores span. For multi-error reporting, each Spanned node already knows its source range — no extra plumbing to locate a recovered error site. chumsky 0.9 span = Range<usize>.

Forward-carried from TASK-0090 (DONE, commit 1c4e90a): the located-error pattern (typed Kind enum + Option<Range> span wrapper, PartialEq forwards to kind only, driver-side display_with_src using compiler::error::offset_to_line_col) is now established on the algo LOWERING side. Algo PARSER multi-error reporting (this task) can reuse compiler::error::offset_to_line_col and the same "convert byte offset to L:C where the source &str is available (driver), keep the producing pass source-text-free" split. ParseError already carries (line,column) directly; the new precedent for the OTHER passes is: carry the byte Range, convert at the surface that owns src.

IMPLEMENTED & FULL GATE GREEN (commits be43c33 feature, 12af9b9 tests).

Error-type design: new error::ParseErrors(pub Vec<ParseError>) — non-empty invariant, Deref<[ParseError]>, .first()/.errors(), per-line Display. parse_algo -> Result<AlgoAst, ParseErrors>; Ok type UNCHANGED (AlgoAst). Blast-radius outcome: ONLY algo_parser.rs migrated; driver + ~9 non-parser test files (sync_inject/transfer_inject/acfg/contract/link/deadlock/block_transform/petri_to_events/algo_lower) compiled & passed with ZERO edits — confirms Ok-preservation strategy. sched parser untouched: map_first_chumsky_error retained verbatim, parse_sched still -> ParseError (TASK-0087 separate).

map_all_chumsky_errors: maps every Simple<char>; order-preserving exact-dup dedup via Vec.contains (NO HashSet) -> deterministic set+order.

ROOT-CAUSE FIX (reproducibility): chumsky 0.9 Simple Display iterates an internal HashSet so "expected one of ..." order was non-deterministic across identical parses (a real reproducibility-gate violation, surfaced empirically by the determinism test). New chumsky_message() rebuilds the message with SORTED expected set; also fixes the same latent non-determinism on the sched parser (shared helper). No test asserts message text; valid input never hits this path.

Test migration (strength preserved): expect_err returns ParseErrors::first(); every legacy .line/.column/.kind assertion byte-identical. NOT adding a blanket len==1 to the shared helper because several legacy fixtures put the sole error at EOF where ;-recovery legitimately reports a structural follow-on (would make exactly-one FALSE, not stronger). Dedicated single_error_input_yields_exactly_one_error_no_cascade pins the real AC (single clean error + valid tail => exactly 1).

GATE (actual): workspace cargo test 430 passed / 0 failed; e2e EXACTLY 30/26/0/4 required-fail 0; determinism-check byte-identical x2 (30/26/0/4 twice); determinism-check-negative perturbed 26 (bites); xbackend-check-negative detected injected corruption (bites); clippy --workspace --all-targets clean; just ci exit 0. Driver evidence: 2-error file prints both (L2C14 `?`, L4C21 `=`) + structural EOF; 1-error+valid-tail file prints EXACTLY 1 (L2C17).

GOTCHA/limitation: ;-only sync set => a malformed FINAL item or a broken stmt inside for{...} yields ONE legitimate secondary follow-on (trailing UnexpectedEof / stray `}`) beyond the primary. Real, bounded, deterministic — NOT a cascade. TASK-0199 filed (keyword-anchored sync) dep 0080/0081; referenced in parser.rs module doc + program_parser doc.

ORCHESTRATOR review-gate close (phase3-ralph, BOTH reviewers GO — combined cycle, both Done): qa-test-runner (re-verified): NO hash-order in any parser-error path (chumsky_message sorts+dedups a Vec; sched shares the same fixed helper), determinism byte-identical x2 + the determinism tests genuinely assert repeat-identical error output; ONLY algo_parser.rs changed among test files (Ok type stayed AlgoAst — blast radius confined, proven structurally); workspace 430/0; negative migrations strength-preserved (expect_err +.first().clone(), zero call-site assertion changes, no masked weakening); multi-error/recovery/no-cascade/bounded-pathological tests genuine; e2e EXACTLY 30/26/0/4/0; both negatives bite; clippy --all-targets clean; ci exit 0; no parse-path panic; no stale "bails on first error" doc residue. mped-architect (re-derived from code incl. chumsky 0.9 library source): error-type design sound + Ok-preserving; the chumsky-HashSet reproducibility ROOT-FIX is real/complete/correctly-placed at the single shared map_one_chumsky_error->chumsky_message helper and GENUINELY fixes the sched parser latent same-bug for free (verified by call graph, zero sched edits — correct root placement); recovery PROVABLY bounded (verified against chumsky recovery.rs: strict >=1-token advance, unconditional break on sync/EOF, no quadratic/hang) + deterministic; the not-blanket-len()==1 decision is correctly reasoned (a sometimes-false assert is wrong not stronger; no-cascade pinned by a dedicated test); decision-0003 clean (the 2 expects are guarded library-contract invariants). One genuine root-fix found DURING verification (chumsky Simple Display HashSet non-determinism) — honest find, airtight fix, zero-behaviour-change for valid input. Non-blocking findings APPLIED IN-THREAD by orchestrator (phase3-ralph step 4): (a) the imprecise error.rs "None sorts last" comment rewritten accurately (commit after 12af9b9; comment-only, gate re-verified determinism byte-identical + clippy --all-targets); (b) the qa-found disclosure UNDERCOUNT (real max is TWO bounded structural follow-ons — stray } AND/OR trailing UnexpectedEof, simultaneously possible in a for{} body near EOF — not "ONE"; bounded, deterministic, does-not-scale, primary always correct, NOT a cascade) corrected in the authoritative forward-looking source TASK-0199 description; (c) TASK-0199 had zero ACs (backlog-discipline gap) — 4 testable ACs added + the low-pri ParseErrors.0 pub(crate) defensive nicety folded into its description (not a separate perfunctory task). TASK-0080 + TASK-0081 Done stand; TASK-0199 (dep 0080/0081) is the precise scoped follow-up for the ;-only under-recovery; forward-carry to TASK-0087/0092 accurate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Algorithm parser now reports EVERY parse error in one pass (multi-error reporting) by recovering at the statement/item `;` boundary. Implemented jointly with TASK-0081 (one coherent chumsky 0.9 change).

What changed:
- error.rs: new ParseErrors(Vec<ParseError>) multi-error owner (non-empty, deterministically ordered, Deref<[ParseError]>, .first()/.errors(), per-line Display). map_all_chumsky_errors with order-preserving no-HashSet dedup. Root-caused & fixed a reproducibility violation: chumsky 0.9 Simple Display iterates an internal HashSet so the "expected one of ..." list was non-deterministic — new chumsky_message rebuilds it sorted (also fixes the sched parser, shared helper). map_first_chumsky_error retained verbatim for sched (TASK-0087, out of scope).
- parser.rs: parse_algo -> Result<AlgoAst, ParseErrors> via parse_recovery; per-item recover_with(skip_until([';'],|_|None).consume_end()).flatten(). Ok type kept = AlgoAst.
- driver: surfaces all algo parse errors, one located line each.
- algo_parser.rs: expect_err migrated mechanically (returns .first(); legacy assertions byte-identical, strength preserved); added multi-error / recovery-resumes / no-cascade / bounded-pathological tests.

Blast radius: ONLY algo_parser.rs negatives migrated; driver + ~9 non-parser test files unchanged (Ok-preservation worked).

User impact: a syntactically broken algorithm program now shows all its errors at once (each with correct line:col) instead of one-error-per-recompile.

Gate: workspace 430 passed/0 failed; e2e 30/26/0/4 required-fail 0; determinism byte-identical x2; det-negative + xbackend-negative bite; clippy --workspace --all-targets clean; just ci exit 0.

Known limitation / follow-up: ;-only sync set yields one legitimate structural follow-on (trailing UnexpectedEof / stray `}` in a broken for body) beyond the primary error — real, bounded, deterministic, not a cascade. TASK-0199 filed (keyword-anchored sync) dep 0080/0081.

Risks: none for valid input (byte-identical codegen, gated). The single chumsky-invariant expect() is decision-0003-compliant (earlier-pass-guaranteed, documented).
<!-- SECTION:FINAL_SUMMARY:END -->
