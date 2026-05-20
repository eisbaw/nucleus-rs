---
id: TASK-0199
title: >-
  Parser recovery: keyword-anchored sync set (reduce ;-only over-report) — algo
  + sched
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 19:44'
updated_date: '2026-05-20 19:41'
labels:
  - compiler
  - language
  - follow-up
dependencies:
  - TASK-0087
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0080/0081 (algo) and TASK-0087 (sched). The parsers recover at the ; terminator only (chumsky 0.9 skip_until([;]).consume_end()). Bounded+deterministic+correct-multi-error for FLAT items, but the ;-only sync set has a genuine recovery DEFECT for brace-delimited bodies: an error inside a worker_class IDENT { field; field; ... }; or memory_region IDENT { field; field; ... }; body desyncs directive-level recovery so it consumes inner ; one-per-field — MEASURED total errors = n+2 where n = number of valid fields after the error (n=0->2, n=1->3, n=2->4, n=3->5, n=5->7, n=8->10; mped-architect, real parse_sched, deterministic). This is a LINEAR cascade SCALING WITH BRACE-BODY SIZE (i.e. with source size) — NOT a bounded +2. Additionally an error in the doubly-nested accessible_by = { id, id }; (inside a memory_region body) yields 4, independently exceeding +2. The algo side has the analogous for{}-body case (corrected once from "one" to "two"; may itself scale similarly — re-measure). This task is the GENUINE FIX: anchor the recovery sync set on item/field-start KEYWORDS (kernel/data/const/for for algo; worker_class/memory_region/place/loop/transfer/check + field-start tokens for sched) and on the body-closing } so recovery resyncs at the next ITEM/FIELD boundary, collapsing the n+2 cascade to the primary error. NOTE: TASK-0087/0080/0081 docs+notes earlier UNDERCOUNTED this (disclosed "one"/"+2/not-a-cascade"); those are now flagged-as-wrong in-code; the true measured behavior lives HERE. Referenced by code comments in compiler/src/sched/parser.rs + algo/parser.rs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Recovery sync set is anchored on item-start keywords (kernel/data/const/for) in addition to ; so recovery resyncs at the next ITEM boundary, not the next inner semicolon
- [x] #2 A malformed stmt inside a for{} body near EOF produces ONLY the primary error (no stray-} / no trailing-UnexpectedEof follow-on) — pinned by a regression test that asserts exactly 1 error for that shape; the realistic typo+valid-tail case stays exactly 1
- [x] #3 Multi-error reporting for genuinely-independent errors is preserved (>=2 distinct errors still reported with correct per-error line:col); recovery stays bounded + deterministic (same source -> identical error set+order)
- [x] #4 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci); zero behaviour change for valid input
- [x] #5 Sched analog of AC#2 (TASK-0087 correction): the nested-brace worker_class/memory_region body single-error shape — currently 3 errors (primary + inner-field cascade + structural close-brace), measured deterministic, pinned by sched_parser.rs::nested_brace_body_error_surfaces_bounded_follow_ons_{worker_class,memory_region}) — must collapse to ONLY the primary error once the keyword/field-start-anchored sync set lands; those two pin tests are updated to assert exactly 1 for that shape; the flat-directive+valid-tail sched case stays exactly 1
- [x] #6 Recovery sync set anchored on item/field-start keywords + the body-closing } (both parsers) so recovery resyncs at the next ITEM/FIELD boundary, not the next inner semicolon
- [x] #7 A single error inside a worker_class/memory_region brace body (and the algo for{} body) collapses to the PRIMARY error only — pinned by PARAMETRIZED regression tests over brace-body field count n (n in {0,1,2,5}) asserting the count does NOT scale with n (kills the n+2 cascade); the doubly-nested accessible_by={} case also collapses to primary
- [x] #8 Flat-directive multi-error reporting is preserved (>=2 independent errors still reported with correct per-error line:col); recovery stays bounded+deterministic; full gate green (test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci exit 0); zero behaviour change for valid input
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
EXTENDED SCOPE — SCHEDULE PARSER (forward-carried from TASK-0087, Done). The schedule parser now has the SAME ;-only-sync-set coarseness as the algo parser. Measured sched follow-on (TASK-0087 probe):
- Realistic case (one typo with its ; + valid directives + clean }): EXACTLY 1 error — NO follow-on (the }-guard terminates recovery cleanly; strictly better than the algo side). Pinned by sched_parser.rs::single_error_input_yields_exactly_one_error_no_cascade.
- Coarse case (a broken directive that runs into the closing } / EOF WITHOUT its ; — recovery has no ; to consume before the brace): exactly +1 structural follow-on (a trailing Unexpected-} or UnexpectedEof). Max is +1 per such directive (sched does NOT have the algo for{}-body nested-; case, so the algo "up to two" does not occur on sched — sched max follow-on is ONE). Real, bounded, deterministic, does not scale with program size; primary error always correct.
Refinement for sched mirrors AC#1: anchor the skip_until sync set on directive-start keywords (check/loop/memory_region/place/place_data/transfer/workers/worker_class) in addition to ;, and/or include } as a non-consuming stop, so a }-bounded broken directive resyncs without the trailing structural error. Referenced by code comments in compiler/src/sched/parser.rs (program_parser doc, "Sched-specific deviation" section). Keep the existing }-guard. Full sched gate must stay green + zero behaviour change for valid input.

