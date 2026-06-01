---
id: TASK-0397
title: >-
  Investigate defensively-unreachable HaloInferenceError::UnknownKernelInCall
  guard
status: To Do
assignee: []
created_date: '2026-06-01 02:02'
updated_date: '2026-06-01 02:18'
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
<!-- SECTION:NOTES:END -->
