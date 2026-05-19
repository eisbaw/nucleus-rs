---
id: TASK-0081
title: 'Algorithm parser: error recovery (skip-to-next-stmt)'
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
Add recovery so the parser can resume at the next plausible statement / item boundary after a syntactic failure, instead of bailing on the first error. Pairs with TASK-0080 (multi-error reporting). Use chumsky's recovery combinators (nested_delimiters / skip_then_retry_until).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The algorithm parser recovers at the next plausible statement/item boundary after a syntactic failure (chumsky recover_with: skip_then_retry_until / nested_delimiters) instead of bailing on the first error — implemented jointly with TASK-0080 (in chumsky they are one coherent change)
- [x] #2 Recovery is bounded and deterministic (no infinite-retry; same source -> same error set+order; reproducibility gate)
- [x] #3 Done jointly with TASK-0080; full gate green (see TASK-0080 ACs)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Joint with TASK-0080 (one coherent chumsky change). Recovery sync point = statement/item boundary. chumsky 0.9: parser.parse_recovery(src) -> (Option<Vec<SpItem>>, Vec<Simple<char>>). Add recover_with(skip_then_retry_until([...])) on the item-level repeated() so a failed item skips to the next real sync token (`;` then layout, or an item-start keyword kernel/data/const, or a stmt start) instead of aborting. Boundedness: skip_then_retry_until is anchored ONLY on concrete sync chars that genuinely advance input; chumsky stops at end(); no unbounded retry. Determinism: chumsky Simple ordering is positional; we never use HashMap/HashSet in the error path; dedup is order-preserving Vec dedup. Tests: recovery (error mid-program, later valid items still parsed+reported), no-spurious-cascade (single error => exactly 1), pathological malformed input terminates with bounded error set. Gate shared with TASK-0080.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED jointly with TASK-0080 (commits be43c33 feature, 12af9b9 tests). See TASK-0080 notes for full detail.

Recovery sync point = statement/item `;` terminator. chumsky 0.9: per-item parser lifted to Option<SpItem>, recover_with(skip_until([';'], |_| None).consume_end()), .repeated().flatten(). Used parse_recovery (not parse) -> (Option<AST>, Vec<Simple>).

KEY chumsky-0.9 subtlety (gotcha for TASK-0087/0092): skip_then_retry_until surfaces only the FIRST error per recovery site and stops the repetition once no further item parses (verified empirically — gave 1 error even for 2 broken items). The fix that makes multi-error ACTUALLY work is skip_until with a recovery VALUE (None here): because the failed element yields a value, .repeated() continues to the next item and each broken item contributes its own error. None placeholders flattened away; partial AST discarded on failure so AST shape / TASK-0082 spans untouched.

Boundedness argument: each skip_until step consumes >=1 char or reaches end-of-input where chumsky's SkipUntil::recover terminates (Err arms); .repeated() makes finite progress every iteration. Pathological 8x wall-of-garbage input terminates with 113 errors (finite, <= O(n), deterministic across runs AND across a different repeat count). Determinism: single fixed sync token + positional chumsky order + SORTED expected-set message (chumsky's default Display HashSet-order was the real non-determinism, root-caused & fixed in error::chumsky_message).

GATE: see TASK-0080 (430/0 tests; e2e 30/26/0/4/0; determinism byte-identical x2; negatives bite; clippy clean; ci exit 0).

Forward-carry filed: TASK-0199 (keyword-anchored sync set to drop the ;-only structural follow-on) dep 0080/0081.