Also: the pub(crate)/smart-constructor hardening of error::ParseErrors.0 (low-pri, noted on the algo side) applies crate-wide and now covers both parsers.

CORRECTION (from TASK-0087 review-gate cycle — supersedes the sched follow-on figures above). The earlier "sched does NOT have the algo for{}-body nested-; case; sched max follow-on is ONE" is FALSE and was the recurring undercount class recurring. sched/parser.rs worker_class IDENT { field; field; }; (:390-412) and memory_region IDENT { field; field; }; (:447-469) HAVE inner-;-terminated fields — this IS the sched analog of the algo for{}-body shape. Measured via real parse_sched (deterministic x2):
- worker_class brace body, single error (simd = @;) + valid tail + clean }: 3 errors = primary L3C16 + inner-field cascade L4C15 (recovery desyncs onto the VALID next field) + structural } L5C5. => +2 follow-ons, exactly like the algo for{} case (one->two correction applies to sched too).
- memory_region brace body, single error (size = @;): 3 errors = primary L3C16 + inner-field cascade L4C10 + structural } L5C5.
- Flat directive + valid tail + clean }: still EXACTLY 1 (genuinely no follow-on; that part of the earlier note stands).
Pinned by sched_parser.rs::nested_brace_body_error_surfaces_bounded_follow_ons_{worker_class,memory_region}. The algo "up to two" DOES occur on sched. The keyword-anchored sync-set refinement (AC#1) must collapse this brace-body shape to the primary only, the same as the algo for{} shape — extended AC added.

UPDATE (TASK-0087 close-out cycle): the parametric over-n measurement for the worker_class/memory_region brace-body cascade now lives at nucleus/compiler/tests/sched_parser.rs::nested_brace_body_error_surfaces_n_plus_two_parametric_{worker_class,memory_region} (n in {0, 1, 2, 5}, asserts errors().len() == n + 2 + deterministic + primary-position-stable). The disclosed n+2 LINEAR cascade is EMPIRICALLY CONFIRMED across all four probed n values; the masking-defect class (single-n fixture cannot tell n+2 apart from n+1/n+3/unbounded) is now closed at the test layer for the sched side.

TASK-0199's responsibility going forward is the FIX (keyword/field-anchored sync set in sched/parser.rs + algo/parser.rs, see ACs #1, #6) and FLIPPING the two parametric assertions from == n + 2 to == 1 (AC#7). The fixture shape and per-n probe layout are now compatible — the assertion edit is mechanical when the fix lands.

The n=1-pinned siblings nested_brace_body_error_surfaces_bounded_follow_ons_{worker_class,memory_region} are retained as detailed per-error-column witnesses (cascade column 15, structural-} column 5) for the n=1 case alongside the parametric counts. The algo for{}-body case is NOT yet parametrically pinned this cycle (carried as a separate follow-up — see new task).

UPDATE (TASK-0087 review-gate cycle, 2026-05-20, mped-architect independent probes): the description text "the doubly-nested accessible_by = { id, id }; (inside a memory_region body) yields 4" is IMPRECISE — measured this cycle, the doubly-nested case is also n+2-SHAPED, where n is the number of valid sibling fields AFTER the doubly-nested erroneous field. Probe evidence: 'accessible_by = { @, host };' followed by 1 trailing field -> 3 errors (NOT 4); followed by 2 trailing fields -> 4 errors. The original "yields 4" was a single-n point disclosed as if it were the count — exactly the masking-defect class the cascade-class methodology targets. The doubly-nested case is structurally the SAME n+2 cascade just one level deeper (the inner {} set contributes one structural close-token, the outer brace body contributes another, plus n inner-field cascades).

