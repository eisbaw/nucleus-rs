---
id: TASK-0191
title: 'eqhash: factored domain-shape x access-pattern (Approach 3)'
status: To Do
assignee: []
created_date: '2026-05-19 13:58'
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
Extend equivalence-by-hashing to capture loop interchange by factoring the hash into domain shape and access pattern components.

Starting point: git tag branchpoint1 (ac12bdaa)
Working directory: equivalence-by-hashing/
Branch from: branchpoint1

## Context

The current loops.py hashes loops as a single combined value. This conflates the iteration domain structure with the body computation, making interchange detection impossible.

## Approach

Separate the hash into two components:
  H(nest) = H_domain(shape) (x) H_access(body, canonical_axes)

- H_domain: hash the shape of the iteration domain as an unordered set of extents. For a rectangular NxM domain: hash = H(N) * H(M) (commutative product, so NxM = MxN).
- H_access: hash the body with axes canonicalized by Approach 2 (intrinsic properties).

This makes interchange of rectangular domains trivially equal at the domain level, and pushes the hard problem into axis canonicalization of the body.

## Key difficulty: non-rectangular domains

Triangular loops like for i:0..N { for j:0..i { ... } } have dependent bounds. The domain shape is not a simple Cartesian product. The dependency structure between bounds reintroduces ordering sensitivity.

Options for dependent bounds:
- Hash the dependency DAG of bounds (which var depends on which)
- Canonicalize by topological sort of the dependency graph
- Restrict to rectangular domains and document the limitation

## Implementation plan
1. Implement H_domain for rectangular (independent-bound) nests
2. Implement H_access with canonical axis ordering from Approach 2
3. Combine via tensor product in the hash field
4. Test rectangular interchange
5. Investigate dependent-bound (triangular) cases
6. Compare with Approach 1 on expressiveness and complexity
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Rectangular interchange (independent bounds) hashes equal
- [ ] #2 Domain hash is commutative: NxM = MxN
- [ ] #3 Triangular/dependent bounds: either handled or honestly documented as limitation
- [ ] #4 Existing alpha-renaming and iter-space rebase tests still pass
- [ ] #5 Honest comparison with Approaches 1 and 2 documented
<!-- AC:END -->
