---
id: TASK-0190
title: 'eqhash: canonical axis ordering by intrinsic properties (Approach 2)'
status: Done
assignee: []
created_date: '2026-05-19 13:58'
updated_date: '2026-05-23 21:34'
labels:
  - eqhash
  - research
  - loops
dependencies: []
references:
  - equivalence-by-hashing/loops.py
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend equivalence-by-hashing to capture loop interchange by assigning canonical variable names based on intrinsic properties rather than nesting depth.

Starting point: git tag branchpoint1 (ac12bdaa)
Working directory: equivalence-by-hashing/
Branch from: branchpoint1

## Context

The current loops.py uses depth-indexed canonical naming ($0, $1) which ties variable identity to nesting depth. Swapping loop order swaps the canonical names, breaking interchange equivalence.

## Approach

Assign canonical variable names based on intrinsic properties of the iteration variable, not binding depth:

Option A - Sort axes by domain bound hashes:
- Variable with iteration space 0..N gets canonical name derived from H(N)
- Variable with iteration space 0..M gets canonical name derived from H(M)
- When bounds differ, this gives a unique canonical ordering independent of nesting

Option B - Sort axes by role fingerprint:
- Hash the body with each variable marked as distinguished, one at a time
- Sort variables by the resulting hash values
- The variable whose removal changes the body hash most (or in sorted hash order) gets $0, etc.

## Key difficulty: tie-breaking

When two axes have identical bounds (both 0..N), Option A produces identical canonical names. Need a tie-breaker. Option B handles this but is expensive (multiple hash evaluations) and fragile (small body changes can flip canonical order, causing spurious hash differences).

## Implementation plan
1. Implement Option A with bound-based sorting
2. Add tie-breaking via body role fingerprinting (Option B)
3. Test interchange with different bounds (should work)
4. Test interchange with identical bounds (stress-tests tie-breaking)
5. Evaluate fragility: do small body edits cause spurious canonical order flips?
6. Compare collision rates and robustness vs Approach 1
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Interchange with different bounds (N vs M) hashes equal
- [ ] #2 Interchange with identical bounds (both 0..N) hashes equal via tie-breaking
- [ ] #3 Tie-breaking is stable: small body changes do not flip canonical order
- [ ] #4 Existing alpha-renaming and iter-space rebase tests still pass
- [ ] #5 Honest comparison with Approach 1 documented
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as OUT-OF-SCOPE-of-Nucleus (orchestrator-direct, cycle 77 sweep). Same as TASK-0189: this is one of 3 alternative approaches for the equivalence-by-hashing/ research subproject (Python prototype, separate from the Nucleus Rust workspace). Per memory 'project-eqhash-subproject', not Nucleus loop work. Reopen within a dedicated eqhash tracker if formal task tracking is wanted there.
<!-- SECTION:FINAL_SUMMARY:END -->