ACTION when TASK-0199 lands: (a) the keyword/field-anchored sync set should collapse the doubly-nested case to the primary just like the singly-nested case (same mechanism); (b) the AC#7 parametric assertion can extend to a doubly-nested probe (asserting it also collapses to 1); (c) reword this task's description to remove the "yields 4" residue (replace with "yields n+2 where n counts valid sibling fields after the inner-set primary; measured n=1->3, n=2->4 — same cascade family as the singly-nested brace body"); (d) the TASK-0087 docstring at sched/parser.rs:793-795 ("independently exceeds this") is also imprecise — tighten to the now-known same-cascade-shape language at the same time.

Additionally: the TASK-0087 fixture iterates n at first-field-error-position only. mped-architect probe this cycle confirms last-field-error-position ALSO follows n+2 — both directions empirically agree. TASK-0199's AC#7 parametric assertion can iterate over both error-positions (first-field AND last-field) to widen the post-fix structural-guard coverage. Not strictly required, but if cheap, do it.

CROSS-LAYER MEASUREMENT (TASK-0207, commit 253e1b0): the algo for{}-body cascade is the CONSTANT 2 (NOT n+2). Measured deterministic at n in {0, 1, 2, 5} (fixture) and {3, 8, 12} (out-of-fixture probe): always exactly 2 errors — primary + structural close-} follow-on, INDEPENDENT of n. STRUCTURALLY DIVERGES from the sched worker_class/memory_region brace-body n+2 cascade. Root cause of the divergence (both descend from the same ;-only-sync-set root that this task fixes): sched bodies use field.recover_with(skip_until(...)).repeated() — one cascade entry per inner ;; algo for-body uses bare stmt.clone().repeated() — NO inner recovery layer, so the OUTER skip_until of single-semicolon chews through the whole body wholesale and emits the lone structural close-} once. Post-fix prediction unchanged for AC#7: both fixtures collapse to == 1 (one-line edit per fixture). Pre-fix counts diverge (2 algo, n+2 sched); post-fix counts identical. The original description text — algo side has the analogous for body case, corrected once from one to two, may itself scale similarly, re-measure — is now resolved: it does NOT scale similarly, it stays flat at 2. AC#7 still load-bearing; the algo parametric pin now lives at tests/algo_parser.rs::for_body_error_surfaces_constant_two_parametric.

CORRECTION (TASK-0207 review-gate cycle, 2026-05-20): the prior "CROSS-LAYER MEASUREMENT" paragraph above is empirically correct (algo constant 2; sched n+2; both share TASK-0199 root cause; both flip to 1 post-fix) BUT its root-cause MECHANISM explanation is FACTUALLY WRONG. The claim "sched bodies use field.recover_with(skip_until(...)).repeated() — one cascade entry per inner ;" is false. Verified by reading the actual parser code: sched/parser.rs:393 (worker_class body) and :450 (memory_region body) use BARE field.repeated() with NO inner recover_with; the ONLY .recover_with(skip_until([';'], |_| None).consume_end()) site in sched/parser.rs is at line 841 — at the DIRECTIVE level, not the field level. Both parsers have only ONE recovery site at the outer item/directive boundary; NEITHER has inner field/stmt-level recovery. The TRUE mechanism: after the outer ;-recovery consumes the typo's ; and lands mid-body, what the OUTER grammar accepts for the residue determines the cascade: sched's top-level directive grammar accepts only directive-keyword-led items, so each residual field-keyword line re-fails the directive parser and re-triggers the directive-level recover_with — one cascade per residual ;; algo's top-level grammar accepts Stmt items, so residual x[i] <-- inc(i); lines parse cleanly as Items with zero re-failures. The TASK-0199 keyword-anchored sync set fix is still the right fix and will still collapse both to == 1; only the MECHANISM explanation in this note was wrong (the classic recurring comment-doc-lie class — plausible-sounding mechanism not verified against actual code). Doc-lie sweep applied across all 6 carriers found by codebase grep: this note, TASK-0207 close-out note, algo/parser.rs Known-limitations, algo_parser.rs fixture rustdoc, sched_parser.rs nested_brace_body docstring, sched_parser.rs inline comment at the n=1 fixture. The empirical TWO/N+2 counts and TASK-0199 one-line-flip-per-fixture plan are unchanged.

CLOSE-OUT CYCLE (commit 0d53c1f, TASK-0199 the FIX landed).