ORCHESTRATOR review-gate close (phase3-ralph, BOTH reviewers GO — combined cycle, both Done): qa-test-runner (re-verified): NO hash-order in any parser-error path (chumsky_message sorts+dedups a Vec; sched shares the same fixed helper), determinism byte-identical x2 + the determinism tests genuinely assert repeat-identical error output; ONLY algo_parser.rs changed among test files (Ok type stayed AlgoAst — blast radius confined, proven structurally); workspace 430/0; negative migrations strength-preserved (expect_err +.first().clone(), zero call-site assertion changes, no masked weakening); multi-error/recovery/no-cascade/bounded-pathological tests genuine; e2e EXACTLY 30/26/0/4/0; both negatives bite; clippy --all-targets clean; ci exit 0; no parse-path panic; no stale "bails on first error" doc residue. mped-architect (re-derived from code incl. chumsky 0.9 library source): error-type design sound + Ok-preserving; the chumsky-HashSet reproducibility ROOT-FIX is real/complete/correctly-placed at the single shared map_one_chumsky_error->chumsky_message helper and GENUINELY fixes the sched parser latent same-bug for free (verified by call graph, zero sched edits — correct root placement); recovery PROVABLY bounded (verified against chumsky recovery.rs: strict >=1-token advance, unconditional break on sync/EOF, no quadratic/hang) + deterministic; the not-blanket-len()==1 decision is correctly reasoned (a sometimes-false assert is wrong not stronger; no-cascade pinned by a dedicated test); decision-0003 clean (the 2 expects are guarded library-contract invariants). One genuine root-fix found DURING verification (chumsky Simple Display HashSet non-determinism) — honest find, airtight fix, zero-behaviour-change for valid input. Non-blocking findings APPLIED IN-THREAD by orchestrator (phase3-ralph step 4): (a) the imprecise error.rs "None sorts last" comment rewritten accurately (commit after 12af9b9; comment-only, gate re-verified determinism byte-identical + clippy --all-targets); (b) the qa-found disclosure UNDERCOUNT (real max is TWO bounded structural follow-ons — stray } AND/OR trailing UnexpectedEof, simultaneously possible in a for{} body near EOF — not "ONE"; bounded, deterministic, does-not-scale, primary always correct, NOT a cascade) corrected in the authoritative forward-looking source TASK-0199 description; (c) TASK-0199 had zero ACs (backlog-discipline gap) — 4 testable ACs added + the low-pri ParseErrors.0 pub(crate) defensive nicety folded into its description (not a separate perfunctory task). TASK-0080 + TASK-0081 Done stand; TASK-0199 (dep 0080/0081) is the precise scoped follow-up for the ;-only under-recovery; forward-carry to TASK-0087/0092 accurate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Algorithm parser error recovery (skip-to-next-stmt), implemented jointly with TASK-0080 — in chumsky 0.9 recovery and multi-error reporting are inseparable.

Recovery: per-item parser lifted to Option<SpItem> with recover_with(skip_until([';'],|_|None).consume_end()), then .repeated().flatten(). On a syntactic failure inside an item, chumsky skips to & consumes the next `;` and resumes item parsing; later valid items still parse, later errors still report.

Bounded: each recovery step consumes >=1 char or hits EOF where chumsky terminates; .repeated() makes finite progress per iteration. Verified: pathological 8x garbage input terminates with a finite (<=O(n)), deterministic error set; determinism gate byte-identical x2.

Deterministic: single fixed sync token, positional chumsky order, sorted expected-set message (the chumsky-Display HashSet order was the real non-determinism — root-caused & fixed in error::chumsky_message).

Crucial chumsky-0.9 lesson (forward-carried): skip_then_retry_until surfaces only ONE error per site and halts repetition; skip_until WITH a recovery value is what makes true multi-error work — the failed element must yield a value so .repeated() continues.

No AST shape change (None placeholders flattened, partial AST discarded on failure; TASK-0082 span substrate untouched). Scope = algorithm parser only.

Gate (shared with TASK-0080): workspace 430/0; e2e 30/26/0/4/0; determinism x2 byte-identical; negatives bite; clippy clean; ci exit 0.

Follow-up TASK-0199 (keyword-anchored sync to drop the ;-only structural follow-on) filed, dep 0080/0081.
<!-- SECTION:FINAL_SUMMARY:END -->
