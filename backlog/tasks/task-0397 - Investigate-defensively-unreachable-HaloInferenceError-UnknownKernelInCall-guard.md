---
id: TASK-0397
title: >-
  Investigate defensively-unreachable HaloInferenceError::UnknownKernelInCall
  guard
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 02:02'
updated_date: '2026-06-01 02:49'
labels:
  - hardening
  - testing
  - dead-code-audit
  - cycle-234
dependencies:
  - TASK-0396
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-234 follow-out of TASK-0396. halo_inference.rs:1211 constructs HaloInferenceError::UnknownKernelInCall when ctx.name_kernels.get(callee) misses. The code comment ITSELF says: 'The per-error fatality predicate would never reach this variant in practice (every production callsite checks name_kernels before halo_inference runs)'. So it is a DEFENSIVE guard that valid .nuc lowering cannot reach -- a contrived input-driven negative test is impossible/wrong. NEEDS a white-box reachability decision: (1) is it genuinely-defensive (then a white-box unit test calling the inner fn with a deliberately-broken name_kernels table, IF the internals are test-accessible, proves the diagnostic shape -- else mark with a documented-unreachable note + debug_assert), (2) is it DEAD (name_kernels is ALWAYS complete by construction => remove the variant), or (3) opacity-gate-rot (subsumed by an earlier name_kernels validation pass that postdates it -- memory feedback-opacity-gate-rot). Determine which and act. LOW; defensive guard, not a live correctness gap.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0396 review (cycle-234 architect P3): SCOPE EXPANSION. Besides HaloInferenceError::UnknownKernelInCall, two MORE construction SITES are defensively-unreachable from .nuc source and belong in this white-box reachability investigation:
- LowerErrorKind::ConstOverflow VIA THE checked_neg (negate) arm (algo/lower.rs:565) -- distinct from the binop arm (lower.rs:578) which TASK-0396 DID test. Reaching the negate arm needs a const evaluating to i64::MIN, but i64::MIN magnitude (9223372036854775808) exceeds the parser parse::<i64>() limit and no const can reach i64::MIN without an earlier producing-binop overflow firing first.
- LowerErrorKind::ShapeOverflow VIA THE checked_neg arm (algo/lower.rs:661) -- same, shape-dim sibling.
So at SITE granularity these two negate arms are unprovable by input (same class as UnknownKernelInCall). prove-the-check-bites at VARIANT granularity IS satisfied by TASK-0396 (the variant bites via its reachable binop arm). DECISION NEEDED here: are these negate sites genuinely-defensive (keep + documented-unreachable note, or a white-box test if internals are reachable via a crafted IR), or dead (the parser limit makes them structurally unreachable => candidate for removal/debug_assert)? Durable lesson: prove-the-check-bites should distinguish VARIANT-granularity (does the enum variant ever fire?) from SITE-granularity (does each construct-site fire?); a variant can have a tested reachable site AND an untested defensively-unreachable site.

=== Cycle-235 CORRECTION ADDENDUM (P3.3) + DELIVERY (commits eacd575, d586a62; orchestrator in-thread) ===

CORRECTION (addendum, NOT rewrite, per hygiene): this task as originally filed carried a FALSIFIED claim, forward-carried from a cycle-234 architect P3: that the ConstOverflow/ShapeOverflow checked_neg (negate) arms are defensively-unreachable because "no const can reach i64::MIN without an earlier producing-binop overflow firing first." That reasoning is WRONG and was falsified EMPIRICALLY (memory feedback-implementer-disclosure-mechanism-wrong, reviewer-subagent variant; cheap-empirical-verification beat the narrative): a COMPUTED expression `-(0 - 9223372036854775807 - 1)` reaches i64::MIN with NO intermediate overflow (0 - i64::MAX = -i64::MAX ok; -i64::MAX - 1 = i64::MIN ok), and negating it trips checked_neg. Probe confirmed: ConstOverflow{op:"negate"} and ShapeOverflow{op:"negate"} both fire from .nuc source. So the negate arms are REACHABLE, not defensively-unreachable.

DELIVERED (all 3 declared sites resolved):
1+2. The 2 negate arms (REACHABLE) -> normal input-driven negative tests in tests/algo_lower.rs: const_expr_negate_i64_min_overflows + shape_dim_negate_i64_min_overflows (assert op=="negate"). Singleton error vectors (architect-confirmed), .first() unambiguous.
3. HaloInferenceError::UnknownKernelInCall (GENUINELY unreachable from link-valid IR -- name_kernels built by build_acfg from the same kernel set; unknown calls already rejected at lowering; architect independently confirmed via lower_rvalue reject + driver linked/acfg pairing) -> WHITE-BOX test in halo_inference.rs #[cfg(test)] mod: empty name_kernels -> guard fires for "ghost" callee. Kept as a typed error (panic-not-diagnostic policy), NOT converted to unreachable!. Orchestrator AND architect mutation-proved it bites (callee->MUTANT => test fails).

GATE (qa + architect re-ran): build clean; clippy 0/0 (doc_lazy_continuation did NOT fire on the new /// white-box doc comment); test 1214/0/3 dev (+3); test-release 1213/0/3 (+3); e2e 385/328/0/57/0 unchanged. qa GO + architect GO.

REVIEW FOLD-BACK: P3.1 (my own comment-doc-lie: "apply_halo_inference builds name_kernels" -> corrected to "build_acfg builds, apply_halo_inference is given it") fixed in-thread (commit d586a62). P2.1 + P2.1b + P2.2 + P3.2 (NEWLY-FOUND checked_neg silent-siblings at build.rs:558 [const-loop-bound mis-diagnosed as NonConstLoopBound] + common.rs:355 [safe] + build.rs:550 "we panic" doc-lie + op-string pins) FILED as TASK-0398 (larger findings -> new task, not silent scope-expansion).

GATE FOOTGUN (qa, durable): never run `just test` and `just test-release` in PARALLEL against the same target dir -- cargo artifact contention causes a spurious test failure; run sequentially.

DONE: all 3 declared sites have a biting negative test; GOx2; falsified narrative corrected; silent-sibling sweep filed (TASK-0398).
<!-- SECTION:NOTES:END -->