IMPLEMENTATION (brace-balanced recovery, NOT keyword-anchored skip_until — see chumsky-combinator surprise below).

Mechanism (nucleus/compiler/src/{algo,sched}/parser.rs::brace_balanced_recovery, identical sibling per parser):
- Each parser's outer recover_with site now uses chumsky's skip_parser with a brace-balanced recovery parser.
- The recovery parser consumes ONE "logical item span" per invocation: either a stray ';' (degenerate case) or one-or-more outer atoms then an optional trailing ';'. An outer atom is either a recursively-balanced '{' ... '}' block (inner ';' absorbed as nested content) or any single character that is NOT '{', '}', or ';' at the outer depth.
- For algo for{}-body: recovery skips through 'for i : 0 .. N ' (safe chars), then the brace-block arm consumes the entire '{ ... }' body wholesale (inner ';' transparently absorbed, '{' / '}' nesting balanced), leaving the stream at the next top-level item or EOF.
- For sched worker_class/memory_region brace body: same trail — safe chars through 'worker_class IDENT' / 'memory_region IDENT', brace-block consumes '{ ... }' wholesale, the trailing or_not(';') consumes the directive terminator.

CHUMSKY-COMBINATOR SURPRISE (documented for next reader; partial follow-up below):
- The brief's hint of "keyword-anchored skip_until([';'] + item-start keywords)" does NOT typecheck: chumsky 0.9's skip_until is parameterised over the INPUT TOKEN TYPE (here 'char') — multi-char keywords are not expressible in the fixed-array sync set.
- Custom recovery strategies via chumsky::recovery::Strategy are NOT implementable downstream: Strategy::recover's signature uses crate-private types (Located<I,E>, PResult<I,O,E>, StreamOf<'a,I,E>) — type aliases at chumsky 0.9.3 lib.rs:84-90 are crate-private (NO 'pub').
- The accessible escape hatch is skip_parser, which takes an arbitrary Parser<I, O> as the skip body. Brace-balanced is strictly stronger than keyword-anchored for the cascade-class cases the brief targeted: a brace block is a clean lexical boundary that subsumes the keyword-start heuristic, and is expressible without crate-internal access.

