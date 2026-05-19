---
id: TASK-0189
title: 'eqhash: commutative accumulation over iteration domain (Approach 1)'
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
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend equivalence-by-hashing to capture loop interchange, fusion, and fission via commutative accumulation over the iteration domain.

Starting point: git tag branchpoint1 (ac12bdaa)
Working directory: equivalence-by-hashing/
Branch from: branchpoint1

## Context

The current loops.py uses depth-indexed canonical naming ($0, $1) which bakes in nesting order. This means loop interchange (swapping i,j) produces different hashes even though the computation traverses the same Cartesian product of points.

## Approach

For pure expressions, a loop nest over domain D computing body(point) is semantically a bag of values { body(p) | p in D }. Traversal order is irrelevant.

Define loop-nest hash as a symmetric function over the domain:
  H(nest) = SUM_{p in eval_points} H(body(p))
where eval_points are deterministic random-looking field elements derived from domain bounds (Schwartz-Zippel style).

Key properties:
- Interchange: same domain, same body, same sum. Free.
- Fusion: SUM H(f(p)) + SUM H(g(p)) = SUM H(f(p) + g(p)) if body-sequencing is additive. Free.
- Fission: reverse of fusion. Free.
- Trip count N is symbolic. "Multiply by trip count" means multiply by H(N) in the hash field. Distributivity still holds.
- For degree-d body polynomial over k variables, collision prob <= d/|F| per Schwartz-Zippel. With |F| = 2^521 this is negligible.

## What this does NOT capture
- Tiling/blocking (changes domain structure from 1 level to 2)
- Skewing (body transform must be recognized as inverse of domain transform)
These require geometric reasoning beyond algebraic hashing.

## Implementation plan
1. Extend loops.py AST with sequence/statement-list node
2. Define H for loop nests using commutative (additive) accumulation
3. Evaluate body at deterministic sample points derived from domain bounds
4. Add test cases: interchange, fusion, fission, negative cases
5. Document soundness argument and limitations
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Loop interchange: for i { for j { g(i,j) } } hashes equal to for j { for i { g(i,j) } }
- [ ] #2 Loop fusion: loop(N,f) ; loop(N,g) hashes equal to loop(N, f;g)
- [ ] #3 Loop fission: reverse of fusion
- [ ] #4 Existing alpha-renaming and iter-space rebase tests still pass
- [ ] #5 Negative cases: different bodies, different bounds still hash unequal
- [ ] #6 Tiling and skewing honestly documented as out-of-scope
<!-- AC:END -->
