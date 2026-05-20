---
id: TASK-0199
title: >-
  Parser recovery: keyword-anchored sync set (reduce ;-only over-report) — algo
  + sched
status: To Do
assignee: []
created_date: '2026-05-19 19:44'
updated_date: '2026-05-20 05:17'
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
- [ ] #1 Recovery sync set is anchored on item-start keywords (kernel/data/const/for) in addition to ; so recovery resyncs at the next ITEM boundary, not the next inner semicolon
- [ ] #2 A malformed stmt inside a for{} body near EOF produces ONLY the primary error (no stray-} / no trailing-UnexpectedEof follow-on) — pinned by a regression test that asserts exactly 1 error for that shape; the realistic typo+valid-tail case stays exactly 1
- [ ] #3 Multi-error reporting for genuinely-independent errors is preserved (>=2 distinct errors still reported with correct per-error line:col); recovery stays bounded + deterministic (same source -> identical error set+order)
- [ ] #4 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci); zero behaviour change for valid input
- [ ] #5 Sched analog of AC#2 (TASK-0087 correction): the nested-brace worker_class/memory_region body single-error shape — currently 3 errors (primary + inner-field cascade + structural close-brace), measured deterministic, pinned by sched_parser.rs::nested_brace_body_error_surfaces_bounded_follow_ons_{worker_class,memory_region}) — must collapse to ONLY the primary error once the keyword/field-start-anchored sync set lands; those two pin tests are updated to assert exactly 1 for that shape; the flat-directive+valid-tail sched case stays exactly 1
- [ ] #6 Recovery sync set anchored on item/field-start keywords + the body-closing } (both parsers) so recovery resyncs at the next ITEM/FIELD boundary, not the next inner semicolon
- [ ] #7 A single error inside a worker_class/memory_region brace body (and the algo for{} body) collapses to the PRIMARY error only — pinned by PARAMETRIZED regression tests over brace-body field count n (n in {0,1,2,5}) asserting the count does NOT scale with n (kills the n+2 cascade); the doubly-nested accessible_by={} case also collapses to primary
- [ ] #8 Flat-directive multi-error reporting is preserved (>=2 independent errors still reported with correct per-error line:col); recovery stays bounded+deterministic; full gate green (test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci exit 0); zero behaviour change for valid input
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

UPDATE (TASK-0087 review-gate cycle, 2026-05-20, mped-architect independent probes): the description text "the doubly-nested accessible_by = { id, id }; (inside a memory_region body) yields 4" is IMPRECISE — measured this cycle, the doubly-nested case is also n+2-SHAPED, where n is the number of valid sibling fields AFTER the doubly-nested erroneous field. Probe evidence: `accessible_by = { @, host };` followed by 1 trailing field → 3 errors (NOT 4); followed by 2 trailing fields → 4 errors. The original "yields 4" was a single-n point disclosed as if it were the count — exactly the masking-defect class the cascade-class methodology targets. The doubly-nested case is structurally the SAME n+2 cascade just one level deeper (the inner `{}` set contributes one structural close-token, the outer brace body contributes another, plus n inner-field cascades).

ACTION when TASK-0199 lands: (a) the keyword/field-anchored sync set should collapse the doubly-nested case to the primary just like the singly-nested case (same mechanism); (b) the AC#7 parametric assertion can extend to a doubly-nested probe (asserting it also collapses to 1); (c) reword this task's description to remove the "yields 4" residue (replace with "yields n+2 where n counts valid sibling fields after the inner-set primary; measured n=1→3, n=2→4 — same cascade family as the singly-nested brace body"); (d) the TASK-0087 docstring at sched/parser.rs:793-795 ("independently exceeds this") is also imprecise — tighten to the now-known same-cascade-shape language at the same time.

Additionally: the TASK-0087 fixture iterates n at first-field-error-position only. mped-architect probe this cycle confirms last-field-error-position ALSO follows n+2 — both directions empirically agree. TASK-0199's AC#7 parametric assertion can iterate over both error-positions (first-field AND last-field) to widen the post-fix structural-guard coverage. Not strictly required, but if cheap, do it.
<!-- SECTION:NOTES:END -->