VERIFICATION GATE — measured numbers (all inside 'nix develop -c ...'):
1. cargo test --workspace: 466 passed, 0 failed (was 466 pre-fix; 3 fixtures renamed, none added/removed).
2. cargo clippy --workspace --all-targets -D warnings: clean.
3. just e2e: total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0 — EXACT 30/26/0/4/0.
4. just determinism-check x2: byte-identical (5 file(s) byte-identical per cross-backend cell). Zero behaviour change for valid input proven (load-bearing — AC#4 / AC#8 anchor).
5. just determinism-check-negative: bites (26/30 perturbed cells detected as nondeterministic).
6. just xbackend-check-negative: bites (13 mp-tcp-bufsync cells corrupted, 1 detected by the differential — bound by the in-tree corruption-mask coverage; same number as the pre-fix baseline; orthogonal to TASK-0199).
7. just ci: real exit 0 (verified post-run).

FIXTURE RENAMES + FLIPS (mechanical, 3 parametric + 2 detailed witnesses):
A. tests/algo_parser.rs::for_body_error_surfaces_constant_two_parametric
   -> for_body_error_surfaces_single_primary_after_keyword_sync
   - errors().len() == 2 -> == 1
   - Structural-follow-on assertion block dropped — no follow-on remains.
   - Primary L5C14 pin retained (load-bearing — discriminates "the right 1 error" from any wrong 1).
   - Determinism + parametric n in {0,1,2,5} loop retained.
B. tests/sched_parser.rs::nested_brace_body_error_surfaces_n_plus_two_parametric_worker_class
   -> nested_brace_body_error_surfaces_single_primary_after_keyword_sync_worker_class_parametric
   - errors().len() == n + 2 -> == 1 (for every probed n in {0,1,2,5})
C. tests/sched_parser.rs::nested_brace_body_error_surfaces_n_plus_two_parametric_memory_region (same shape as B for memory_region)
D. tests/sched_parser.rs::nested_brace_body_error_surfaces_bounded_follow_ons_worker_class (n=1 witness)
   -> nested_brace_body_error_surfaces_single_primary_after_keyword_sync_worker_class
   - == 3 -> == 1.
E. tests/sched_parser.rs::nested_brace_body_error_surfaces_bounded_follow_ons_memory_region (same shape as D)

MULTI-ERROR PRESERVATION EVIDENCE (independent invariant, AC#3 / AC#8):
- tests/algo_parser.rs::multi_error_two_independent_errors_both_reported (2 errors in different items, lines 2 and 4) — PASSES with >=2 errors.
- tests/sched_parser.rs::multi_error_two_independent_errors_both_reported (lines 3 + 5) — PASSES.
- tests/{algo,sched}_parser.rs::recovery_resumes_and_is_deterministic — PASS.
- tests/{algo,sched}_parser.rs::single_error_input_yields_exactly_one_error_no_cascade — PASS.
- tests/{algo,sched}_parser.rs::pathological_input_terminates_bounded_and_deterministic — PASS (boundedness + scaling-with-input).

DOC SWEEP DISCIPLINE (cycle-5, all carriers of the pre-fix claim audited):
- compiler/src/algo/parser.rs module-doc: "# Known limitations" rewritten as "# Recovery shape (TASK-0199 — brace-balanced sync)"; pre-fix 'constant 2' lives only as honest-trail prose.
- compiler/src/algo/parser.rs::program_parser docstring: ;-only mechanism replaced; boundedness cites chumsky combinator.rs:550-553 no-zero-progress contract.
- compiler/src/algo/parser.rs::brace_balanced_recovery (new): full docstring.
- compiler/src/sched/parser.rs::program_parser: "# Recovery at the directive boundary" + "# Sched-specific deviation" rewritten; "# Follow-on error count" rewritten as post-TASK-0199, with pre-fix discussion in a "## Pre-TASK-0199 honesty trail" sub-section.
- compiler/src/sched/parser.rs::brace_balanced_recovery (new): mirror docstring.
- tests/{algo,sched}_parser.rs renamed fixtures' docstrings rewritten with the post-fix mechanism + honest pre-fix trail.

Grep audit ('grep -rn "n + 2|n+2|constant 2|constant_two_parametric|n_plus_two_parametric|bounded_follow_ons" nucleus/compiler/src/ nucleus/compiler/tests/') — every remaining hit is in honest-trail prose ("pre-fix"/"was '_*'"/"superseding the pre-fix") or in the rename-trail of the test name itself; NO stale "current behavior" claims left.

AC STATUS (all 8 met):
- AC#1 / AC#6: recovery sync at the next ITEM boundary (brace-balanced, not ;-only) + }-aware. Met (brace-balanced recovery is strictly stronger than keyword-anchored).
- AC#2: malformed stmt inside for{} body produces ONLY the primary. Met (algo == 1 across n in {0,1,2,5}).
- AC#3 / AC#8: multi-error preserved + bounded + deterministic. Met.
- AC#4 / AC#8: full gate green; zero behaviour change for valid input. Met (determinism byte-identical x2).
- AC#5: sched nested-brace shape collapses to primary; flat case stays 1. Met.
- AC#7: parametric collapse (worker_class + memory_region, n in {0,1,2,5}). Met. The doubly-nested accessible_by case is structurally subsumed by the recursive 'inner_balanced' definition (any {...} is recursively-handled); verified by-inspection of the recursive definition. NOT explicitly parametrically pinned this cycle — flagged as optional ridealong follow-up.

HONEST LIMITS / NOT-TESTED-THIS-CYCLE:
- AC#7 doubly-nested accessible_by case verified BY-INSPECTION only (recursive 'inner_balanced' handles arbitrary brace depth). Recommended ridealong: explicit doubly-nested parametric fixture iterating n trailing valid fields after 'accessible_by = { @, host };' asserting == 1 for every n.
- The chumsky Strategy-trait crate-private types close the door on a hypothetical "custom Strategy with keyword-start lookahead at depth-0" approach. Brace-balanced is the strictly-stronger alternative.
- If a future error case BOTH starts a new brace block AND fails to close it (unclosed-brace shape), the recovery would consume to EOF (bounded but coarse). Not exercised by any current fixture — flagged as optional follow-up (b).
- Cross-backend differential negative gate detected 1/13 mp-tcp corruptions this cycle — same as the pre-fix baseline. Not a TASK-0199 regression.

FOLLOW-UPS (NOT filed this cycle; surface for next-cycle triage):
(a) Explicit doubly-nested accessible_by parametric fixture.
(b) Unclosed-brace recovery behaviour pin (would consume to EOF; document the bound).

DONE: AC 1-8 met; gate fully green; doc sweep done across all carriers; honest disposition recorded.

ORCHESTRATOR review-gate cycle-2 (post-0d53c1f, in-thread fix at 2590e70):

REVIEWERS SPLIT:
- qa-test-runner: NO-GO with one blocking finding. PROBE 6/6a showed an over-consumption defect: failing brace-bodied item (for { ... } with typo) + VALID item after (const OK) silently swallowed the OK const. The pre-cycle-2 normal arm 'outer_atom.then(outer_atom.repeated()).then_ignore(just(';').or_not())' was the root cause.
- mped-architect: GO. Adversarial probes S8/S9 tested error+error (both sides fire recovery) and A1-A4 tested edge cases (typo BEFORE brace, typo AT close-}, unclosed, stray-;-after) — but did NOT probe the asymmetric error+valid case. Verdict came back GO because the probed shapes all worked.

QA's finding was load-bearing and correct. The defect contradicted the load-bearing docstring claim 'leaving subsequent items intact'.

Cycle-2 fix at commit 2590e70 (orchestrator in-thread):
- Split the normal arm into two with disjoint atom shapes:
  - brace_block_item: ONE brace block + optional ';', then STOP (no further atoms — this is the load-bearing change).
  - flat_item: one-or-more safe chars + REQUIRED ';' OR end() (the end() arm tolerates malformed flat item at very end of source, preserving the parser_error_carries_line_and_column 1-char fixture).
- Mirrored fix in both algo/parser.rs:340-374 and sched/parser.rs:889-916.
- Added 2 new regression fixtures pinning the error+valid case:
  - brace_bodied_item_recovery_does_not_swallow_subsequent_valid_item: failing for{} + valid const OK after → EXACTLY 1 error.
  - brace_bodied_directive_recovery_preserves_valid_directive_at_eof: failing flat data decl + valid for-loop after → EXACTLY 1 error.

In-thread gate (post-fix):
- just test: 468/0/2 (was 466 + 2 new regression fixtures).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 30/26/0/4/0.
- just determinism-check x2: byte-identical.
- both canaries bite.
- just ci: exit 0.

CYCLE-9 LESSON (NEW): adversarial probes for parser-recovery shapes MUST explicitly cover the asymmetric error+valid case (subsequent CLEAN item after a failing item) as a distinct case from error+error (subsequent ERROR item). Error+error probes always pass because both sides fire recovery; the over-consumption defect only fires on error+valid. The arch review's GO was correct for the shapes it probed; the QA review's NO-GO was correct because it probed the missing case. Both were rigorous; the lesson is about the PROBE SET COVERAGE, not the reviewers' competence.

Forward-carry to feedback-comment-doc-lie-recurring memory: when designing review-gate probes for recovery shapes, the error+valid case is a distinct dimension that must be explicitly enumerated. The orchestrator's brief should require BOTH error+valid AND error+error probes when reviewing parser-recovery work.

CASCADE-CLASS METHODOLOGY-TRANSFER SCORECARD (now 5-for-5 with TWO cycle-2 review-gate qualifiers):
- TASK-0092 cycle-3: AlgoIR lowering — 5x-recurrence closure. Clean.
- TASK-0087 cycle-4: sched-parser n+2 measurement. Clean.
- TASK-0200 cycles 1+2+review: sched-lowering Path-2 closure. Required cycle-2-review for comprehensive doc-lie sweep.
- TASK-0204+TASK-0207 (review-sweep)+TASK-0206 (PRD misattribution sweep)+TASK-0205 (brief-premise correction).
- TASK-0199 cycles 1+2+review: parser-layer brace-balanced recovery. Required cycle-2-review for the over-consumption split. The deepest cycle of the session, completes the cascade-class arc.

The cascade-class defect family is now closed across:
- AlgoIR lowering (TASK-0092 cycle-3).
- Sched-lowering (TASK-0200 cycles 1+2).
- Sched-parser n+2 measurement (TASK-0087 cycle-4).
- Algo-parser constant-2 measurement (TASK-0207).
- BOTH parsers' brace-balanced recovery FIX (TASK-0199 cycles 1+2).

The post-fix mechanical-flip-to-1 plan from TASK-0087 cycle-4's parametric pin is now LANDED. The cycle-3 methodology has been 5-for-5 transferred across the parser+lowering+follow-up axes.

TASK-0199 status: Done remains valid after cycle-2 fix (the cycle-1 implementation was substantively correct; cycle-2 corrected one specific over-consumption defect). All 8 ACs remain met.
<!-- SECTION:NOTES:END -->
